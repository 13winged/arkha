# Скачать `protoc` для сборки `candle-onnx`.
#
# Зачем он нужен. `candle-onnx` генерирует разбор ONNX на этапе
# сборки и требует компилятор протокольных буферов. В системе его
# обычно нет, и ставить глобально не хочется: сборка должна
# зависеть от проекта, а не от того, что настроено на машине.
#
# Куда кладёт: src-tauri/tools/protoc. Путь оттуда прописан в
# src-tauri/.cargo/config.toml, поэтому после запуска этого скрипта
# `cargo build` и `cargo test` работают сами.
#
# Лицензия: BSD-3-Clause. docs/09-лицензии.md, раздел «VAD».

$ошибка = 1
$хост = "https://api.github.com/repos/protocolbuffers/protobuf/releases/latest"
$куда = Join-Path $PSScriptRoot "..\src-tauri\tools\protoc"

if (Test-Path (Join-Path $куда "bin\protoc.exe")) {
    Write-Host "protoc уже на месте: $куда"
    exit 0
}

try {
    $ошибка = 0
    $релиз = Invoke-RestMethod $хост -Headers @{ "User-Agent" = "arkha" } -TimeoutSec 30
    $версия = $релиз.tag_name
    $нужен = $релиз.assets | Where-Object { $_.name -match 'win64\.zip$' } | Select-Object -First 1
    if (-not $нужен) {
        throw "в релизе $версия нет сборки win64"
    }

    $zip = Join-Path $env:TEMP "protoc-$версия.zip"
    Write-Host "качаю $($нужен.name), $($нужен.size) байт"
    Invoke-WebRequest $нужен.browser_download_url -OutFile $zip -TimeoutSec 180

    New-Item -ItemType Directory -Force -Path $куда | Out-Null
    Expand-Archive -Path $zip -DestinationPath $куда -Force

    $protoc = Join-Path $куда "bin\protoc.exe"
    if (-not (Test-Path $protoc)) {
        throw "protoc.exe не появился в $куда"
    }
    Write-Host "готово: $protoc ($(& $protoc --version))"
}
finally {
    if ($ошибка) {
        exit 1
    }
}