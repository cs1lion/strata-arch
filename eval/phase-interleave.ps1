param(
    [string]$TimelineExe = "target/release/arch-view.exe",
    [string]$EmptyExe = "target/release/examples/empty_window.exe",
    [string]$Repo = "$env:TEMP\tsx-hist",
    [int]$Pairs = 9,
    [int]$AutoCloseMs = 500,
    [switch]$NoSnapshot
)

$timelinePath = (Resolve-Path $TimelineExe).Path
$emptyPath = (Resolve-Path $EmptyExe).Path
if (-not (Test-Path $Repo)) { throw "样本仓库不存在: $Repo" }

$labels = @(
    "main:start",
    "prep:git-log",
    "prep:history",
    "prep:layers",
    "prep:layout",
    "app:created",
    "app:entry",
    "window:callback",
    "window:created",
    "window:activated",
    "frame:render",
    "frame:presented",
    "quit:auto",
    "main:end"
)

function Get-EnvironmentSnapshot {
    $now = (Get-Date).ToString("yyyy-MM-ddTHH:mm:ss.fff")
    $arch = @(Get-Process -Name arch-view -ErrorAction SilentlyContinue).Count
    $cargo = @(Get-Process -Name cargo -ErrorAction SilentlyContinue).Count
    $rustc = @(Get-Process -Name rustc -ErrorAction SilentlyContinue).Count
    $top = (Get-Process |
        Sort-Object CPU -Descending |
        Select-Object -First 5 |
        ForEach-Object { "$($_.ProcessName)#$($_.Id)=$([math]::Round($_.CPU, 1))s" }) -join ", "
    $load = "unavailable"
    try {
        $load = [math]::Round(((Get-CimInstance Win32_Processor -ErrorAction Stop |
                Measure-Object -Property LoadPercentage -Average).Average), 0)
    } catch {
        $load = "unavailable ($($_.Exception.Message))"
    }
    $gpu = "unavailable"
    try {
        $adapters = @(Get-CimInstance Win32_VideoController -ErrorAction Stop)
        if ($adapters.Count -eq 0) {
            $gpu = "none reported"
        } else {
            $gpu = ($adapters | ForEach-Object {
                    "$($_.Name) | driver=$($_.DriverVersion) | status=$($_.Status)"
                }) -join " ;; "
        }
    } catch {
        $gpu = "unavailable ($($_.Exception.Message))"
    }
    return "snapshot time=$now cpu_load=$load arch-view=$arch cargo=$cargo rustc=$rustc top=[$top] gpu=[$gpu]"
}

function Invoke-Run {
    param(
        [string]$Kind,
        [string]$ExePath,
        [string[]]$RunArgs,
        [int]$Index,
        [bool]$Snapshot
    )

    $snapshotText = $null
    if ($Snapshot) {
        $snapshotText = Get-EnvironmentSnapshot
        [Console]::WriteLine($snapshotText)
    }
    $startedAt = (Get-Date).ToString("yyyy-MM-ddTHH:mm:ss.fff")

    $psi = [System.Diagnostics.ProcessStartInfo]::new()
    $psi.FileName = $ExePath
    $psi.UseShellExecute = $false
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.Environment["STRATA_PHASE_MARKERS"] = "1"
    foreach ($argument in $RunArgs) {
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
        } elseif ($line.StartsWith("timeline: ")) {
            $marks["prep:reported"] = $watch.ElapsedMilliseconds
        }
        $line = $process.StandardOutput.ReadLine()
    }
    $stderr = $process.StandardError.ReadToEnd()
    $process.WaitForExit()
    $exitCode = $process.ExitCode
    $exitMs = $watch.ElapsedMilliseconds
    if ($exitCode -ne 0) {
        throw "pair $Index $Kind 退出码 $exitCode`: $stderr"
    }

    $absolute = [ordered]@{}
    $previous = 0L
    foreach ($label in $labels) {
        if ($marks.Contains($label)) {
            $value = [long]$marks[$label]
            $absolute[$label] = $value
            [Console]::WriteLine(("{0,-8} pair {1,-2} {2,-16} abs={3,6}ms +{4,5}ms" -f $Kind, $Index, $label, $value, ($value - $previous)))
            $previous = $value
        }
    }
    [Console]::WriteLine(("{0,-8} pair {1,-2} {2,-16} abs={3,6}ms +{4,5}ms" -f $Kind, $Index, "process:exit", $exitMs, ($exitMs - $previous)))
    $absolute["process:exit"] = $exitMs

    return [pscustomobject]@{
        Kind = $Kind
        Index = $Index
        Marks = $absolute
        Exit = $exitMs
        StartedAt = $startedAt
        Snapshot = $snapshotText
    }
}

