param(
    [string]$Exe = "target/release/arch-view.exe",
    [string]$Repo = "$env:TEMP\tsx-hist",
    [string]$Mode = "timeline",
    [int]$Runs = 3,
    [int]$AutoCloseMs = 3000
)

$exePath = (Resolve-Path $Exe).Path
if (-not (Test-Path $Repo)) { throw "样本仓库不存在: $Repo" }

$labels = @(
    "main:start",
    "prep:git-log",
    "prep:history",
    "prep:layers",
    "prep:layout",
    "prep:reported",
    "app:created",
    "app:entry",
    "window:callback",
    "window:created",
    "window:activated",
    "frame:render",
    "frame:presented",
    "load:start",
    "load:stage:git-log",
    "load:stage:history",
    "load:stage:layers",
    "load:stage:layout",
    "load:done",
    "load:failed",
    "content:render",
    "content:presented",
    "quit:auto",
    "main:end"
)

$records = @()
for ($run = 1; $run -le $Runs; $run++) {
    $psi = [System.Diagnostics.ProcessStartInfo]::new()
    $psi.FileName = $exePath
    $psi.UseShellExecute = $false
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.Environment["STRATA_PHASE_MARKERS"] = "1"
    foreach ($argument in @($Mode, $Repo, "--auto-close-ms", "$AutoCloseMs")) {
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
        throw "第 $run 次运行退出码 $exitCode`: $stderr"
    }

    $absolute = [ordered]@{}
    $previous = 0L
    foreach ($label in $labels) {
        if ($marks.Contains($label)) {
            $value = [long]$marks[$label]
            $absolute[$label] = $value
            Write-Output ("run {0}  {1,-16} abs={2,6}ms  +{3,5}ms" -f $run, $label, $value, ($value - $previous))
            $previous = $value
        }
    }
    Write-Output ("run {0}  {1,-16} abs={2,6}ms  +{3,5}ms" -f $run, "process:exit", $exitMs, ($exitMs - $previous))
    $absolute["process:exit"] = $exitMs
    $records += [pscustomobject]@{ Run = $run; Marks = $absolute; Exit = $exitMs }

    if ($run -lt $Runs) {
        Start-Sleep -Milliseconds 800
    }
}

function Median([long[]]$values) {
    $sorted = @($values | Sort-Object)
    $middle = [int][math]::Floor($sorted.Count / 2)
    if ($sorted.Count % 2 -eq 1) { return $sorted[$middle] }
    return [long](($sorted[$middle - 1] + $sorted[$middle]) / 2)
}

$summaryLabels = @($labels + "process:exit")
Write-Output ""
Write-Output "summary ($Runs runs, mode=$Mode, auto-close=${AutoCloseMs}ms)"
Write-Output ("{0,-16} {1,8} {2,8} {3,8}" -f "phase", "median", "min", "max")
foreach ($label in $summaryLabels) {
    $values = @($records | Where-Object { $_.Marks.Contains($label) } | ForEach-Object { [long]$_.Marks[$label] })
    if ($values.Count -eq 0) { continue }
    $median = Median $values
    $min = ($values | Measure-Object -Minimum).Minimum
    $max = ($values | Measure-Object -Maximum).Maximum
    Write-Output ("{0,-16} {1,8} {2,8} {3,8}" -f $label, $median, $min, $max)
}

$derived = @(
    @{ Name = "phase1 prep+app-created"; Start = "main:start"; End = "app:created" },
    @{ Name = "prep total"; Start = "main:start"; End = "prep:layout" },
    @{ Name = "app-created->window-callback"; Start = "app:created"; End = "window:callback" },
    @{ Name = "window-callback->created"; Start = "window:callback"; End = "window:created" },
    @{ Name = "window-created->activated"; Start = "window:created"; End = "window:activated" },
    @{ Name = "activated->first-presented"; Start = "window:activated"; End = "frame:presented" },
    @{ Name = "start->first-presented"; Start = "main:start"; End = "frame:presented" },
    @{ Name = "start->load-start"; Start = "main:start"; End = "load:start" },
    @{ Name = "window-callback->load-start"; Start = "window:callback"; End = "load:start" },
    @{ Name = "load-start->load-done"; Start = "load:start"; End = "load:done" },
    @{ Name = "load-done->content-presented"; Start = "load:done"; End = "content:presented" },
    @{ Name = "content-render->content-presented"; Start = "content:render"; End = "content:presented" },
    @{ Name = "start->content-presented"; Start = "main:start"; End = "content:presented" },
    @{ Name = "process->content-presented"; Start = "process:exit"; End = "content:presented" },
    @{ Name = "quit->main-end"; Start = "quit:auto"; End = "main:end" },
    @{ Name = "quit->exit"; Start = "quit:auto"; End = "process:exit" },
    @{ Name = "first-presented->exit"; Start = "frame:presented"; End = "process:exit" }
)
Write-Output ""
Write-Output ("{0,-26} {1,8} {2,8} {3,8}" -f "derived", "median", "min", "max")
foreach ($item in $derived) {
    $values = @()
    foreach ($record in $records) {
        $run = $record
        if ($run.Marks.Contains($item.Start) -and $run.Marks.Contains($item.End)) {
            $start = [long]$run.Marks[$item.Start]
            $end = [long]$run.Marks[$item.End]
            if ($item.Name -eq "process->first-presented") {
                $start = 0L
            }
            $values += ($end - $start)
        }
    }
    if ($values.Count -eq 0) { continue }
    $median = Median $values
    $min = ($values | Measure-Object -Minimum).Minimum
    $max = ($values | Measure-Object -Maximum).Maximum
    Write-Output ("{0,-26} {1,8} {2,8} {3,8}" -f $item.Name, $median, $min, $max)
}
