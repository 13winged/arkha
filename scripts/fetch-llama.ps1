# Принести движок llama.cpp для сборки установщика.
#
# Зачем. Движок не лежит в репозитории: папка data\bin в .gitignore,
# потому что это 712 МиБ. Из-за этого установленная сборка не может
# запустить модель — рядом с exe ничего нет, а искать больше негде.
# Проверено 15.10.2026.
#
# Что приносим. Только сборку для процессора:
#
#   llama-b11583-bin-win-cpu-x64.zip    18.6 МиБ
#
# Это решение владельца: библиотеки CUDA весят
# 523 МиБ и грузятся только когда на машине есть карта. Машине без
# видеокарты они не нужны, а 19 МиБ движка помещаются в установщик,
# и профили T1 и T2 работают сразу после установки.
#
# Куда: src-tauri\resources\bin\llama. Папка в .gitignore — она
# получается скачиванием, и держать 19 МиБ двоичных файлов в
# истории git незачем.
#
# Лицензия движка: MIT, ggml-org/llama.cpp. При поставке двоичных
# файлов в установщик текст лицензии обязан туда попасть —
# см. docs\09-лицензии.md.

$ошибка = 1
$тег = "b11583"
$куда = Join-Path $PSScriptRoot "..\src-tauri\resources\bin\llama"
$маркер = Join-Path $куда "llama-server.exe"

if (Test-Path $маркер) {
    Write-Host "движок уже на месте: $куда"
    exit 0
}

$api = "https://api.github.com/repos/ggml-org/llama.cpp/releases/tags/$тег"
$имя = "llama-$тег-bin-win-cpu-x64.zip"
$адрес = "https://github.com/ggml-org/llama.cpp/releases/download/$тег/$имя"

try {
    $ошибка = 0

    # Имя файла берётся из списка активов, а не пишется руками.
    # Ручное имя уже стоило нам двух неверных скачиваний в другой
    # сессии: релиз есть, а файла с таким именем нет.
    $релиз = Invoke-RestMethod $api -Headers @{ "User-Agent" = "arkha" } -TimeoutSec 30
    $актив = $релиз.assets | Where-Object { $_.name -eq $имя } | Select-Object -First 1
    if (-not $актив) {
        throw "в релизе $тег нет файла $имя"
    }
    $ожидаемо = [int64]$актив.size
    Write-Host "скачиваю $имя, $([int]($ожидаемо/1MB)) МиБ"

    $zip = Join-Path $env:TEMP $имя
    Invoke-WebRequest $актив.browser_download_url -OutFile $zip -TimeoutSec 600

    $получено = (Get-Item $zip).Length
    if ($получено -ne $ожидаемо) {
        throw "размер не сошёлся: ждали $ожидаемо, получили $получено"
    }

    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $архив = [System.IO.Compression.ZipFile]::OpenRead($zip)

    # Только то, чем ArkHa пользуется, плюс все библиотеки.
    # `llama.dll` подгружает `ggml-cpu-*.dll` во время работы по
    # микроархитектуре процессора, поэтому набор этой ветки берётся
    # целиком: отфильтровать его по названию нельзя, нужный файл
    # выбирается на той же машине, что его загрузит.
    $нужны = @()
    $мусор = 0
    foreach ($э in $архив.Entries) {
        if ($э.Length -eq 0) { continue }
        $расширение = [System.IO.Path]::GetExtension($э.Name)
        if ($расширение -eq ".dll") {
            $нужны += $э
        }
        elseif ($э.Name -eq "llama-server.exe" -or $э.Name -eq "llama-tts.exe") {
            $нужны += $э
        }
        else {
            $мусор += $э.Length
        }
    }
    $архив.Dispose()

    if ($нужны.Count -eq 0) {
        throw "в архиве не нашлось ни нужных файлов, ни библиотек"
    }

    New-Item -ItemType Directory -Force -Path $куда | Out-Null
    [System.IO.Compression.ZipFile]::ExtractToDirectory($zip, $куда)

    # Лицензия движка обязана ехать вместе с двоичными файлами.
    $лицензия = Join-Path $PSScriptRoot "..\src-tauri\resources\bin\LICENSE-llama.cpp"
    if (-not (Test-Path $лицензия)) {
        New-Item -ItemType Directory -Force -Path (Split-Path $лицензия -Parent) | Out-Null
        Invoke-WebRequest "https://raw.githubusercontent.com/ggml-org/llama.cpp/$тег/LICENSE" `
            -OutFile $лицензия -TimeoutSec 60
    }

    $размер = (Get-ChildItem $куда -File | Measure-Object Length -Sum).Sum
    Write-Host "готово: $куда"
    Write-Host ("  файлов: " + (Get-ChildItem $куда -File).Count + ", весят " + [int]($размер/1MB) + " МиБ")
    Write-Host ("  пропущено лишних exe на " + [int]($мусор/1MB) + " МиБ")
    Write-Host ("  llama-server.exe: " + (Test-Path $маркер))
}
finally {
    if ($ошибка) {
        exit 1
    }
}