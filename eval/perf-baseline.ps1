# ticket 10：固定样本的性能与内存基线。
#
# 口径：
#   - 只测 release 构建的 arch-cli；默认 3 次运行，记录每次墙钟与峰值 WorkingSet64。
#   - 内存门禁用 OS 生命周期峰值 `PeakWorkingSet64`（`osPeakWsMb`），代表进程驻留集（RSS）峰值；
#     轮询 `WorkingSet64`（`polledWsMb`）只是采样下界，可能漏掉短峰，不用于门禁。
#   - 门禁：`index` 与 `history` 每次运行的峰值必须 < `-MaxPeakMb`（默认 512MB），超限退出 1。
#   - 结果写入 out/perf.json（不入库）；仓内只更新 eval/baseline/current.json 摘要。
#
# 用法：
#   pwsh -File eval/perf-baseline.ps1
#   pwsh -File eval/perf-baseline.ps1 -Runs 3 -Exe target/release/arch-cli.exe
#   pwsh -File eval/perf-baseline.ps1 -MaxPeakMb 1   # 反向用例：门禁必须判红

[CmdletBinding()]
param(
    [string]$Exe = "target/release/arch-cli.exe",
    [string]$OutJson = "out/perf.json",
    [int]$Runs = 3,
    [string]$CorpusRoot = "tests/corpus",
    [string]$CorpusManifest = "eval/corpus.toml",
    [int]$MaxPeakMb = 512
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

if (-not (Test-Path $Exe)) {
    throw "缺少 $Exe；先运行 cargo build --release -p arch-cli"
}

$outDir = Split-Path -Parent $OutJson
if (-not (Test-Path $outDir)) {
    New-Item -ItemType Directory -Path $outDir | Out-Null
}

function Get-Commit([string]$Name) {
    $text = Get-Content $CorpusManifest -Raw
    $pattern = 'name = "' + [regex]::Escape($Name) + '"\s*\r?\n(?:[^\[]*?)commit = "([0-9a-f]{40})"'
    $match = [regex]::Match($text, $pattern)
    if (-not $match.Success) {
        throw "在 $CorpusManifest 中找不到 $Name 的 commit"
    }
    return $match.Groups[1].Value
}

# 轮询 WorkingSet64 得到峰值内存；进程过短时可能采不到，此时记为 null 而不是 0。
function Invoke-Sampled([string]$Label, [string[]]$Arguments) {
    $stdout = Join-Path $outDir "perf-$Label-$([guid]::NewGuid().ToString('N')).stdout.txt"
    $stderr = Join-Path $outDir "perf-$Label-$([guid]::NewGuid().ToString('N')).stderr.txt"
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $proc = Start-Process -FilePath $Exe -ArgumentList $Arguments -PassThru -NoNewWindow `
        -RedirectStandardOutput $stdout -RedirectStandardError $stderr
    $peak = $null
    while (-not $proc.HasExited) {
        try {
            $ws = $proc.WorkingSet64
            if ($null -ne $ws -and ($null -eq $peak -or $ws -gt $peak)) {
                $peak = $ws
            }
        } catch {
            # 进程退出瞬间读取会失败，忽略即可。
        }
        Start-Sleep -Milliseconds 15
    }
    $proc.WaitForExit()
    $sw.Stop()
    $exitCode = $proc.ExitCode
    $osPeak = $null
    try {
        $osPeak = $proc.PeakWorkingSet64
    } catch {
        # 某些平台/进程对象不暴露 OS 峰值，退回轮询值。
    }
    $stdoutText = Get-Content $stdout -Raw -ErrorAction SilentlyContinue
    $stderrText = Get-Content $stderr -Raw -ErrorAction SilentlyContinue
    Remove-Item $stdout, $stderr -ErrorAction SilentlyContinue

    [pscustomobject]@{
        label       = $Label
        arguments   = $Arguments
        exitCode    = $exitCode
        wallMs      = [math]::Round($sw.Elapsed.TotalMilliseconds, 1)
        polledWsMb  = if ($null -eq $peak) { $null } else { [math]::Round($peak / 1MB, 1) }
        osPeakWsMb  = if ($null -eq $osPeak) { $null } else { [math]::Round($osPeak / 1MB, 1) }
        stdoutFirst = if ($stdoutText) { ($stdoutText -split "`n")[0] } else { $null }
        stderrFirst = if ($stderrText) { ($stderrText -split "`n")[0] } else { $null }
    }
}

