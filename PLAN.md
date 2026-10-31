# ArkHa Cross-Platform Plan: Windows + Linux

## Objective
Port ArkHa from Windows-native to support both Windows and Linux (Debian-based, X11), with future iOS support planned. Current focus: Windows (native, no WSL2/Docker) and Linux (X11).

## Architecture
Common interface + platform-specific implementations via `#[cfg(target_os = "...")]`:

```
src-tauri/src/
  voice/
    mod.rs              # common commands (unchanged)
    record.rs           # interface: start/stop/resample
    record_windows.rs   # WASAPI (existing)
    record_linux.rs     # ALSA/PipeWire via cpal (new)
    hotkey.rs           # interface: enable/disable/hotkey
    hotkey_windows.rs   # WH_KEYBOARD_LL (existing)
    hotkey_linux.rs     # X11 XGrabKey on Ctrl+Alt+M (new)
    vad.rs              # Silero ONNX (unchanged, cross-platform)
    stt.rs, tts.rs      # unchanged (cross-platform models)
  permissions.rs        # interface: is_denied_app
  permissions_windows.rs # arkha.exe / Win32 (existing)
  permissions_linux.rs   # /proc/*/cmdline (new)
```

## Dependencies

### Common (all platforms)
- `rubato = "5.0.1"` — sinc resampling (already added)
- `rayon = "1.12.0"` — parallel RMS (already added)
- `tracing = "0.1.44"`, `tracing-subscriber = { version = "0.3.23", features = ["env-filter"] }` — logging (already added)
- `candle-onnx = "0.11"`, `prost = "0.14"`, `zip = "8"`, `tauri = { version = "2", features = [] }`

### Windows only
```toml
[target.'cfg(target_os = "windows")'.dependencies]
windows = { version = "0.62", features = [...] }
```

### Linux only
```toml
[target.'cfg(target_os = "linux")'.dependencies]
cpal = "0.15"
libc = "0.14"
```

## Implementation Phases

| Phase | Task | Platforms |
|-------|------|-----------|
| 1 | Audit & move platform deps: `windows` under `cfg(windows)`, `cpal`+`libc` under `cfg(linux)` | Windows + Linux |
| 2 | `record_linux.rs`: record via cpal (ALSA/PipeWire), mono 16kHz, device enumeration | Linux |
| 3 | `hotkey_linux.rs`: X11 `XGrabKey` on Ctrl+Alt+M, push-to-talk, auto-repeat filter | Linux (X11) |
| 4 | `permissions_linux.rs`: `is_denied_app` via `/proc/*/cmdline` | Linux |
| 5 | CI matrix `windows-latest` + `ubuntu-latest`, replace `fetch-llama.ps1` | Windows + Linux |
| 6 | Tests: `cargo test --lib` on both OS, `docs/19` adapted for Linux | Windows + Linux |
| 7 | Installers: NSIS/MSI (Windows), `.deb`/`.AppImage`/tarball (Linux) | Windows + Linux |

## Risks & Mitigations

| Risk | Severity | Mitigation |
|------|----------|-----------|
| Wayland no global hotkeys without compositor | High | Start with X11 only; Wayland deferred |
| Default audio device virtualized | Medium | Enumerate active capture devices |
| `fetch-llama.ps1` broken on Linux CI | Medium | Cross-platform script or separate steps |
| Cyrillic Rust identifiers | Low | UTF-8, compiler behaves consistently |

## Status
Plan documented. Implementation pending (agent transition from Plan mode to execution mode).

## Next Steps (if proceeding)
1. Execute Phase 1: move dependencies, verify `cargo check`
2. Execute Phase 2: implement `record_linux.rs`
3. Execute Phase 3: implement `hotkey_linux.rs`
4. Set up CI matrix
5. Run tests on both platforms