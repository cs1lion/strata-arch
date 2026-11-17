# A/B 降格对照的判据自测（票 24 阶段 2 · 把配对规则与判据固化成入仓可重放脚本）
#
# 为什么要有这个文件：2026-11-17 首轮判据评估是在一次性临时脚本里做的，配对规则
# 没有入仓 ⇒ 读数不可复核、不可重放。本文件把两件事变成可审的仓库资产：
#   (1) **按段名配对**（不是按相邻下标）；
#   (2) P1/P2/P3/N1/S1/S3/OB 七例判据，判据实现直接复用登记的
#       `eval/lib-paired-stats.ps1::Get-PairedAnalysis`，本脚本**不自定阈值**。
#
# 与 `eval/frame-timing.ps1` 的分工（组合关系，不是重复）：
#   - 能力凭证（每个降格段必须有 `ab:skip=` 打点）由 frame-timing 断言，缺失即 exit 1；
#     本脚本调用它并把它非零退出**当作失败**，因此"段名换了但配置没换"这类缺陷在这里
#     一定传不成 PASS。
#   - 本脚本只负责配对与显著性判据。用 `-PositiveJson/-NegativeJson` 复用既有产物时，
#     凭证那一环已由当初那次运行把关，本脚本会明确提示"未重新校验凭证"。
#
# 三例方向预设（跑前定死，跑后不得回改）：
#   P1 p95 侧必须显著且变体更快（gap = base − variant > 0）
#   P2 median 侧符号**只记录不判定**——降格在 median 侧可正可负（票 24 修订段实测过反向），
#      把它当失败判据会把"效应本身双向"误读成"检验力不足"
#   P3 base/variant 互换后，两侧 mean 必须反号
#   N1 同配置对照（BA↔BA）两侧都必须不显著（假阳性检验）
#   S1 段实例须覆盖 cycle 且正/负对照的段名集合符合预期
#   S3 尾段丢弃与未配对实例都必须显式报告，且未配对实例 ≤ 1
#   OB 顺序偏置自检：正对照必须**非空转**（baseFirst 与 variantFirst 两组都非空）且未判不可信
#
# 退出码：0 = 七例全符合；1 = 任一例不符合（判据不成立或运行本身失败）；
#         2 = 前置条件不满足（缺 release 二进制 / 缺 frame-timing / 缺产物），**不表示报警**。
#
# 已知边界：需要可见 Windows 桌面会话与 release 构建 ⇒ 人工项，不进 CI；
#           读数与本机 GPU/字体/负载相关，不得跨机比较，也不得与历史批次直接混算。