function Median([long[]]$values) {
    $sorted = @($values | Sort-Object)
    $middle = [int][math]::Floor($sorted.Count / 2)
    if ($sorted.Count % 2 -eq 1) { return $sorted[$middle] }
    return [long](($sorted[$middle - 1] + $sorted[$middle]) / 2)
}

function Correlation([double[]]$X, [double[]]$Y) {
    $n = $X.Count
    if ($n -lt 2 -or $n -ne $Y.Count) { return $null }
    $meanX = ($X | Measure-Object -Average).Average
    $meanY = ($Y | Measure-Object -Average).Average
    $numerator = 0.0
    $denominatorX = 0.0
    $denominatorY = 0.0
    for ($i = 0; $i -lt $n; $i++) {
        $dx = $X[$i] - $meanX
        $dy = $Y[$i] - $meanY
        $numerator += $dx * $dy
        $denominatorX += $dx * $dx
        $denominatorY += $dy * $dy
    }
    if ($denominatorX -eq 0 -or $denominatorY -eq 0) { return $null }
    return $numerator / [math]::Sqrt($denominatorX * $denominatorY)
}

function Show-Summary {
    param([string]$Kind, $Records)

    $selected = @($Records | Where-Object { $_.Kind -eq $Kind })
    [Console]::WriteLine("")
    [Console]::WriteLine("summary kind=$Kind runs=$($selected.Count)")
    if ($selected.Count -eq 0) { return }

    [Console]::WriteLine(("{0,-16} {1,8} {2,8} {3,8}" -f "phase", "median", "min", "max"))
    foreach ($label in ($labels + "process:exit")) {
        $values = @($selected | Where-Object { $_.Marks.Contains($label) } | ForEach-Object { [long]$_.Marks[$label] })
        if ($values.Count -eq 0) { continue }
        $median = Median $values
        $min = ($values | Measure-Object -Minimum).Minimum
        $max = ($values | Measure-Object -Maximum).Maximum
        [Console]::WriteLine(("{0,-16} {1,8} {2,8} {3,8}" -f $label, $median, $min, $max))
    }

    $derived = @(
        @{ Name = "prep total"; Start = "main:start"; End = "prep:layout" },
        @{ Name = "app-created->window-callback"; Start = "app:created"; End = "window:callback" },
        @{ Name = "window-callback->created"; Start = "window:callback"; End = "window:created" },
        @{ Name = "window-created->activated"; Start = "window:created"; End = "window:activated" },
        @{ Name = "activated->first-presented"; Start = "window:activated"; End = "frame:presented" },
        @{ Name = "start->first-presented"; Start = "main:start"; End = "frame:presented" },
        @{ Name = "process->first-presented"; Start = ""; End = "frame:presented" },
        @{ Name = "quit->exit"; Start = "quit:auto"; End = "process:exit" },
        @{ Name = "first-presented->exit"; Start = "frame:presented"; End = "process:exit" }
    )
    [Console]::WriteLine("")
    [Console]::WriteLine(("{0,-30} {1,8} {2,8} {3,8}" -f "derived", "median", "min", "max"))
    foreach ($item in $derived) {
        $values = @()
        foreach ($record in $selected) {
            if ($item.Start -eq "") {
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
        [Console]::WriteLine(("{0,-30} {1,8} {2,8} {3,8}" -f $item.Name, $median, $min, $max))
    }
}

$records = @()
$totalRuns = $Pairs * 2
$runNumber = 0
for ($pair = 1; $pair -le $Pairs; $pair++) {
    # Alternate the order inside each pair so neither build always runs first.
    $order = if ($pair % 2 -eq 1) { @("timeline", "empty") } else { @("empty", "timeline") }
    foreach ($kind in $order) {
        $runNumber++
        [Console]::WriteLine("")
        [Console]::WriteLine("pair=$pair order=$($order -join ',') kind=$kind run=$runNumber/$totalRuns")
        if ($kind -eq "timeline") {
            $record = Invoke-Run -Kind "timeline" -ExePath $timelinePath -RunArgs @("timeline", $Repo, "--auto-close-ms", "$AutoCloseMs") -Index $pair -Snapshot (-not $NoSnapshot)
        } else {
            $record = Invoke-Run -Kind "empty" -ExePath $emptyPath -RunArgs @("empty", $Repo, "--auto-close-ms", "$AutoCloseMs") -Index $pair -Snapshot (-not $NoSnapshot)
        }
        $records += $record
        if ($runNumber -lt $totalRuns) {
            Start-Sleep -Milliseconds 800
        }
    }
}

Show-Summary -Kind "timeline" -Records $records
Show-Summary -Kind "empty" -Records $records

[Console]::WriteLine("")
[Console]::WriteLine("pair table (ms)")
[Console]::WriteLine(("{0,4} {1,16} {2,16} {3,10} {4,18} {5,18} {6,12}" -f "pair", "timeline_first", "empty_first", "first_diff", "timeline_window", "empty_window", "window_diff"))
$firstTimeline = @()
$firstEmpty = @()
$windowTimeline = @()
$windowEmpty = @()
for ($pair = 1; $pair -le $Pairs; $pair++) {
    $timelineRecord = $records | Where-Object { $_.Kind -eq "timeline" -and $_.Index -eq $pair } | Select-Object -First 1
    $emptyRecord = $records | Where-Object { $_.Kind -eq "empty" -and $_.Index -eq $pair } | Select-Object -First 1
    if ($null -eq $timelineRecord -or $null -eq $emptyRecord) { continue }

    $timelineFirst = $null
    $emptyFirst = $null
    $timelineWindow = $null
    $emptyWindow = $null
    if ($timelineRecord.Marks.Contains("frame:presented")) {
        $timelineFirst = [long]$timelineRecord.Marks["frame:presented"]
    }
    if ($emptyRecord.Marks.Contains("frame:presented")) {
        $emptyFirst = [long]$emptyRecord.Marks["frame:presented"]
    }
    if ($timelineRecord.Marks.Contains("app:created") -and $timelineRecord.Marks.Contains("window:callback")) {
        $timelineWindow = [long]$timelineRecord.Marks["window:callback"] - [long]$timelineRecord.Marks["app:created"]
    }
    if ($emptyRecord.Marks.Contains("app:created") -and $emptyRecord.Marks.Contains("window:callback")) {
        $emptyWindow = [long]$emptyRecord.Marks["window:callback"] - [long]$emptyRecord.Marks["app:created"]
    }

    $firstDiff = if ($null -ne $timelineFirst -and $null -ne $emptyFirst) { $timelineFirst - $emptyFirst } else { $null }
    $windowDiff = if ($null -ne $timelineWindow -and $null -ne $emptyWindow) { $timelineWindow - $emptyWindow } else { $null }
    [Console]::WriteLine(("{0,4} {1,16} {2,16} {3,10} {4,18} {5,18} {6,12}" -f $pair, $timelineFirst, $emptyFirst, $firstDiff, $timelineWindow, $emptyWindow, $windowDiff))

    if ($null -ne $timelineFirst -and $null -ne $emptyFirst) {
        $firstTimeline += [double]$timelineFirst
        $firstEmpty += [double]$emptyFirst
    }
    if ($null -ne $timelineWindow -and $null -ne $emptyWindow) {
        $windowTimeline += [double]$timelineWindow
        $windowEmpty += [double]$emptyWindow
    }
}

$firstCorrelation = Correlation $firstTimeline $firstEmpty
$windowCorrelation = Correlation $windowTimeline $windowEmpty
[Console]::WriteLine("")
[Console]::WriteLine("correlation first-frame r=$firstCorrelation (n=$($firstTimeline.Count))")
[Console]::WriteLine("correlation window-segment r=$windowCorrelation (n=$($windowTimeline.Count))")
if ($firstTimeline.Count -gt 0) {
    [Console]::WriteLine("first-frame mean timeline=$([math]::Round(($firstTimeline | Measure-Object -Average).Average, 0)) empty=$([math]::Round(($firstEmpty | Measure-Object -Average).Average, 0))")
}
if ($windowTimeline.Count -gt 0) {
    [Console]::WriteLine("window mean timeline=$([math]::Round(($windowTimeline | Measure-Object -Average).Average, 0)) empty=$([math]::Round(($windowEmpty | Measure-Object -Average).Average, 0))")
}
