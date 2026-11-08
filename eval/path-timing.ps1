# 四条用户可见路径的端到端耗时测量（C 档预算前读数）。
#
# 口径：
#   - 统一使用 release 构建；默认 3 次运行，逐次记录、取中位数/p95（3 次时 p95=最大值）。
#   - 时间戳由外部进程用 Stopwatch 记录 `phase:` 行到达时间；生产代码只在
#     `STRATA_PHASE_MARKERS=1` 时打印标记，不在视图源码里读墙钟。
#   - `index` 只测 arch-cli 进程墙钟（纯 core，不含 UI）。
#   - `map`/`timeline`/`diff` 测 arch-view 进程墙钟与内部阶段；`--auto-close-ms` 只负责收尾，
#     不计入「到首帧/到内容」的读数。
#   - `timeline` 默认使用 617 个 first-parent 提交的真实克隆 `$env:TEMP\tsx-hist`；
#     `tests/corpus/excalidraw` 是无 `.git` 的归档副本，不能用于历史路径。
#   - `diff` 使用 `-DiffRepo` 指向一个真实 git 仓库；`-PrepareDiffRepo` 会用
#     `tests/corpus/excalidraw` 复制并建两个提交（base=完整语料，head=新增 perf-probe.ts）。
#
# 用法：
#   pwsh -File eval/path-timing.ps1 -Path index
#   pwsh -File eval/path-timing.ps1 -Path map -Corpus tests/corpus/excalidraw
#   pwsh -File eval/path-timing.ps1 -Path timeline
#   pwsh -File eval/path-timing.ps1 -Path diff -PrepareDiffRepo