[CmdletBinding()]
param(
    # 每个段批次的帧数。太小会让单批 median 噪声压过效应；太大拉长运行时间。
    [int]$SegmentFrames = 20,
    # cycle 模式 (BA, skip, skip, BA) 的重复次数；6 次给出 24 个段实例（12 对，双向各 6）。
    [int]$Cycles = 6,
    # 判据参数：与 eval/paired-ab.ps1 的默认值保持一致，不在本脚本里另起一套。
    [double]$K = 2,
    [double]$SigMinRatio = 0.8334,
    [string]$Exe = 'target/release/arch-view.exe',
    [string]$OutDir = '',
    # 复用已有产物时给定（跳过运行）；两者必须同时提供。
    [string]$PositiveJson = '',
    [string]$NegativeJson = ''
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

. (Join-Path $PSScriptRoot 'lib-paired-stats.ps1')

$timing = Join-Path $PSScriptRoot 'frame-timing.ps1'
$suffix = if ($IsWindows) { '.exe' } else { '' }
$exePath = $Exe
if (-not $exePath.EndsWith($suffix)) { $exePath = "$exePath$suffix" }

$reuse = ($PositiveJson -ne '' -and $NegativeJson -ne '')
if (-not (Test-Path -LiteralPath (Join-Path $root 'eval/lib-paired-stats.ps1'))) {
    Write-Output "ab-skip-selftest: PRECHECK FAIL 缺 eval/lib-paired-stats.ps1"
    exit 2
}
if (-not (Test-Path -LiteralPath $timing)) {
    Write-Output "ab-skip-selftest: PRECHECK FAIL 缺 frame-timing 脚本"
    exit 2
}
if (-not $reuse -and -not (Test-Path -LiteralPath $exePath)) {
    Write-Output ("ab-skip-selftest: PRECHECK FAIL 缺 {0}（先 cargo build --release -p arch-view）；退出码 2 不表示报警" -f $exePath)
    exit 2
}
if ($SegmentFrames -lt 10) { Write-Output "ab-skip-selftest: PRECHECK FAIL -SegmentFrames 不得小于 10（单批样本过少）"; exit 2 }
if ($Cycles -lt 3) { Write-Output "ab-skip-selftest: PRECHECK FAIL -Cycles 不得小于 3（配对样本过少不足以做符号检验）"; exit 2 }

$runId = Get-Date -Format 'yyyyMMdd-HHmmss'
if ($OutDir -eq '') { $OutDir = Join-Path $root ("out/ab-skip-selftest-" + $runId) }
if (-not (Test-Path -LiteralPath $OutDir)) { New-Item -ItemType Directory -Force -Path $OutDir | Out-Null }

$script:cases = @()
function Add-Case {
    param([string]$Id, [string]$Expect, [bool]$Ok, [string]$Detail)
    $mark = if ($Ok) { 'PASS' } else { 'FAIL' }
    $script:cases += [pscustomobject]@{ id = $Id; ok = $Ok; detail = $Detail }
    Write-Output ("  [{0}] {1,-3} 期望: {2,-46} 实得: {3}" -f $mark, $Id, $Expect, $Detail)
}

function Invoke-Timing {
    param([string]$Label, [string]$Cycle, [string]$JsonPath)
    # 完整消费管道：用 Select-Object -First 截断会提前终止子进程并污染 $LASTEXITCODE。
    $out = (& pwsh -NoProfile -File $timing -Mode ab -AbCycle $Cycle `
            -AbSegmentFrames $SegmentFrames -Exe $Exe -OutJson $JsonPath 2>&1 | Out-String)
    return [pscustomobject]@{ code = $LASTEXITCODE; out = $out }
}

function Get-SegmentRows {
    param([string]$JsonPath)
    $a = Get-Content -LiteralPath $JsonPath -Raw | ConvertFrom-Json
    return [pscustomobject]@{ rows = @($a.abSegments); panFrames = [int]$a.panFrames }
}

# 按段名配对：base 恒为参照段（`BA`），variant 为降格段（`skip:*`）；
# baseFirst 记录该对比中 base 是否在时间上先出现——这是顺序偏置自检的分组依据。
# 同名相邻（BA↔BA 或 skip↔skip）不配成对照对，计入 unpaired 并显式报告。
function Build-ContrastPairs {
    param([object[]]$Rows)
    $pairs = @()
    $used = New-Object bool[] $Rows.Count
    for ($i = 0; $i -lt $Rows.Count - 1; $i += 1) {
        if ($used[$i]) { continue }
        $x = $Rows[$i]; $y = $Rows[$i + 1]
        if ($x.segment -eq $y.segment) { continue }
        $base = $x; $variant = $y; $baseFirst = $true
        if ($x.segment -like 'skip:*') { $base = $y; $variant = $x; $baseFirst = $false }
        $pairs += [pscustomobject]@{
            baseMedianMs    = [double]$base.stats.median
            variantMedianMs = [double]$variant.stats.median
            baseP95Ms       = [double]$base.stats.p95
            variantP95Ms    = [double]$variant.stats.p95
            baseFirst       = $baseFirst
        }
        $used[$i] = $true
        $used[$i + 1] = $true
    }
    $unpaired = @()
    for ($i = 0; $i -lt $Rows.Count; $i += 1) { if (-not $used[$i]) { $unpaired += $Rows[$i].segment } }
    return [pscustomobject]@{ pairs = $pairs; unpaired = $unpaired }
}

Write-Output ("ab-skip-selftest: SegmentFrames={0} Cycles={1} K={2} SigMinRatio={3} reuse={4}" -f `
    $SegmentFrames, $Cycles, $K, $SigMinRatio, $reuse)

$posJson = $PositiveJson; $negJson = $NegativeJson
$posRunCode = -1; $negRunCode = -1
if ($reuse) {
    foreach ($need in @($PositiveJson, $NegativeJson)) {
        if (-not (Test-Path -LiteralPath $need)) {
            Write-Output ("ab-skip-selftest: PRECHECK FAIL 指定产物不存在 {0}" -f $need); exit 2
        }
    }
    Write-Output "  注意：复用既有产物，本次未重新校验 ab:skip 能力凭证（该把关由产生这批读数的 frame-timing 运行完成）"
} else {
    # 4 段模式重复：使 (BA,skip) 与 (skip,BA) 两种先后各占一半，顺序自检才不会空转。
    $unit = @('BA', 'skip:edges', 'skip:edges', 'BA')
    $posCycle = (($unit * $Cycles) -join ',')
    $negUnit = @('BA', 'BA', 'BA', 'BA')
    $negCycle = (($negUnit * $Cycles) -join ',')
    $posJson = Join-Path $OutDir 'positive.json'
    $negJson = Join-Path $OutDir 'negative.json'
    Write-Output "== 正对照：同 tuning 只变降格 (BA vs skip:edges) =="
    $r1 = Invoke-Timing -Label 'positive' -Cycle $posCycle -JsonPath $posJson
    Write-Output ("  run exit={0}" -f $r1.code)
    Write-Output "== 负对照：同配置 (BA vs BA) =="
    $r2 = Invoke-Timing -Label 'negative' -Cycle $negCycle -JsonPath $negJson
    Write-Output ("  run exit={0}" -f $r2.code)
    $posRunCode = $r1.code; $negRunCode = $r2.code
    $credMatch = [regex]::Match($r1.out, '凭证核对：需要=(\d+) 实得=(\d+) 覆盖spec=\[([^\]]*)\]')
    if ($credMatch.Success) {
        Write-Output ("  能力凭证（继承自 frame-timing 的断言）: want={0} got={1} specs=[{2}]" -f `
            $credMatch.Groups[1].Value, $credMatch.Groups[2].Value, $credMatch.Groups[3].Value)
    }
}

$pos = Get-SegmentRows -JsonPath $posJson
$neg = Get-SegmentRows -JsonPath $negJson

# 负对照两侧同名，Build-ContrastPairs 按定义不会配对；这里为它单独构造"同配置配对"，
# 用途是假阳性检验：两侧配置完全相同，任何显著性都只可能来自噪声或判据缺陷。
function Build-SameConfigPairs {
    param([object[]]$Rows)
    $pairs = @()
    for ($i = 0; $i + 1 -lt $Rows.Count; $i += 2) {
        $x = $Rows[$i]; $y = $Rows[$i + 1]
        $pairs += [pscustomobject]@{
            baseMedianMs    = [double]$x.stats.median
            variantMedianMs = [double]$y.stats.median
            baseP95Ms       = [double]$x.stats.p95
            variantP95Ms    = [double]$y.stats.p95
            baseFirst       = $true
        }
    }
    return [pscustomobject]@{ pairs = $pairs; unpaired = @() }
}

$posBuilt = Build-ContrastPairs -Rows $pos.rows
$negBuilt = Build-SameConfigPairs -Rows $neg.rows
$posCount = $posBuilt.pairs.Count
$negCount = $negBuilt.pairs.Count
Write-Output ("  配对：正对照 pairs={0} unpaired=[{1}] / 负对照 pairs={2}" -f `
    $posCount, ($posBuilt.unpaired -join ' '), $negCount)

$failRun = (-not $reuse) -and ($posRunCode -ne 0 -or $negRunCode -ne 0)
Add-Case 'RUN' '两次 frame-timing 运行均 exit 0' `
    ([bool](-not $failRun)) `
    ($(if ($reuse) { '复用产物，未运行（凭证由产生批次把关）' } else { "positive={0} negative={1}" -f $posRunCode, $negRunCode }))

if ($posCount -lt 4 -or $negCount -lt 4) {
    Add-Case 'N' '配对样本充足（两侧各 >=4 对）' $false ("posPairs={0} negPairs={1}" -f $posCount, $negCount)
} else {
    Add-Case 'N' '配对样本充足（两侧各 >=4 对）' $true ("posPairs={0} negPairs={1}" -f $posCount, $negCount)
}

$pa = Get-PairedAnalysis -Pairs $posBuilt.pairs -K $K -SigMinRatio $SigMinRatio
$na = Get-PairedAnalysis -Pairs $negBuilt.pairs -K $K -SigMinRatio $SigMinRatio
# P3：整批换边重算，mean 必须反号（gap 定义随之翻转）
$swapped = @($posBuilt.pairs | ForEach-Object { [pscustomobject]@{
    baseMedianMs    = $_.variantMedianMs; variantMedianMs = $_.baseMedianMs
    baseP95Ms       = $_.variantP95Ms;    variantP95Ms    = $_.baseP95Ms
    baseFirst       = -not $_.baseFirst
} })
$sa = Get-PairedAnalysis -Pairs $swapped -K $K -SigMinRatio $SigMinRatio

function Show-Side {
    param([string]$Tag, [object]$A)
    Write-Output ("    {0} median: mean={1}ms std={2}ms thr={3}ms sameSign={4} sig={5} rel={6}%" -f `
        $Tag, $A.medianStats.meanMs, $A.medianStats.stdMs, $A.medianStats.thresholdMs, `
        $A.medianStats.sameSign, $A.medianStats.significant, $A.medianStats.relPct)
    Write-Output ("    {0}    p95: mean={1}ms std={2}ms thr={3}ms sameSign={4} sig={5} rel={6}%" -f `
        $Tag, $A.p95Stats.meanMs, $A.p95Stats.stdMs, $A.p95Stats.thresholdMs, `
        $A.p95Stats.sameSign, $A.p95Stats.significant, $A.p95Stats.relPct)
}
Show-Side 'POS' $pa
Show-Side 'NEG' $na

Add-Case 'P1' 'p95 侧显著且变体更快（gap>0）' `
    ($pa.p95Stats.significant -and ($pa.p95Stats.meanMs -gt 0)) `
    ("mean={0}ms thr={1}ms sameSign={2} rel={3}%" -f $pa.p95Stats.meanMs, $pa.p95Stats.thresholdMs, $pa.p95Stats.sameSign, $pa.p95Stats.relPct)
Add-Case 'P2' 'median 侧符号已记录（不判方向成败）' `
    ($null -ne $pa.medianStats.meanMs) `
    ("mean={0}ms sameSign={1} sig={2} rel={3}%" -f $pa.medianStats.meanMs, $pa.medianStats.sameSign, $pa.medianStats.significant, $pa.medianStats.relPct)
Add-Case 'P3' '换边后两侧 mean 均反号' `
    (($sa.medianStats.meanMs * $pa.medianStats.meanMs -lt 0) -and ($sa.p95Stats.meanMs * $pa.p95Stats.meanMs -lt 0)) `
    ("orig median={0} p95={1} / swapped median={2} p95={3}" -f `
        $pa.medianStats.meanMs, $pa.p95Stats.meanMs, $sa.medianStats.meanMs, $sa.p95Stats.meanMs)
Add-Case 'N1' '同配置对照两侧均不显著（无假阳性）' `
    ((-not $na.medianStats.significant) -and (-not $na.p95Stats.significant)) `
    ("median mean={0}ms thr={1}ms sig={2} / p95 mean={3}ms thr={4}ms sig={5}" -f `
        $na.medianStats.meanMs, $na.medianStats.thresholdMs, $na.medianStats.significant, `
        $na.p95Stats.meanMs, $na.p95Stats.thresholdMs, $na.p95Stats.significant)

$posNames = @($pos.rows | ForEach-Object { $_.segment } | Sort-Object -Unique)
$negNames = @($neg.rows | ForEach-Object { $_.segment } | Sort-Object -Unique)
$wantInstances = [math]::Floor($pos.panFrames / $SegmentFrames)
Add-Case 'S1' '实例覆盖 cycle 且段名集合符合预期' `
    (@($pos.rows).Count -ge 4 * $Cycles -and $posNames.Count -eq 2 -and `
     ($posNames -contains 'BA') -and ($posNames -contains 'skip:edges') -and `
     $negNames.Count -eq 1 -and ($negNames -contains 'BA')) `
    ("posInstances={0}(>= {1}) posNames=[{2}] negNames=[{3}]" -f @($pos.rows).Count, (4 * $Cycles), ($posNames -join ' '), ($negNames -join ' '))

$unpairedCount = @($posBuilt.unpaired).Count
Add-Case 'S3' '尾段丢弃与未配对实例已报告且 <=1' `
    ($wantInstances -ge @($pos.rows).Count -and $unpairedCount -le 1) `
    ("boundaries={0} batches={1} tailDropped={2} unpaired={3}[{4}]" -f `
        $wantInstances, @($pos.rows).Count, ($wantInstances - @($pos.rows).Count), $unpairedCount, ($posBuilt.unpaired -join ' '))

$bfN = [int]$pa.orderBias.baseFirstN
$vfN = [int]$pa.orderBias.variantFirstN
Add-Case 'OB' '顺序自检非空转（两组都非空）且未判不可信' `
    ($bfN -gt 0 -and $vfN -gt 0 -and (-not [bool]$pa.orderBias.detected) -and (-not [bool]$pa.orderBias.p95Detected)) `
    ("baseFirstN={0} variantFirstN={1} delta={2}ms/std={3}ms p95Delta={4}ms/std={5}ms" -f `
        $bfN, $vfN, $pa.orderBias.deltaMs, $pa.medianStats.stdMs, $pa.orderBias.p95DeltaMs, $pa.p95Stats.stdMs)

$summary = [ordered]@{
    schemaVersion = 1
    kind          = 'ab-skip-selftest'
    recordedAt    = (Get-Date).ToString('yyyy-MM-ddTHH:mm:ss+08:00')
    head          = (git -C $root rev-parse HEAD)
    reusedArtifacts = [bool]$reuse
    params        = [ordered]@{ SegmentFrames = $SegmentFrames; Cycles = $Cycles; K = $K; SigMinRatio = $SigMinRatio }
    runExitCodes  = [ordered]@{ positive = $posRunCode; negative = $negRunCode }
    pairs         = [ordered]@{ positive = $posCount; negative = $negCount; positiveUnpaired = @($posBuilt.unpaired) }
    positive      = [ordered]@{ median = $pa.medianStats; p95 = $pa.p95Stats; orderBias = $pa.orderBias }
    negative      = [ordered]@{ median = $na.medianStats; p95 = $na.p95Stats }
    swapped       = [ordered]@{ medianMeanMs = $sa.medianStats.meanMs; p95MeanMs = $sa.p95Stats.meanMs }
    cases         = @($script:cases | ForEach-Object { [ordered]@{ id = $_.id; ok = $_.ok; detail = $_.detail } })
}
$summaryPath = Join-Path $OutDir 'summary.json'
$summary | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $summaryPath -Encoding utf8

$bad = @($script:cases | Where-Object { -not $_.ok }).Count
Write-Output ("  摘要: {0}" -f $summaryPath)
Write-Output ("ab-skip-selftest: verdict={0} cases={1} failed={2}" -f `
    $(if ($bad -eq 0) { 'PASS' } else { 'FAIL' }), $script:cases.Count, $bad)
if ($bad -gt 0) { exit 1 }
exit 0
