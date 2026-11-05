<#
.SYNOPSIS
  长稳回归：连续采集压力 + 长时间常驻挂机，结果写入 CSV。

.DESCRIPTION
  发布前在目标机上跑一轮：
  1. 连续采集 --soak N 次（检查 GDI/USER 句柄与内存泄漏）
  2. 常驻挂机，每隔若干分钟无 UI 采集一次，记录耗时/内存/句柄

  结果目录：%LOCALAPPDATA%\window-cap\logs
    soak-<时间戳>.csv          每次采样的指标
    soak-<时间戳>-summary.txt  汇总与结论

.EXAMPLE
  .\scripts\soak-24h.ps1
  .\scripts\soak-24h.ps1 -Hours 2 -IntervalMinutes 5 -CaptureCount 50
#>
param(
    [double]$Hours = 24,
    [int]$IntervalMinutes = 15,
    [int]$CaptureCount = 200,
    [string]$Exe = '',
    [switch]$Force
)

$ErrorActionPreference = 'Stop'

if (-not ('WindowCapSoak.Win32' -as [type])) {
    Add-Type -Namespace WindowCapSoak -Name Win32 -MemberDefinition @'
[DllImport("user32.dll")] public static extern uint GetGuiResources(IntPtr hProcess, uint uiFlags);
[DllImport("kernel32.dll")] public static extern IntPtr OpenProcess(uint dwDesiredAccess, bool bInheritHandle, uint dwProcessId);
[DllImport("kernel32.dll")] public static extern bool CloseHandle(IntPtr hObject);
'@
}

# 步骤 1：定位可执行文件与日志目录。
$root = Split-Path -Parent $PSScriptRoot
if ([string]::IsNullOrWhiteSpace($Exe)) {
    $Exe = Join-Path $root 'target\release\window-cap.exe'
}
if (-not (Test-Path $Exe)) {
    throw "未找到 $Exe；请先执行 cargo build --release"
}

$logDir = Join-Path $env:LOCALAPPDATA 'window-cap\logs'
New-Item -ItemType Directory -Force -Path $logDir | Out-Null
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$csv = Join-Path $logDir "soak-$stamp.csv"
$summaryPath = Join-Path $logDir "soak-$stamp-summary.txt"
$shotDir = Join-Path $env:TEMP "window-cap-soak"
New-Item -ItemType Directory -Force -Path $shotDir | Out-Null

function Invoke-WindowCap([string[]]$Arguments) {
    # GUI 子系统的 exe 用 & 调用不会等待，这里显式等待并取退出码。
    $proc = Start-Process -FilePath $Exe -ArgumentList $Arguments -PassThru -Wait
    return $proc.ExitCode
}

function Get-LogTail([string]$pattern, [int]$count = 1) {
    $logPath = Join-Path $logDir 'app.log'
    if (-not (Test-Path $logPath)) { return @() }
    return @(Select-String -Path $logPath -Pattern $pattern | Select-Object -Last $count |
        ForEach-Object { $_.Line })
}

function Get-ProcessMetrics([int]$processId) {
    $handle = [WindowCapSoak.Win32]::OpenProcess(0x1400, $false, $processId)
    $gdi = 0
    $user = 0
    if ($handle -ne [IntPtr]::Zero) {
        $gdi = [WindowCapSoak.Win32]::GetGuiResources($handle, 0)
        $user = [WindowCapSoak.Win32]::GetGuiResources($handle, 1)
        [void][WindowCapSoak.Win32]::CloseHandle($handle)
    }
    return [pscustomobject]@{ Gdi = $gdi; User = $user }
}

# 步骤 2：确保没有旧实例干扰（否则热键/句柄统计会混入别的进程）。
$existing = @(Get-Process window-cap -ErrorAction SilentlyContinue)
if ($existing.Count -gt 0) {
    if (-not $Force) {
        throw "检测到 $($existing.Count) 个 window-cap 进程；确认无重要截图后加 -Force 重跑"
    }
    $existing | Stop-Process -Force
    Start-Sleep -Seconds 2
}
Remove-Item (Join-Path $logDir 'session.flag') -ErrorAction SilentlyContinue

"time,event,elapsed_ms,memory_mb,gdi,user,handles" | Set-Content -Path $csv -Encoding utf8
$failures = New-Object System.Collections.Generic.List[string]
$samples = New-Object System.Collections.Generic.List[object]
$resident = $null
$soakResult = ''
$probe = 0

