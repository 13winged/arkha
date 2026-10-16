# Испытательное окно для гейта S8.
#
# Настоящее окно Win32 с настоящими кнопками. Нужно для проверки
# клика: элемент ищется по своему тексту через EnumChildWindows, и
# проверить это можно только на реальном окне.
#
# Кнопка «Сохранить» пишет содержимое поля в файл, переданный первым
# аргументом. Так результат клика можно проверить, а не только то,
# что клик куда-то ушёл.
#
# Идентификаторы латиницей намеренно: кириллица в именах переменных
# PowerShell работает, но лишняя причина ломаться тут не нужна.
# Кириллица есть только в тексте интерфейса — там она и проверяется.

param([Parameter(Mandatory = $true)][string]$OutFile)

Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing

$form = New-Object System.Windows.Forms.Form
$form.Text = "Проверка ArkHa"
$form.Width = 560
$form.Height = 360

$box = New-Object System.Windows.Forms.TextBox
$box.Multiline = $true
$box.Dock = "Fill"
$form.Controls.Add($box)

$clear = New-Object System.Windows.Forms.Button
$clear.Text = "Очистить"
$clear.Dock = "Bottom"
$clear.Height = 36
$clear.Add_Click({ $box.Clear() })
$form.Controls.Add($clear)

$save = New-Object System.Windows.Forms.Button
$save.Text = "Сохранить"
$save.Dock = "Bottom"
$save.Height = 36
$save.Add_Click({
    [System.IO.File]::WriteAllText($OutFile, $box.Text, [System.Text.UTF8Encoding]::new($false))
})
$form.Controls.Add($save)

[void]$form.ShowDialog()