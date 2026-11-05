//! OS-protected storage for the Remember Me session token.
//!
//! Windows uses DPAPI in current-user scope. Other platforms fail closed
//! rather than falling back to plaintext or home-grown encryption.

#[cfg(windows)]
mod platform {
    use std::path::PathBuf;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData,
        CryptUnprotectData,
    };

    const MAX_CIPHERTEXT_BYTES: u64 = 16 * 1024;

    fn token_path() -> PathBuf {
        crate::profile::config_dir().join("remembered-session.dpapi")
    }

    fn protect(plaintext: &[u8]) -> Result<Vec<u8>, String> {
        let cb_data = u32::try_from(plaintext.len())
            .map_err(|_| "токен Remember Me слишком большой".to_string())?;
        let input = CRYPT_INTEGER_BLOB {
            cbData: cb_data,
            pbData: plaintext.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB::default();

        unsafe {
            CryptProtectData(
                &input,
                PCWSTR::null(),
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
        .map_err(|error| format!("DPAPI не смог защитить токен: {error}"))?;

        copy_and_free(output)
    }

    fn unprotect(ciphertext: &[u8]) -> Result<Vec<u8>, String> {
        let cb_data = u32::try_from(ciphertext.len())
            .map_err(|_| "файл Remember Me слишком большой".to_string())?;
        let input = CRYPT_INTEGER_BLOB {
            cbData: cb_data,
            pbData: ciphertext.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB::default();

        unsafe {
            CryptUnprotectData(
                &input,
                None,
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
        .map_err(|error| format!("Windows не смог расшифровать токен Remember Me: {error}"))?;

        copy_and_free(output)
    }

    fn copy_and_free(blob: CRYPT_INTEGER_BLOB) -> Result<Vec<u8>, String> {
        if blob.pbData.is_null() {
            return Err("Windows вернул пустой токен Remember Me".into());
        }
        if blob.cbData == 0 {
            unsafe {
                let _ = LocalFree(Some(HLOCAL(blob.pbData.cast())));
            }
            return Err("Windows вернул пустой токен Remember Me".into());
        }
        let bytes = unsafe {
            std::slice::from_raw_parts(blob.pbData, blob.cbData as usize).to_vec()
        };
        unsafe {
            let _ = LocalFree(Some(HLOCAL(blob.pbData.cast())));
        }
        Ok(bytes)
    }

    pub fn supported() -> bool {
        true
    }

    pub fn store_token(token: &str) -> Result<(), String> {
        let encrypted = protect(token.as_bytes())?;
        let path = token_path();
        let parent = path
            .parent()
            .ok_or_else(|| "не удалось определить каталог Remember Me".to_string())?;
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        std::fs::write(path, encrypted)
            .map_err(|error| format!("не удалось сохранить токен Remember Me: {error}"))
    }

    pub fn load_token() -> Result<Option<String>, String> {
        let path = token_path();
        let ciphertext = match std::fs::read(&path) {
            Ok(ciphertext) => ciphertext,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("не удалось прочитать токен Remember Me: {error}")),
        };
        if ciphertext.is_empty() || ciphertext.len() as u64 > MAX_CIPHERTEXT_BYTES {
            return Err("файл Remember Me повреждён или имеет недопустимый размер".into());
        }
        let plaintext = unprotect(&ciphertext)?;
        let token = String::from_utf8(plaintext)
            .map_err(|_| "токен Remember Me повреждён".to_string())?;
        if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("токен Remember Me имеет неверный формат".into());
        }
        Ok(Some(token))
    }

    pub fn clear_token() -> Result<(), String> {
        match std::fs::remove_file(token_path()) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("не удалось удалить токен Remember Me: {error}")),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn dpapi_round_trip_is_bound_to_current_windows_user() {
            let token = "ab".repeat(32);
            let ciphertext = protect(token.as_bytes()).unwrap();
            assert_ne!(ciphertext, token.as_bytes());
            assert_eq!(unprotect(&ciphertext).unwrap(), token.as_bytes());
        }
    }
}

#[cfg(not(windows))]
mod platform {
    pub fn supported() -> bool {
        false
    }

    pub fn store_token(_token: &str) -> Result<(), String> {
        Err("Remember Me пока поддерживается только в Windows Credential Protection (DPAPI)".into())
    }

    pub fn load_token() -> Result<Option<String>, String> {
        Err("защищённое хранилище Remember Me не поддерживается на этой платформе".into())
    }

    pub fn clear_token() -> Result<(), String> {
        Ok(())
    }
}

pub fn supported() -> bool {
    platform::supported()
}

pub fn store_token(token: &str) -> Result<(), String> {
    platform::store_token(token)
}

pub fn load_token() -> Result<Option<String>, String> {
    platform::load_token()
}

pub fn clear_token() -> Result<(), String> {
    platform::clear_token()
}