try {
    # 步骤 3：连续采集压力测试。
    Write-Host "[1/2] 连续采集 $CaptureCount 次…"
    $soakExit = Invoke-WindowCap @('--soak', "$CaptureCount")
    $soakLines = @(Get-LogTail 'soak 结束' 1)
    if ($soakLines.Count -gt 0) {
        $soakResult = $soakLines[0]
    } else {
        $soakResult = "exit=$soakExit"
    }
    "$(Get-Date -Format 'o'),soak-done$soakExit,,,,," | Add-Content -Path $csv -Encoding utf8
    if ($soakExit -ne 0) { $failures.Add("soak 退出码 $soakExit") }
    Write-Host "      $soakResult"
    if ($soakResult -match 'GDI=\d+ \(([+-]?\d+)\)') {
        $gdiDelta = [int]$Matches[1]
        if ($gdiDelta -gt 20) { $failures.Add("soak 后 GDI 句柄增长 $gdiDelta") }
    }

    # 步骤 4：常驻挂机 + 周期采集。
    Write-Host "[2/2] 常驻挂机 $Hours 小时，每 $IntervalMinutes 分钟采集一次…"
    $resident = Start-Process -FilePath $Exe -ArgumentList '--hotkey' -PassThru
    Start-Sleep -Seconds 5
    if ($resident.HasExited) {
        throw "常驻实例启动后立即退出（exit=$($resident.ExitCode)），请查看 app.log"
    }

    $deadline = (Get-Date).AddHours($Hours)
    $firstMemory = $null
    while ((Get-Date) -lt $deadline) {
        Start-Sleep -Seconds ($IntervalMinutes * 60)
        if ($resident.HasExited) {
            $failures.Add("常驻实例在挂机期间退出（exit=$($resident.ExitCode)）")
            break
        }

        $probe++
        $shot = Join-Path $shotDir "probe-$probe.png"
        $startedAt = Get-Date
        $exit = Invoke-WindowCap @('--capture', '--output', $shot, '--json')
        $elapsedMs = 0
        $jsonLines = @(Get-LogTail '"elapsed_ms"' 1)
        if ($jsonLines.Count -gt 0 -and $jsonLines[0] -match '"elapsed_ms":([0-9.]+)') {
            $elapsedMs = [double]$Matches[1]
        }

        $resident.Refresh()
        $memoryMb = [math]::Round($resident.WorkingSet64 / 1MB, 1)
        $metrics = Get-ProcessMetrics $resident.Id
        if ($null -eq $firstMemory) { $firstMemory = $memoryMb }

        $samples.Add([pscustomobject]@{
            Elapsed  = $elapsedMs
            MemoryMb = $memoryMb
        })
        "$($startedAt.ToString('o')),capture-$probe,$elapsedMs,$memoryMb,$($metrics.Gdi),$($metrics.User),$($resident.HandleCount)" |
            Add-Content -Path $csv -Encoding utf8

        if ($exit -ne 0) { $failures.Add("第 $probe 次周期采集退出码 $exit") }
        if ($elapsedMs -gt 2000) { $failures.Add("第 $probe 次采集耗时 $elapsedMs ms 超过 2000 ms") }
        if ($memoryMb -gt 400) { $failures.Add("常驻内存 $memoryMb MB 超过 400 MB") }
        Write-Host ("      #{0} {1:HH:mm:ss} elapsed={2:N1}ms mem={3}MB gdi={4} user={5} handles={6}" -f
            $probe, $startedAt, $elapsedMs, $memoryMb, $metrics.Gdi, $metrics.User, $resident.HandleCount)
    }
}
finally {
    # 步骤 5：收尾（无论成败都结束常驻实例并清掉会话标记）。
    if ($null -ne $resident -and -not $resident.HasExited) {
        Stop-Process -Id $resident.Id -Force
    }
    Start-Sleep -Seconds 1
    Remove-Item (Join-Path $logDir 'session.flag') -ErrorAction SilentlyContinue
}

# 步骤 6：输出结论。
$maxMemory = 0
$maxElapsed = 0
if ($samples.Count -gt 0) {
    $maxMemory = ($samples | Measure-Object -Property MemoryMb -Maximum).Maximum
    $maxElapsed = ($samples | Measure-Object -Property Elapsed -Maximum).Maximum
    if ($null -ne $firstMemory -and ($maxMemory - $firstMemory) -gt 50) {
        $failures.Add("内存从 $firstMemory MB 增长到 $maxMemory MB（>50 MB）")
    }
}

$verdict = if ($failures.Count -eq 0) { 'PASS' } else { 'FAIL' }
$lines = @(
    "window-cap 长稳回归 $stamp",
    "时长: $Hours h / 采样间隔: $IntervalMinutes min / 周期采集: $probe 次 / 连拍: $CaptureCount 次",
    "结论: $verdict",
    "soak: $soakResult",
    "最大采集耗时: $([math]::Round($maxElapsed, 1)) ms",
    "最大常驻内存: $maxMemory MB",
    "采样明细: $csv"
)
if ($failures.Count -gt 0) {
    $lines += '问题:'
    $lines += ($failures | ForEach-Object { "  - $_" })
}
$lines | Set-Content -Path $summaryPath -Encoding utf8
$lines | ForEach-Object { Write-Host $_ }

if ($verdict -eq 'PASS') { exit 0 } else { exit 1 }
