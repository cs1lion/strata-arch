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
#   pwsh -File eval/frame-timing.ps1 -Mode search -SearchPrefix "/components"
#
# search 模式：外部线程向窗口 PostMessage 逐字输入，靠应用自己打的 `phase: key:*`
# 界定输入窗口，读该窗口内 `phase: frame` 的间隔（输入期间帧）。仅 Windows 实现；
# 其他平台直接报错退出，不产假读数。
#
# 输出：stdout 读数表 + JSON 明细（含原始帧间隔）。

[CmdletBinding()]
param(
    [string]$Repo = "tests/corpus/excalidraw",
    [ValidateSet("pan", "reveal", "idle", "ab", "search", "stability")]
    [string]$Mode = "pan",
    # search 模式：输入内容与逐字间隔（毫秒）。
    [string]$SearchPrefix = "/components",
    [int]$SearchKeyDelayMs = 25,
    # 从进程启动到开始输入前的等待；必须大于语料加载时间，否则按键落在加载态上。
    [int]$SearchStartDelayMs = 4000,
    [int]$Frames = 120,
    # stability 模式：固定负载（同进程、同配置、往返平移）下重复的分段数；每分段 Frames 帧。
    # 段间差异即「测量噪声地板」，用于判断 20% / 33ms / 路线收益是否可分辨。
    [int]$StabilitySegments = 12,
    [string]$Skip = "",
    [string]$AbCycle = "base,B,BA",
    [int]$AbSegmentFrames = 60,
    [int]$IdleMs = 5000,
    # reveal 模式的自动退出上限；软件渲染/低端机上可调大，正常路径到 `reveal:done` 后仍要等它。
    [int]$RevealAutoCloseMs = 30000,
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
        # 揭示默认开启，但平移/空闲/交替测的是稳态帧率，显式关闭揭示以免把 80 帧动画算进去。
        $arguments += "--no-reveal"
        $envVars["STRATA_AUTOPAN_FRAMES"] = "$Frames"
        $autoCloseMs = 2500 + ($Frames * 250) + 8000
        $windowTag = "pan"
    }
    "reveal" {
        $arguments += "--reveal"
        # 揭示是固定 900ms 时长驱动（帧率只决定中间帧疏密），正常路径到 `reveal:done`
        # 就退出；这个上限只是软件渲染/低端机上的自动退出兜底。
        $autoCloseMs = $RevealAutoCloseMs
        $windowTag = "reveal"
    }
    "idle" {
        $arguments += "--no-reveal"
        $autoCloseMs = 2500 + $IdleMs + 5000
        $windowTag = "idle"
    }
    "search" {
        if (-not $IsWindows) {
            Write-Output "frame timing: mode=search 仅实现 Windows 按键注入（PostMessage）；本平台未测"
            exit 2
        }
        $arguments += "--no-reveal"
        # 输入窗口约 SearchPrefix.Length * (延迟 + 处理) 毫秒；起点由 SearchStartDelayMs 控制。
        $autoCloseMs = $SearchStartDelayMs + ($SearchPrefix.Length * ($SearchKeyDelayMs + 40)) + 4000
        $windowTag = "search"
    }
    "ab" {
        # 交替测的是稳态帧率，揭示动画会把前 80 帧混进来，显式关闭。
        $arguments += "--no-reveal"
        $segments = @($AbCycle.Split(',') | Where-Object { $_.Trim() })
        $envVars["STRATA_AB_CYCLE"] = ($segments -join ",")
        $envVars["STRATA_AB_SEGMENT_FRAMES"] = "$AbSegmentFrames"
        # 末尾多跑一整段作为补齐，保证最后一段也有完整 F 帧（该段同样按帧下标切分统计）。
        $envVars["STRATA_AUTOPAN_FRAMES"] = "$($segments.Count * $AbSegmentFrames + $AbSegmentFrames)"
        $autoCloseMs = 2500 + (($segments.Count + 1) * $AbSegmentFrames * 120) + 8000
        $windowTag = "ab"
    }
    "stability" {
        # 稳定性测量：同进程、同配置、往返平移，切成 Segments 段，每段 Frames 帧。
        # 段间差异即「测量噪声地板」（无真实效应时的随机波动），用于判断 20% / 33ms / 路线收益是否可分辨。
        $arguments += "--no-reveal"
        $envVars["STRATA_AUTOPAN_FRAMES"] = "$($StabilitySegments * $Frames)"
        $autoCloseMs = 2500 + ($StabilitySegments * $Frames * 120) + 8000
        $windowTag = "stability"
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

# search 模式：独立线程按固定间隔 PostMessage 逐字输入，不改应用侧任何逻辑。
$injector = $null
if ($Mode -eq "search") {
    $injector = Start-ThreadJob -ArgumentList @($process.Id, $SearchPrefix, $SearchKeyDelayMs, $SearchStartDelayMs) -ScriptBlock {
        param($TargetPid, $Keys, $DelayMs, $StartDelayMs)
        Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Threading;

public class SearchKeyInject {
    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int Left; public int Top; public int Right; public int Bottom; }

    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, UIntPtr e);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, UIntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern int MapVirtualKey(uint c, uint t);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern short VkKeyScanW(char c);

    private static void PostChar(IntPtr hwnd, uint vk, uint scan) {
        IntPtr down = (IntPtr)(1 | ((long)scan << 16));
        IntPtr up = (IntPtr)(1 | ((long)scan << 16) | (1L << 30) | (1L << 31));
        PostMessage(hwnd, 0x0100, (UIntPtr)vk, down);
        Thread.Sleep(5);
        PostMessage(hwnd, 0x0101, (UIntPtr)vk, up);
    }

    public static int TypeText(IntPtr hwnd, string text, int delayMs) {
        int sent = 0;
        foreach (char ch in text) {
            short vks = VkKeyScanW(ch);
            uint vk = (uint)(vks & 0xFF);
            uint scan = (uint)MapVirtualKey(vk, 0);
            PostChar(hwnd, vk, scan);
            Thread.Sleep(delayMs);
            sent++;
        }
        return sent;
    }
}
'@
        $hwnd = [IntPtr]::Zero
        for ($i = 0; $i -lt 150; $i++) {
            $proc = Get-Process -Id $TargetPid -ErrorAction SilentlyContinue
            if (-not $proc) { return "process-gone" }
            if ($proc.MainWindowHandle -ne 0) { $hwnd = [IntPtr]$proc.MainWindowHandle; break }
            Start-Sleep -Milliseconds 100
        }
        if ($hwnd -eq [IntPtr]::Zero) { return "no-window" }
        # 等首屏就绪（内容加载完成）后激活窗口、点一下画布让画布拿到键盘焦点，再逐字输入。
        Start-Sleep -Milliseconds $StartDelayMs
        [SearchKeyInject]::SetForegroundWindow($hwnd) | Out-Null
        $rect = New-Object SearchKeyInject+RECT
        if ([SearchKeyInject]::GetWindowRect($hwnd, [ref]$rect)) {
            [SearchKeyInject]::SetCursorPos(($rect.Left + 30), ($rect.Top + 60)) | Out-Null
            [SearchKeyInject]::mouse_event(0x02, 0, 0, 0, [UIntPtr]::Zero)
            [SearchKeyInject]::mouse_event(0x04, 0, 0, 0, [UIntPtr]::Zero)
        }
        Start-Sleep -Milliseconds 150
        $sent = [SearchKeyInject]::TypeText($hwnd, $Keys, $DelayMs)
        return "sent=$sent"
    }
}

