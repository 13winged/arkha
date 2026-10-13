$ErrorActionPreference = "Stop"
$root = "C:\Users\winged\src\archotic"
$bin = "$root\data\bin"
$env:PATH = "$bin\cudart;$env:PATH"
$model = "$env:APPDATA\Archotic\models\Qwen3.5-9B-Q5_K_M.gguf"
$port = 18111

Get-Process llama-server -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 2

$args = @(
  "-m", $model,
  "-ngl", "99", "-c", "16384",
  "-ctk", "q4_0", "-ctv", "q4_0",
  "--jinja", "--reasoning", "off", "-lv", "4",
  "--port", $port.ToString(),
  "--host", "127.0.0.1"
)
$log = "$env:TEMP\probe.log"
Remove-Item $log -Force -ErrorAction SilentlyContinue
$p = Start-Process -FilePath "$bin\llama\llama-server.exe" -ArgumentList $args `
  -WorkingDirectory $bin -PassThru -RedirectStandardError $log -RedirectStandardOutput "$env:TEMP\probe.out"

$ready = $false
for ($i = 0; $i -lt 120; $i++) {
  Start-Sleep -Seconds 2
  try {
    $h = Invoke-RestMethod -Uri "http://127.0.0.1:$port/health" -TimeoutSec 3
    if ($h.status -eq "ok") { $ready = $true; break }
  } catch {}
}
if (-not $ready) { Write-Output "СЕРВЕР НЕ ПОДНЯЛСЯ"; Get-Content $log -Tail 30; Stop-Process $p -Force; exit 1 }
Write-Output "сервер готов"

# Папка с кириллицей — именно как в гейте. Если сработает, дело не в
# имени папки.
$ascii = "C:\Users\winged\Desktop\scripts\probe-ascii"
$kir = "C:\Users\winged\Desktop\scripts\probe-папка"
New-Item -ItemType Directory -Force -Path $ascii | Out-Null
New-Item -ItemType Directory -Force -Path $kir | Out-Null
Set-Content -Path "$ascii\заметка.txt" -Value "Купить хлеб и молоко до 18:00" -Encoding UTF8
Set-Content -Path "$kir\заметка.txt" -Value "Купить хлеб и молоко до 18:00" -Encoding UTF8

$trials = 4
$script:прошло = 0
foreach ($folder in @($ascii)) {
  Write-Output ""
  Write-Output ("######## ПАПКА: " + $folder + " ########")
  for ($t = 0; $t -lt $trials; $t++) {

$tools = @(
  @{
    type = "function"
    function = @{
      name = "fs.list"
      description = "Показать, что лежит в папке. Можно читать только внутри: $folder"
      parameters = @{
        type = "object"
        properties = @{ path = @{ type = "string"; description = "Полный путь к папке" } }
        required = @("path")
      }
    }
  },
  @{
    type = "function"
    function = @{
      name = "fs.read"
      description = "Прочитать текстовый файл. Можно читать только внутри: $folder. Список папки показывает только имена и размеры — чтобы узнать, что внутри файла, вызови чтение."
      parameters = @{
        type = "object"
        properties = @{ path = @{ type = "string"; description = "Полный путь к файлу" } }
        required = @("path")
      }
    }
  }
)

# Две системные строки под сравнение: короткая и полная из
# `agent::live`. Если полная ломает поведение, дело в ней.
$sysShort = "Ты — локальный ИИ-секретарь для Windows. Отвечай по-русски, кратко. У тебя есть инструменты."
$sysFull = @"
Ты — локальный ИИ-секретарь для Windows. Отвечай по-русски, кратко.

У тебя есть инструменты. Если задача требует узнать что-то о файлах или папках — вызови инструмент, а не отвечай по памяти и не отвечай, что у тебя нет доступа: доступ есть ровно у тех папок, что перечислены в описании инструментов.

Если инструмент отказал, скажи пользователю, что не вышло и почему. Не повторяй тот же вызов.

Список папки показывает только имена и размеры. Чтобы узнать, что внутри файла, вызови чтение — по одному списку ответить нельзя.

Никогда не выдумывай содержимое файла. Прочитать файл можно только инструментом.
"@

$task = "Найди файл заметка.txt среди разрешённых папок и ответь одним предложением, что в нём написано. Если не найдёшь — так и скажи, не выдумывай."
$system = if ($env:PROBE_SYS -eq "full") { $sysFull } else { $sysShort }

$body = @{
  messages = @(
    @{ role = "system"; content = $system },
    @{ role = "user"; content = $task }
  )
  tools = $tools
  max_tokens = 300
  stream = $false
  temperature = 0.3
} | ConvertTo-Json -Depth 10

$r1 = Invoke-RestMethod -Uri "http://127.0.0.1:$port/v1/chat/completions" -Method Post `
  -Body ([System.Text.Encoding]::UTF8.GetBytes($body)) -ContentType "application/json"
