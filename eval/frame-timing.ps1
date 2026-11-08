# 大图帧预算测量：外部进程读 `phase: frame` 时间戳，算帧间隔分布。
#
# 不在 arch-view 源码里取墙钟；时间戳全部由本脚本提供。
#
# 用法：
#   pwsh -File eval/frame-timing.ps1 -Mode idle
#   pwsh -File eval/frame-timing.ps1 -Mode reveal
#   pwsh -File eval/frame-timing.ps1 -Mode pan -Frames 120
#   pwsh -File eval/frame-timing.ps1 -Mode pan -Frames 120 -Skip "labels"
#   pwsh -File eval/frame-timing.ps1 -Mode pan -Frames 120 -Skip "edges,labels"
#
# 输出：stdout 读数表 + JSON 明细（含原始帧间隔）。

[CmdletBinding()]
param(
    [string]$Repo = "tests/corpus/excalidraw",
    [ValidateSet("pan", "reveal", "idle")]
    [string]$Mode = "pan",
    [int]$Frames = 120,
    [string]$Skip = "",
    [int]$IdleMs = 5000,
    [string]$Exe = "target/release/arch-view.exe",
    [string]$OutJson = "out/frame-timing.json"
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$suffix = if ($IsWindows) { ".exe" } else { "" }
if (-not $Exe.EndsWith($suffix)) { $Exe = "$Exe$suffix" }
$exePath = (Resolve-Path $Exe).Path

$arguments = @("map", $Repo)
$envVars = @{ STRATA_PHASE_MARKERS = "1"; STRATA_FRAME_TRACE = "1" }
$windowTag = ""

switch ($Mode) {
    "pan" {
        $envVars["STRATA_AUTOPAN_FRAMES"] = "$Frames"
        $autoCloseMs = 2500 + ($Frames * 250) + 8000
        $windowTag = "pan"
    }
    "reveal" {
        $arguments += "--reveal"
        $autoCloseMs = 35000
        $windowTag = "reveal"
    }
    "idle" {
        $autoCloseMs = 2500 + $IdleMs + 5000
        $windowTag = "idle"
    }
}
if ($Skip) { $envVars["STRATA_RENDER_SKIP"] = $Skip }
$arguments += @("--auto-close-ms", "$autoCloseMs")

$psi = [System.Diagnostics.ProcessStartInfo]::new()
$psi.FileName = $exePath
$psi.UseShellExecute = $false
$psi.RedirectStandardOutput = $true
$psi.RedirectStandardError = $true
foreach ($arg in $arguments) { $psi.ArgumentList.Add($arg) }
foreach ($key in $envVars.Keys) { $psi.Environment[$key] = $envVars[$key] }

$watch = [System.Diagnostics.Stopwatch]::StartNew()
$process = [System.Diagnostics.Process]::new()
$process.StartInfo = $psi
$process.Start() | Out-Null

$marks = [ordered]@{}
$frameTimes = New-Object System.Collections.Generic.List[double]
$panFrameTimes = New-Object System.Collections.Generic.List[double]
$firstLine = ""
$counters = ""
$line = $process.StandardOutput.ReadLine()
while ($null -ne $line) {
    $elapsed = [double]$watch.Elapsed.TotalMilliseconds
    if ($line.StartsWith("phase: ")) {
        $label = $line.Substring(7).Trim()
        if (-not $marks.Contains($label)) { $marks[$label] = $elapsed }
        if ($label -eq "frame") { $frameTimes.Add($elapsed) }
        if ($label -eq "pan:frame") { $panFrameTimes.Add($elapsed) }
    } elseif ($line.StartsWith("frame: ")) {
        $counters = $line
    } elseif ($line.StartsWith("map: ") -and -not $firstLine) {
        $firstLine = $line
    }
    $line = $process.StandardOutput.ReadLine()
}
$stderr = $process.StandardError.ReadToEnd()
$process.WaitForExit()
$watch.Stop()

function Get-Stats {
    param([double[]]$Values)
    if ($Values.Count -eq 0) {
        return [pscustomobject]@{ count = 0; median = $null; p95 = $null; min = $null; max = $null; mean = $null }
    }
    $sorted = @($Values | Sort-Object)
    $count = $sorted.Count
    $median = if ($count % 2 -eq 1) {
        $sorted[[int](($count - 1) / 2)]
    } else {
        ($sorted[$count / 2 - 1] + $sorted[$count / 2]) / 2
    }
    $p95Index = [math]::Max(0, [math]::Ceiling(0.95 * $count) - 1)
    return [pscustomobject]@{
        count = $count
        median = [math]::Round($median, 2)
        p95 = [math]::Round($sorted[$p95Index], 2)
        min = [math]::Round($sorted[0], 2)
        max = [math]::Round($sorted[$count - 1], 2)
        mean = [math]::Round(($Values | Measure-Object -Average).Average, 2)
    }
}

function Get-Intervals {
    param([double[]]$Times)
    $intervals = New-Object System.Collections.Generic.List[double]
    for ($index = 1; $index -lt $Times.Count; $index += 1) {
        $intervals.Add($Times[$index] - $Times[$index - 1])
    }
    return $intervals.ToArray()
}

function Get-WindowTimes {
    param([double[]]$Times, $Start, $End)
    return @($Times | Where-Object { $_ -ge $Start -and $_ -le $End })
}

$startMark = $marks["content:presented"]
if ($null -eq $startMark) { $startMark = $marks["reveal:start"] }
if ($null -eq $startMark) { $startMark = 0 }

$intervals = @()
switch ($Mode) {
    "pan" {
        $panStart = if ($panFrameTimes.Count -gt 0) { $panFrameTimes[0] } else { $startMark }
        $panEnd = if ($marks.Contains("pan:done")) { $marks["pan:done"] } else { $watch.Elapsed.TotalMilliseconds }
        $intervals = Get-Intervals (Get-WindowTimes $frameTimes $panStart $panEnd)
    }
    "reveal" {
        $revealStart = if ($marks.Contains("reveal:start")) { $marks["reveal:start"] } else { $startMark }
        $revealEnd = if ($marks.Contains("reveal:done")) { $marks["reveal:done"] } else { $watch.Elapsed.TotalMilliseconds }
        $intervals = Get-Intervals (Get-WindowTimes $frameTimes $revealStart $revealEnd)
    }
    "idle" {
        $idleStart = $startMark + 500
        $idleEnd = $startMark + 500 + $IdleMs
        $idleFrames = @($frameTimes | Where-Object { $_ -gt $idleStart -and $_ -le $idleEnd }).Count
        $stats = [pscustomobject]@{ count = $idleFrames; median = 0; p95 = 0; min = 0; max = 0; mean = 0 }
    }
}

if ($Mode -ne "idle") { $stats = Get-Stats $intervals }
$fps = if ($stats.median -and $stats.median -gt 0) { [math]::Round(1000 / $stats.median, 1) } else { $null }

$detail = [pscustomobject]@{
    mode = $Mode
    repo = $Repo
    skip = $Skip
    framesTotal = $frameTimes.Count
    panFrames = $panFrameTimes.Count
    autoCloseMs = $autoCloseMs
    exitCode = $process.ExitCode
    counters = $counters
    mapLine = $firstLine
    prepDoneMs = $marks["map:prep:done"]
    contentPresentedMs = $marks["content:presented"]
    revealStartMs = $marks["reveal:start"]
    revealDoneMs = $marks["reveal:done"]
    panDoneMs = $marks["pan:done"]
    intervals = [pscustomobject]@{
        count = $stats.count
        medianMs = $stats.median
        p95Ms = $stats.p95
        minMs = $stats.min
        maxMs = $stats.max
        meanMs = $stats.mean
        fpsFromMedian = $fps
    }
    rawIntervalsMs = @($intervals | ForEach-Object { [math]::Round($_, 2) })
    stderrTail = @(($stderr -split "`n") | Where-Object { $_ -and $_.Trim() } | Select-Object -Last 3)
}
$outDir = Split-Path -Parent $OutJson
if ($outDir -and -not (Test-Path -LiteralPath $outDir)) {
    New-Item -ItemType Directory -Force -Path $outDir | Out-Null
}
$detail | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $OutJson -Encoding utf8

Write-Output ("frame timing: mode={0} skip={1} repo={2}" -f $Mode, $(if ($Skip) { $Skip } else { "-" }), $Repo)
Write-Output ("  {0}" -f $firstLine)
Write-Output ("  {0}" -f $counters)
Write-Output ("  frames={0} window Intervals={1}" -f $detail.framesTotal, $stats.count)
if ($Mode -eq "idle") {
    Write-Output ("  idle frames in {0}ms window: {1}（判据要求 0）" -f $IdleMs, $stats.count)
} else {
    Write-Output ("  interval median={0}ms p95={1}ms min={2}ms max={3}ms mean={4}ms (~{5}fps)" -f `
            $stats.median, $stats.p95, $stats.min, $stats.max, $stats.mean, $fps)
    Write-Output ("  reveal={0}ms prepDone={1}ms contentPresented={2}ms panDone={3}ms" -f `
            $(if ($null -ne $detail.revealDoneMs -and $null -ne $detail.revealStartMs) { $detail.revealDoneMs - $detail.revealStartMs } else { "-" }), `
            $detail.prepDoneMs, $detail.contentPresentedMs, $detail.panDoneMs)
}
Write-Output ("  exit={0} wrote {1}" -f $process.ExitCode, $OutJson)

if ($process.ExitCode -ne 0) { exit 1 }
exit 0