function Get-Median([double[]]$Values) {
    if ($Values.Count -eq 0) { return $null }
    $sorted = $Values | Sort-Object
    $mid = [math]::Floor($sorted.Count / 2)
    if ($sorted.Count % 2 -eq 1) { return $sorted[$mid] }
    return [math]::Round(($sorted[$mid - 1] + $sorted[$mid]) / 2, 1)
}

$samples = @(
    @{ name = "tsx"; path = Join-Path $CorpusRoot "tsx"; task = "index" },
    @{ name = "excalidraw"; path = Join-Path $CorpusRoot "excalidraw"; task = "index" }
)

$records = @()
$summary = @{}
$toolVersion = (Select-String -Path "Cargo.toml" -Pattern '^version = "([^"]+)"' | Select-Object -First 1).Matches.Groups[1].Value

foreach ($sample in $samples) {
    if (-not (Test-Path $sample.path)) {
        throw "缺少样本缓存 $($sample.path)；先运行 eval/fetch-corpus.ps1"
    }
    $commit = Get-Commit $sample.name
    $artifact = Join-Path $outDir "perf-$($sample.name)-index.json"
    $sampleRuns = @()
    for ($i = 1; $i -le $Runs; $i++) {
        $sampleRuns += Invoke-Sampled "$($sample.name)-index-$i" @("index", $sample.path, "--out", $artifact)
    }
    $walls = @($sampleRuns | ForEach-Object { $_.wallMs })
    $summary[$sample.name] = [pscustomobject]@{
        task    = "index"
        commit  = $commit
        runs    = $sampleRuns
        medianMs = Get-Median $walls
        maxMs   = ($walls | Measure-Object -Maximum).Maximum
        peakWsMb = ($sampleRuns | ForEach-Object { $_.osPeakWsMb } | Where-Object { $null -ne $_ } | Measure-Object -Maximum).Maximum
        polledPeakWsMb = ($sampleRuns | ForEach-Object { $_.polledWsMb } | Where-Object { $null -ne $_ } | Measure-Object -Maximum).Maximum
    }
    $records += $sampleRuns
}

$historySample = "excalidraw"
$historyPath = Join-Path $CorpusRoot $historySample
$historyCommit = Get-Commit $historySample
$historySummary = $null
if (Test-Path (Join-Path $historyPath ".git")) {
    $artifact = Join-Path $outDir "perf-$historySample-history.json"
    $sampleRuns = @()
    for ($i = 1; $i -le $Runs; $i++) {
        $sampleRuns += Invoke-Sampled "$historySample-history-$i" @(
            "history", $historyPath, "--out", $artifact, "--max-commits", "500"
        )
    }
    $walls = @($sampleRuns | ForEach-Object { $_.wallMs })
    $historySummary = [pscustomobject]@{
        task     = "history"
        commit   = $historyCommit
        runs     = $sampleRuns
        medianMs = Get-Median $walls
        maxMs    = ($walls | Measure-Object -Maximum).Maximum
        peakWsMb = ($sampleRuns | ForEach-Object { $_.osPeakWsMb } | Where-Object { $null -ne $_ } | Measure-Object -Maximum).Maximum
        polledPeakWsMb = ($sampleRuns | ForEach-Object { $_.polledWsMb } | Where-Object { $null -ne $_ } | Measure-Object -Maximum).Maximum
    }
    $records += $sampleRuns
} else {
    $historySummary = [pscustomobject]@{
        task   = "history"
        commit = $historyCommit
        status = "skipped"
        reason = "样本缓存没有 .git（归档副本），无法复现历史窗口读数；需真实克隆"
    }
}