$call = $r1.choices[0].message.tool_calls[0]
Write-Output ("ПЕРВЫЙ ВЫЗОВ: " + $call.function.name + " " + $call.function.arguments)

# Вручную подставляем результат — ровно так, как это делает наш цикл.
$body2 = @{
  messages = @(
    @{ role = "system"; content = $system },
    @{ role = "user"; content = $task },
    @{ role = "assistant"; content = $null; tool_calls = $r1.choices[0].message.tool_calls },
    @{ role = "tool"; tool_call_id = $call.id; name = "fs.list"; content = "заметка.txt (31 байт)`n[всего: папок 0, файлов 1]" }
  )
  tools = $tools
  max_tokens = 300
  stream = $false
  temperature = 0.3
} | ConvertTo-Json -Depth 10

$r2 = Invoke-RestMethod -Uri "http://127.0.0.1:$port/v1/chat/completions" -Method Post `
  -Body ([System.Text.Encoding]::UTF8.GetBytes($body2)) -ContentType "application/json"

$m2 = $r2.choices[0].message
    if ($m2.tool_calls) {
      $ok = $false
      $name = $m2.tool_calls[0].function.name
      if ($name -eq "fs.read") {
        # Доводим цепочку до конца, как это сделал бы цикл.
        $args = $m2.tool_calls[0].function.arguments | ConvertFrom-Json
        $body3 = @{
          messages = @(
            @{ role = "system"; content = $system },
            @{ role = "user"; content = $task },
            @{ role = "assistant"; content = $null; tool_calls = $r1.choices[0].message.tool_calls },
            @{ role = "tool"; tool_call_id = $call.id; name = "fs.list"; content = "заметка.txt (31 байт)`n[всего: папок 0, файлов 1]" },
            @{ role = "assistant"; content = $null; tool_calls = $m2.tool_calls },
            @{ role = "tool"; tool_call_id = $m2.tool_calls[0].id; name = "fs.read"; content = "Купить хлеб и молоко до 18:00" }
          )
          tools = $tools
          max_tokens = 300
          stream = $false
          temperature = 0.3
        } | ConvertTo-Json -Depth 10
        $r3 = Invoke-RestMethod -Uri "http://127.0.0.1:$port/v1/chat/completions" -Method Post `
          -Body ([System.Text.Encoding]::UTF8.GetBytes($body3)) -ContentType "application/json"
        $a3 = $r3.choices[0].message.content
        $ok = ($a3 -match "хлеб" -or $a3 -match "молоко" -or $a3 -match "18:00")
        Write-Output ("  [$t] вызвал fs.read -> итог: " + $a3)
      } else {
        Write-Output ("  [$t] вызвал $name (не чтение) -> стоп")
      }
      if (-not $ok) { $ok = $false }
    } else {
      $ok = ($m2.content -match "хлеб" -or $m2.content -match "молоко")
      Write-Output ("  [$t] без вызова -> " + $m2.content)
    }
    if ($ok) { $script:прошло++ }
  }
  Write-Output ("  ИТОГ по папке: $script:прошло")
  $script:прошло = 0
}

Stop-Process $p -Force -ErrorAction SilentlyContinue