$marks = [ordered]@{}
$frameTimes = New-Object System.Collections.Generic.List[double]
$panFrameTimes = New-Object System.Collections.Generic.List[double]
$abMarks = New-Object System.Collections.Generic.List[object]
$abRows = @()
$abSummary = @()
$firstLine = ""
$counters = ""
$lastKeyMark = $null
$line = $process.StandardOutput.ReadLine()
while ($null -ne $line) {
    $elapsed = [double]$watch.Elapsed.TotalMilliseconds
    if ($line.StartsWith("phase: ")) {
        $label = $line.Substring(7).Trim()
        if (-not $marks.Contains($label)) { $marks[$label] = $elapsed }
        if ($label -eq "frame") { $frameTimes.Add($elapsed) }
        if ($label -eq "pan:frame") { $panFrameTimes.Add($elapsed) }
        if ($label.StartsWith("key:")) { $lastKeyMark = $elapsed }
        if ($label.StartsWith("ab:segment=")) {
            $abMarks.Add([pscustomobject]@{ name = $label.Substring(11); at = $elapsed })
        }
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

$injectorResult = ""
if ($injector) {
    $injector | Wait-Job -Timeout 10 | Out-Null
    $injectorResult = (@($injector | Receive-Job) -join " ")
    $injector | Remove-Job -Force -ErrorAction SilentlyContinue
}

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

function Get-StdDev {
    param([double[]]$Values)
    $n = $Values.Count
    if ($n -lt 2) { return $null }
    $mean = ($Values | Measure-Object -Average).Average
    $sumSq = 0.0
    foreach ($v in $Values) { $sumSq += ($v - $mean) * ($v - $mean) }
    return [math]::Round([math]::Sqrt($sumSq / ($n - 1)), 3)
}

# 稳定性摘要：把同一配置、同进程、多分段的读数当作「无效应时的噪声分布」，
# 给出段间离散度与据此推导的噪声地板（约 95% 置信下可分辨的最小效应）。
function Get-StabilitySummary {
    param([double[]]$Medians, [double[]]$P95s)
    $meanMed = ($Medians | Measure-Object -Average).Average
    $stdMed = Get-StdDev $Medians
    $cvPct = if ($meanMed -and $stdMed -ne $null) { [math]::Round($stdMed / $meanMed * 100, 2) } else { $null }
    $minMed = ($Medians | Measure-Object -Minimum).Minimum
    $maxMed = ($Medians | Measure-Object -Maximum).Maximum
    $spread = [math]::Round($maxMed - $minMed, 2)
    $spreadPct = if ($meanMed) { [math]::Round($spread / $meanMed * 100, 2) } else { $null }
    $meanP95 = ($P95s | Measure-Object -Average).Average
    $stdP95 = Get-StdDev $P95s
    $cvP95Pct = if ($meanP95 -and $stdP95 -ne $null) { [math]::Round($stdP95 / $meanP95 * 100, 2) } else { $null }
    $minP95 = ($P95s | Measure-Object -Minimum).Minimum
    $maxP95 = ($P95s | Measure-Object -Maximum).Maximum
    $spreadP95 = [math]::Round($maxP95 - $minP95, 2)
    # 噪声地板：取「2×标准差（约 95% 置信半宽）」与「段间极差」的较大者，作为保守下界。
    $floorMs = if ($stdMed -ne $null) { [math]::Round([math]::Max(2 * $stdMed, $spread), 2) } else { $spread }
    return [pscustomobject]@{
        segments = $Medians.Count
        medianMeanMs = [math]::Round($meanMed, 2)
        medianStdMs = $stdMed
        medianCvPct = $cvPct
        medianMinMs = [math]::Round($minMed, 2)
        medianMaxMs = [math]::Round($maxMed, 2)
        medianSpreadMs = $spread
        medianSpreadPct = $spreadPct
        p95MeanMs = [math]::Round($meanP95, 2)
        p95StdMs = $stdP95
        p95CvPct = $cvP95Pct
        p95MinMs = [math]::Round($minP95, 2)
        p95MaxMs = [math]::Round($maxP95, 2)
        p95SpreadMs = $spreadP95
        noiseFloorMs = $floorMs
    }
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
    "search" {
        # 输入窗口由应用自己打的 `phase: key:/` 与最后一次 `phase: key:*` 界定。
        $keyStart = $marks["key:/"]
        if ($null -ne $keyStart -and $null -ne $lastKeyMark -and $lastKeyMark -gt $keyStart) {
            $intervals = Get-Intervals (Get-WindowTimes $frameTimes $keyStart $lastKeyMark)
        } else {
            $intervals = @()
        }
    }
    "idle" {
        $idleStart = $startMark + 500
        $idleEnd = $startMark + 500 + $IdleMs
        $idleFrames = @($frameTimes | Where-Object { $_ -gt $idleStart -and $_ -le $idleEnd }).Count
        $stats = [pscustomobject]@{ count = $idleFrames; median = 0; p95 = 0; min = 0; max = 0; mean = 0 }
    }
    "ab" {
        # 帧下标即渲染序号（每个 MapView 帧输出一行 `phase: frame`，空闲时不渲染）。
        # 第 i 段固定覆盖帧 [i*F, (i+1)*F)，段内单独统计；不足 F 帧的尾段丢弃，不跨段混算。
        $abRows = @()
        $frameArray = $frameTimes.ToArray()
        $seen = @{}
        for ($index = 0; $index -lt $abMarks.Count; $index += 1) {
            $from = $index * $AbSegmentFrames
            $to = [math]::Min($from + $AbSegmentFrames, $frameArray.Count)
            if (($to - $from) -lt $AbSegmentFrames) { continue }
            $segmentIntervals = Get-Intervals @($frameArray[$from..($to - 1)])
            $segmentStats = Get-Stats $segmentIntervals
            $name = $abMarks[$index].name
            $instance = if ($seen.ContainsKey($name)) { $seen[$name] + 1 } else { 1 }
            $seen[$name] = $instance
            $abRows += [pscustomobject]@{
                segment = $name
                instance = $instance
                intervalsMs = @($segmentIntervals | ForEach-Object { [math]::Round($_, 2) })
                stats = $segmentStats
            }
        }
        $abSummary = @()
        foreach ($group in ($abRows | Group-Object segment)) {
            $medianOfMedians = (Get-Stats @($group.Group | ForEach-Object { $_.stats.median })).median
            $medianOfP95 = (Get-Stats @($group.Group | ForEach-Object { $_.stats.p95 })).median
            $abSummary += [pscustomobject]@{
                segment = $group.Name
                batches = $group.Count
                medianOfMediansMs = $medianOfMedians
                medianOfP95Ms = $medianOfP95
            }
        }
    }
    "stability" {
        # 固定负载：同进程、同配置、往返平移，切成 Segments 段，每段 Frames 帧。
        # 整段 pooled 间隔用于 `intervals` 块；段间离散度用于稳定性摘要。
        $intervals = Get-Intervals $frameTimes
        $segRows = @()
        $frameArray = $frameTimes.ToArray()
        for ($seg = 0; $seg -lt $StabilitySegments; $seg += 1) {
            $from = $seg * $Frames
            $to = [math]::Min($from + $Frames, $frameArray.Count)
            if (($to - $from) -lt $Frames) { break }
            $segIntervals = Get-Intervals @($frameArray[$from..($to - 1)])
            $segStats = Get-Stats $segIntervals
            $segRows += [pscustomobject]@{
                segment = $seg + 1
                intervalsMs = @($segIntervals | ForEach-Object { [math]::Round($_, 2) })
                stats = $segStats
            }
        }
        $segMedians = @($segRows | ForEach-Object { $_.stats.median })
        $segP95 = @($segRows | ForEach-Object { $_.stats.p95 })
        $stabilitySummary = Get-StabilitySummary -Medians $segMedians -P95s $segP95
    }
}

if ($Mode -notin @("idle", "ab")) { $stats = Get-Stats $intervals }
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
    search = [ordered]@{
        keys = $SearchPrefix
        keyDelayMs = $SearchKeyDelayMs
        keyStartMs = $marks["key:/"]
        lastKeyMs = $lastKeyMark
        injected = $injectorResult
    }
    rawIntervalsMs = @($intervals | ForEach-Object { [math]::Round($_, 2) })
    abSegments = @($abRows)
    abSummary = @($abSummary)
    stabilitySegments = @($segRows)
    stabilitySummary = $stabilitySummary
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
if ($Mode -ne "ab") {
    Write-Output ("  frames={0} window Intervals={1}" -f $detail.framesTotal, $stats.count)
}
if ($Mode -eq "idle") {
    Write-Output ("  idle frames in {0}ms window: {1}（判据要求 0）" -f $IdleMs, $stats.count)
} elseif ($Mode -eq "ab") {
    if ($abRows.Count -eq 0) {
        Write-Output "  未读到 phase: ab:segment 打点（检查 STRATA_FRAME_TRACE / STRATA_AB_CYCLE）"
    } else {
        Write-Output ("  {0,-10} {1,-7} {2,6} {3,9} {4,9} {5,9} {6,9} {7,9}" -f `
                "segment", "batch", "samples", "median", "p95", "min", "max", "mean")
        foreach ($row in $abRows) {
            Write-Output ("  {0,-10} {1,7} {2,6} {3,9} {4,9} {5,9} {6,9} {7,9}" -f `
                    $row.segment, $row.instance, $row.stats.count, $row.stats.median, $row.stats.p95, `
                    $row.stats.min, $row.stats.max, $row.stats.mean)
        }
        Write-Output ""
        Write-Output ("  {0,-10} {1,7} {2,16} {3,14}" -f "segment", "batches", "median(median)", "median(p95)")
        foreach ($row in $abSummary) {
            Write-Output ("  {0,-10} {1,7} {2,16} {3,14}" -f `
                    $row.segment, $row.batches, $row.medianOfMediansMs, $row.medianOfP95Ms)
        }
    }
} elseif ($Mode -eq "stability") {
    if ($segRows.Count -eq 0) {
        Write-Output "  未读到足够的 phase: frame 打点（检查 STRATA_FRAME_TRACE / STRATA_AUTOPAN_FRAMES）"
    } else {
        Write-Output ("  {0,-8} {1,7} {2,9} {3,9} {4,9} {5,9} {6,9}" -f `
                "segment", "samples", "median", "p95", "min", "max", "mean")
        foreach ($row in $segRows) {
            Write-Output ("  {0,-8} {1,7} {2,9} {3,9} {4,9} {5,9} {6,9}" -f `
                    $row.segment, $row.stats.count, $row.stats.median, $row.stats.p95, `
                    $row.stats.min, $row.stats.max, $row.stats.mean)
        }
        Write-Output ""
        $s = $stabilitySummary
        Write-Output ("  segments={0} medianMean={1}ms medianStd={2}ms medianCV={3}%" -f `
                $s.segments, $s.medianMeanMs, $s.medianStdMs, $s.medianCvPct)
        Write-Output ("  medianMin={0}ms medianMax={1}ms medianSpread={2}ms ({3}%)" -f `
                $s.medianMinMs, $s.medianMaxMs, $s.medianSpreadMs, $s.medianSpreadPct)
        Write-Output ("  p95Mean={0}ms p95CV={1}% p95Spread={2}ms" -f `
                $s.p95MeanMs, $s.p95CvPct, $s.p95SpreadMs)
        Write-Output ("  => noise floor (>=2x std or segment range) = {0}ms" -f $s.noiseFloorMs)
    }
} else {
    Write-Output ("  interval median={0}ms p95={1}ms min={2}ms max={3}ms mean={4}ms (~{5}fps)" -f `
            $stats.median, $stats.p95, $stats.min, $stats.max, $stats.mean, $fps)
    if ($Mode -eq "search") {
        Write-Output ("  keys={0} injected={1}" -f $SearchPrefix, $injectorResult)
        Write-Output ("  keyWindow={0}ms keyStart={1}ms lastKey={2}ms" -f `
                $(if ($null -ne $marks["key:/"] -and $null -ne $lastKeyMark) { [math]::Round($lastKeyMark - $marks["key:/"], 1) } else { "-" }), `
                $marks["key:/"], $lastKeyMark)
    }
    Write-Output ("  reveal={0}ms prepDone={1}ms contentPresented={2}ms panDone={3}ms" -f `
            $(if ($null -ne $detail.revealDoneMs -and $null -ne $detail.revealStartMs) { $detail.revealDoneMs - $detail.revealStartMs } else { "-" }), `
            $detail.prepDoneMs, $detail.contentPresentedMs, $detail.panDoneMs)
}
Write-Output ("  exit={0} wrote {1}" -f $process.ExitCode, $OutJson)

if ($process.ExitCode -ne 0) { exit 1 }
exit 0
