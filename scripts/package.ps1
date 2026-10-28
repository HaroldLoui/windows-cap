# 便携版打包

param(
    [switch]$NoBuild
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$versionLine = Select-String -Path "Cargo.toml" -Pattern '^version = "(.+)"$'
$version = $versionLine.Matches[0].Groups[1].Value

if (-not $NoBuild) {
    cargo build --release
}

$dist = Join-Path $root "dist\window-cap-$version"
if (Test-Path $dist) {
    Remove-Item $dist -Recurse -Force
}
New-Item -ItemType Directory -Path $dist | Out-Null

Copy-Item "target\release\window-cap.exe" (Join-Path $dist "window-cap.exe")
if (Test-Path "assets\app.ico") {
    Copy-Item "assets\app.ico" $dist
}

$configExample = @(
    '[hotkey]',
    'capture = "Ctrl+Alt+A"',
    'quit = "Ctrl+Alt+Q"',
    '',
    '[capture]',
    'delay_ms = 0',
    '',
    '[output]',
    'save_dir = "Pictures/Screenshots"',
    'format = "png"',
    '',
    '[ui]',
    'language = "zh-CN"'
) -join "`n"
[System.IO.File]::WriteAllText((Join-Path $dist "config.example.toml"), $configExample)

$readme = @(
    "window-cap $version 便携版",
    '',
    '运行:',
    '  window-cap.exe                 托盘常驻（Ctrl+Alt+A 截图，Ctrl+Alt+Q 退出）',
    '  window-cap.exe --capture ...   无 UI 采集',
    '  window-cap.exe --help          全部参数',
    '',
    '配置文件:',
    '  %APPDATA%\window-cap\config.toml（首次运行自动生成）',
    '  可直接使用同目录 config.example.toml 作为模板。',
    '',
    '开机自启:',
    '  window-cap.exe --autostart on|off|status，或托盘菜单开关。'
) -join "`n"
[System.IO.File]::WriteAllText((Join-Path $dist "README.txt"), $readme)

$zip = Join-Path $root "dist\window-cap-$version-portable.zip"
if (Test-Path $zip) {
    Remove-Item $zip -Force
}
Compress-Archive -Path (Join-Path $dist "*") -DestinationPath $zip -CompressionLevel Optimal
Write-Host "打包完成: $zip"