$gateViolations = @()
$gateChecked = @{}
foreach ($name in $summary.Keys) {
    $value = $summary[$name]
    $peak = $value.peakWsMb
    $metric = "osPeakWsMb"
    if ($null -eq $peak) {
        $peak = $value.polledPeakWsMb
        $metric = "polledWsMb"
    }
    $gateChecked[$name] = [pscustomobject]@{ task = $value.task; metric = $metric; peakMb = $peak }
    if ($null -eq $peak) {
        $gateViolations += [pscustomobject]@{ sample = $name; task = $value.task; metric = "unmeasured"; peakMb = $null; limitMb = $MaxPeakMb }
    } elseif ($peak -gt $MaxPeakMb) {
        $gateViolations += [pscustomobject]@{ sample = $name; task = $value.task; metric = $metric; peakMb = $peak; limitMb = $MaxPeakMb }
    }
}
if ($historySummary.status -ne "skipped") {
    $peak = $historySummary.peakWsMb
    $metric = "osPeakWsMb"
    if ($null -eq $peak) {
        $peak = $historySummary.polledPeakWsMb
        $metric = "polledWsMb"
    }
    $gateChecked["history"] = [pscustomobject]@{ task = "history"; metric = $metric; peakMb = $peak }
    if ($null -eq $peak) {
        $gateViolations += [pscustomobject]@{ sample = "history"; task = "history"; metric = "unmeasured"; peakMb = $null; limitMb = $MaxPeakMb }
    } elseif ($peak -gt $MaxPeakMb) {
        $gateViolations += [pscustomobject]@{ sample = "history"; task = "history"; metric = $metric; peakMb = $peak; limitMb = $MaxPeakMb }
    }
}

$payload = [pscustomobject]@{
    schemaVersion = 1
    generatedAt   = (Get-Date).ToString("o")
    toolVersion   = $toolVersion
    hardware      = [pscustomobject]@{
        cpu              = (Get-CimInstance Win32_Processor | Select-Object -First 1).Name
        logicalProcessors = [Environment]::ProcessorCount
        os               = [Environment]::OSVersion.VersionString
        powershell       = $PSVersionTable.PSVersion.ToString()
    }
    runs    = $Runs
    samples = $summary
    history = $historySummary
    records = $records
    memoryGate = [pscustomobject]@{
        limitMb    = $MaxPeakMb
        metric     = "osPeakWsMb (PeakWorkingSet64; polledWsMb only as fallback)"
        checked    = $gateChecked
        passed     = ($gateViolations.Count -eq 0)
        violations = $gateViolations
    }
}

$payload | ConvertTo-Json -Depth 8 | Set-Content -Path $OutJson -Encoding utf8
Write-Output "wrote $OutJson"
$summary.GetEnumerator() | ForEach-Object {
    $value = $_.Value
    Write-Output ("{0}: median {1}ms, max {2}ms, peakWs {3}Mb" -f $_.Key, $value.medianMs, $value.maxMs, $value.peakWsMb)
}
if ($historySummary.status -eq "skipped") {
    Write-Output "history: skipped ($($historySummary.reason))"
} else {
    Write-Output ("history: median {0}ms, max {1}ms, peakWs {2}Mb" -f $historySummary.medianMs, $historySummary.maxMs, $historySummary.peakWsMb)
}

if ($gateViolations.Count -gt 0) {
    Write-Output ("memory gate: FAILED (limit {0}Mb)" -f $MaxPeakMb)
    $gateViolations | ForEach-Object {
        Write-Output ("  {0}/{1}: peak {2}Mb ({3}) > {4}Mb" -f $_.sample, $_.task, $_.peakMb, $_.metric, $_.limitMb)
    }
    exit 1
}
Write-Output ("memory gate: passed (all peaks < {0}Mb)" -f $MaxPeakMb)