[CmdletBinding()]
param(
    [ValidateSet("index", "map", "timeline", "diff")]
    [string]$Path = "map",
    [string]$CliExe = "target/release/arch-cli.exe",
    [string]$ViewExe = "target/release/arch-view.exe",
    [string]$Corpus = "tests/corpus/excalidraw",
    [string]$HistoryRepo = "$env:TEMP\tsx-hist",
    [string]$DiffRepo = "$env:TEMP\excalidraw-diff",
    [string]$From = "HEAD~1",
    [string]$To = "HEAD",
    [int]$Runs = 3,
    [int]$AutoCloseMs = 0,
    [string]$OutJson = "",
    [switch]$PrepareDiffRepo,
    [switch]$EnforceBudget
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

function Resolve-Exe([string]$value) {
    $resolved = Resolve-Path $value
    return $resolved.Path
}

$cliPath = Resolve-Exe $CliExe
$viewPath = Resolve-Exe $ViewExe

function Prepare-DiffRepo {
    param([string]$Source, [string]$Target)
    if (Test-Path (Join-Path $Target ".git")) {
        return
    }
    if (-not (Test-Path $Source)) {
        throw "缺少语料 $Source；先运行 eval/fetch-corpus.ps1"
    }
    if (Test-Path $Target) {
        Remove-Item -LiteralPath $Target -Recurse -Force
    }
    New-Item -ItemType Directory -Force -Path $Target | Out-Null
    Copy-Item -Path (Join-Path $Source "*") -Destination $Target -Recurse -Force
    git -C $Target init -q
    git -C $Target config user.email "strata-eval@example.invalid"
    git -C $Target config user.name "strata-eval"
    git -C $Target add -A
    git -C $Target commit -q -m "base: excalidraw corpus"
    Set-Content -LiteralPath (Join-Path $Target "perf-probe.ts") -Encoding utf8 -Value "export const perfProbe = 1;`n"
    git -C $Target add perf-probe.ts
    git -C $Target commit -q -m "head: add perf-probe.ts"
    Write-Output "prepared diff repo: $Target"
}

if ($Path -eq "diff" -and $PrepareDiffRepo) {
    Prepare-DiffRepo -Source $Corpus -Target $DiffRepo
}

if (-not $OutJson) {
    $OutJson = "out/path-timing-$Path.json"
}
$outDir = Split-Path -Parent $OutJson
if ($outDir -and -not (Test-Path $outDir)) {
    New-Item -ItemType Directory -Path $outDir | Out-Null
}

# 每个路径的派生指标：Start/End 为 phase 标记；空 Start 表示从进程启动（0ms）算起。
$derivedByPath = @{
    index = @(
        @{ Name = "process wall"; Start = ""; End = "process:exit" }
    )
    map = @(
        @{ Name = "prep total"; Start = "map:prep:start"; End = "map:prep:done" },
        @{ Name = "  layers"; Start = "map:prep:start"; End = "map:prep:layers" },
        @{ Name = "  callgraph"; Start = "map:prep:layers"; End = "map:prep:callgraph" },
        @{ Name = "  layout"; Start = "map:prep:callgraph"; End = "map:prep:layout" },
        @{ Name = "  reveal init"; Start = "map:prep:layout"; End = "map:prep:done" },
        @{ Name = "first visible (loading)"; Start = "main:start"; End = "frame:presented" },
        @{ Name = "app init"; Start = "app:created"; End = "window:created" },
        @{ Name = "window->first frame"; Start = "window:created"; End = "frame:presented" },
        @{ Name = "ready->content presented"; Start = "map:prep:done"; End = "content:presented" },
        @{ Name = "total to content presented"; Start = "main:start"; End = "content:presented" },
        @{ Name = "process wall"; Start = ""; End = "process:exit" }
    )
    timeline = @(
        @{ Name = "first frame (loading)"; Start = "main:start"; End = "frame:presented" },
        @{ Name = "start->load:start"; Start = "main:start"; End = "load:start" },
        @{ Name = "  git log"; Start = "load:start"; End = "load:stage:git-log" },
        @{ Name = "  sampling history"; Start = "load:stage:git-log"; End = "load:stage:history" },
        @{ Name = "  layers"; Start = "load:stage:history"; End = "load:stage:layers" },
        @{ Name = "  layout"; Start = "load:stage:layers"; End = "load:stage:layout" },
        @{ Name = "ready->content presented"; Start = "load:done"; End = "content:presented" },
        @{ Name = "total to content presented"; Start = "main:start"; End = "content:presented" },
        @{ Name = "process wall"; Start = ""; End = "process:exit" }
    )
    diff = @(
        @{ Name = "first visible (loading)"; Start = "main:start"; End = "frame:presented" },
        @{ Name = "prep total (diff_commits)"; Start = "diff:prep:start"; End = "diff:prep:done" },
        @{ Name = "app init"; Start = "app:created"; End = "window:created" },
        @{ Name = "window->first frame"; Start = "window:created"; End = "frame:presented" },
        @{ Name = "ready->content presented"; Start = "diff:prep:done"; End = "content:presented" },
        @{ Name = "total to content presented"; Start = "main:start"; End = "content:presented" },
        @{ Name = "process wall"; Start = ""; End = "process:exit" }
    )
}
$derived = $derivedByPath[$Path]

$commandText = switch ($Path) {
    "index" { "$CliExe index $Corpus" }
    "map" { "$ViewExe map $Corpus --auto-close-ms $AutoCloseMs" }
    "timeline" { "$ViewExe timeline $HistoryRepo --auto-close-ms $AutoCloseMs" }
    "diff" { "$ViewExe diff $DiffRepo --from $From --to $To --auto-close-ms $AutoCloseMs" }
}

function Invoke-IndexRun([int]$Index) {
    $watch = [System.Diagnostics.Stopwatch]::new()
    $watch.Restart()
    $psi = [System.Diagnostics.ProcessStartInfo]::new()
    $psi.FileName = $cliPath
    $psi.UseShellExecute = $false
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    foreach ($argument in @("index", $Corpus, "--out", "out/path-timing-index-artifact.json")) {
        $psi.ArgumentList.Add($argument)
    }
    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $psi
    $process.Start() | Out-Null
    $process.StandardOutput.ReadToEnd() | Out-Null
    $stderr = $process.StandardError.ReadToEnd()
    $process.WaitForExit()
    $watch.Stop()
    if ($process.ExitCode -ne 0) {
        throw "index 第 $Index 次运行退出码 $($process.ExitCode): $stderr"
    }
    return [ordered]@{ "process:exit" = $watch.ElapsedMilliseconds }
}

function Invoke-ViewRun([int]$Index) {
    $psi = [System.Diagnostics.ProcessStartInfo]::new()
    $psi.FileName = $viewPath
    $psi.UseShellExecute = $false
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.Environment["STRATA_PHASE_MARKERS"] = "1"
    $arguments = switch ($Path) {
        "map" { @("map", $Corpus, "--auto-close-ms", "$AutoCloseMs") }
        "timeline" { @("timeline", $HistoryRepo, "--auto-close-ms", "$AutoCloseMs") }
        "diff" { @("diff", $DiffRepo, "--from", $From, "--to", $To, "--auto-close-ms", "$AutoCloseMs") }
    }
    foreach ($argument in $arguments) {
        $psi.ArgumentList.Add($argument)
    }

    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $psi
    $marks = [ordered]@{}
    $watch = [System.Diagnostics.Stopwatch]::new()
    $watch.Restart()
    $process.Start() | Out-Null
    $line = $process.StandardOutput.ReadLine()
    while ($null -ne $line) {
        if ($line.StartsWith("phase: ")) {
            $label = $line.Substring(7).Trim()
            if (-not $marks.Contains($label)) {
                $marks[$label] = $watch.ElapsedMilliseconds
            }
        }
        $line = $process.StandardOutput.ReadLine()
    }
    $stderr = $process.StandardError.ReadToEnd()
    $process.WaitForExit()
    $watch.Stop()
    if ($process.ExitCode -ne 0) {
        throw "第 $Index 次运行退出码 $($process.ExitCode): $stderr"
    }
    $marks["process:exit"] = $watch.ElapsedMilliseconds
    return [pscustomobject]@{ Marks = $marks }
}

function Median([long[]]$values) {
    if ($values.Count -eq 0) { return $null }
    $sorted = @($values | Sort-Object)
    $middle = [int][math]::Floor($sorted.Count / 2)
    if ($sorted.Count % 2 -eq 1) { return [long]$sorted[$middle] }
    return [long](($sorted[$middle - 1] + $sorted[$middle]) / 2)
}

function P95([long[]]$values) {
    if ($values.Count -eq 0) { return $null }
    $sorted = @($values | Sort-Object)
    $index = [int][math]::Ceiling(0.95 * $sorted.Count) - 1
    if ($index -lt 0) { $index = 0 }
    return [long]$sorted[$index]
}

$records = @()
for ($run = 1; $run -le $Runs; $run++) {
    $record = if ($Path -eq "index") { Invoke-IndexRun $run } else { (Invoke-ViewRun $run).Marks }
    $records += [pscustomobject]@{ Run = $run; Marks = $record }
    Write-Output ""
    Write-Output ("run {0}/{1} path={2}" -f $run, $Runs, $Path)
    if ($Path -eq "index") {
        Write-Output ("  {0,-26} {1,8}ms" -f "process wall", $record["process:exit"])
    } else {
        foreach ($item in $derived) {
            if ([string]::IsNullOrEmpty($item.Start)) { continue }
            if ($record.Contains($item.Start) -and $record.Contains($item.End)) {
                $value = [long]$record[$item.End] - [long]$record[$item.Start]
                Write-Output ("  {0,-26} {1,8}ms" -f $item.Name, $value)
            }
        }
        Write-Output ("  {0,-26} {1,8}ms" -f "process wall", $record["process:exit"])
    }
    if ($run -lt $Runs) {
        Start-Sleep -Milliseconds 800
    }
}

Write-Output ""
Write-Output ("summary path={0} runs={1} command={2}" -f $Path, $Runs, $commandText)
Write-Output ("{0,-26} {1,8} {2,8} {3,8} {4,8}" -f "metric", "median", "min", "max", "p95")
$summary = [ordered]@{}
foreach ($item in $derived) {
    $values = @()
    foreach ($record in $records) {
        if ([string]::IsNullOrEmpty($item.Start)) {
            if ($record.Marks.Contains($item.End)) {
                $values += [long]$record.Marks[$item.End]
            }
        } elseif ($record.Marks.Contains($item.Start) -and $record.Marks.Contains($item.End)) {
            $values += ([long]$record.Marks[$item.End] - [long]$record.Marks[$item.Start])
        }
    }
    if ($values.Count -eq 0) { continue }
    $median = Median $values
    $min = ($values | Measure-Object -Minimum).Minimum
    $max = ($values | Measure-Object -Maximum).Maximum
    $p95 = P95 $values
    Write-Output ("{0,-26} {1,8} {2,8} {3,8} {4,8}" -f $item.Name, $median, $min, $max, $p95)
    $summary[$item.Name] = [pscustomobject]@{
        median = $median
        min = $min
        max = $max
        p95 = $p95
        values = $values
    }
}

$budgetByPath = @{
    index = @(
        @{ Metric = "process wall"; Median = 500; P95 = 800 }
    )
    map = @(
        @{ Metric = "first visible (loading)"; Median = 1200; P95 = 2000 }
        @{ Metric = "total to content presented"; Median = 3000; P95 = 4000 }
    )
    timeline = @(
        @{ Metric = "first frame (loading)"; Median = 1500; P95 = 2500 }
        @{ Metric = "total to content presented"; Median = 3000; P95 = 4000 }
    )
    diff = @(
        @{ Metric = "first visible (loading)"; Median = 1500; P95 = 2500 }
        @{ Metric = "total to content presented"; Median = 12000; P95 = 16000 }
    )
}
$budget = $budgetByPath[$Path]
$violations = @()
foreach ($item in $budget) {
    $reading = $summary[$item.Metric]
    if ($null -eq $reading) {
        $violations += "缺少指标: $($item.Metric)"
        continue
    }
    if ($reading.median -gt $item.Median) {
        $violations += ("{0}: median {1}ms > {2}ms" -f $item.Metric, $reading.median, $item.Median)
    }
    if ($reading.p95 -gt $item.P95) {
        $violations += ("{0}: p95 {1}ms > {2}ms" -f $item.Metric, $reading.p95, $item.P95)
    }
}
$budgetPassed = $violations.Count -eq 0
if ($EnforceBudget) {
    if ($budgetPassed) {
        Write-Output "budget: passed (path=$Path)"
    } else {
        Write-Output "budget: FAILED"
        foreach ($violation in $violations) {
            Write-Output "  $violation"
        }
    }
}

$hardware = [pscustomobject]@{
    cpu = (Get-CimInstance Win32_Processor | Select-Object -First 1).Name
    logicalProcessors = [Environment]::ProcessorCount
    os = [Environment]::OSVersion.VersionString
    powershell = $PSVersionTable.PSVersion.ToString()
}
$payload = [pscustomobject]@{
    schemaVersion = 1
    generatedAt = (Get-Date).ToString("o")
    profile = "release"
    path = $Path
    command = $commandText
    runs = $Runs
    autoCloseMs = $AutoCloseMs
    corpus = $Corpus
    historyRepo = $(if ($Path -eq "timeline") { $HistoryRepo } else { $null })
    diffRepo = $(if ($Path -eq "diff") { $DiffRepo } else { $null })
    hardware = $hardware
    budget = $budget
    budgetPassed = $budgetPassed
    budgetViolations = $violations
    summary = $summary
    records = $records
}
$payload | ConvertTo-Json -Depth 8 | Set-Content -Path $OutJson -Encoding utf8
Write-Output ""
Write-Output "wrote $OutJson"

if ($EnforceBudget -and -not $budgetPassed) {
    exit 1
}
