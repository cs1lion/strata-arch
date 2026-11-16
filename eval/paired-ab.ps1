# 配对 A/B 读数汇总器：只做统计，不启动进程、不改变任何测量口径。
#
# 用法：
#   pwsh -File eval/paired-ab.ps1 -RunDir out/<票>-<时间戳> -Variants labels,edges-labels
#
# 输入文件名约定（由调用方按配对顺序产出，见 docs/tickets/arch-tool/23-input-repaint-route-bc.md）：
#   pair<k>-base.json          基线档（-Mode search -Skip ""）
#   pair<k>-<variant>.json     变体档（-Mode search -Skip "<variant>"）
# 同对内两个文件按 LastWriteTime 判定谁先跑，用于顺序偏置自检（判据 P3）。
#
# 统计口径：
#   gap_k = median(base_k) - median(variant_k)        # 正数表示变体更快
#   显著 = |mean(gap)| > K × (2 × std(gap) / √N)  且  同号对数 / N >= SigMinRatio
#   地板 = 配对差的 std(gap)。
#   median 与 p95 两侧共用同一判据式（Get-PairedStats 单点实现）；两侧 rel% 分母分别取
#   base 的 median 均值与 base 的 p95 均值（同量纲可比）。同号计数统一为「与 mean 同向」。
# 为什么不用同进程段间 std 当分母：2026-11-16 实测批间中位漂移 2.61~3.08ms，
# 远大于段间地板 0.28~3.9ms，段间地板不含批间方差，拿它做批次对比会系统性假阳性
# （见 out/t23-stability-searchload-* 与 out/t23-stage0-* 的 JSON）。
#
# 退出码：0 = 汇总完成（结论见输出，含「不显著」与「顺序效应」两种如实标注）；
#         1 = 输入不完整/不一致（缺对、非零 exitCode、mapLine 漂移），不产出结论。

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$RunDir,
    [string[]]$Variants = @("labels", "edges-labels"),
    [double]$K = 2,
    # 符号检验门槛：6 对里至少 5 对同号（0.8334 ≈ 5/6），与判据 P2 一致。
    [double]$SigMinRatio = 0.8334,
    [string]$OutJson = ""
)

$ErrorActionPreference = "Stop"
# 判据式与配对分析来自共享 lib：paired-ab 与 selftest 必须用同一实现，禁止在本文件另写一份
# （docs/process/eval-harness.md §6.1 第 6 条）。
. (Join-Path $PSScriptRoot "lib-paired-stats.ps1")
if (-not (Test-Path -LiteralPath $RunDir)) {
    Write-Output "paired-ab: run 目录不存在 $RunDir"
    exit 1
}

# Get-StdDev / Get-PairedStats / Get-PairedAnalysis 均来自 eval/lib-paired-stats.ps1（已在上方 dot-source），
# 本文件不再持有判据式实现。

function Read-RunJson {
    param([string]$Path)
    $d = Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
    if ($d.exitCode -ne 0) { throw "exitCode=$($d.exitCode) in $Path" }
    if ($null -eq $d.intervals.medianMs -or $d.intervals.count -lt 1) { throw "无间隔读数 in $Path" }
    return $d
}

# 守卫：所有 run 的语料规模行必须一致，否则配对不成立（判据 P1/P4 的前提）。
$basePaths = @{}
foreach ($f in Get-ChildItem -LiteralPath $RunDir -Filter "pair*-base.json") {
    if ($f.Name -match '^pair(\d+)-base\.json$') { $basePaths[[int]$Matches[1]] = $f.FullName }
}
if ($basePaths.Count -eq 0) {
    Write-Output "paired-ab: 未找到 pair<k>-base.json"
    exit 1
}

$mapLines = @()
foreach ($p in $basePaths.Values) { $mapLines += (Read-RunJson -Path $p).mapLine }
$mapConsistent = (@($mapLines | Select-Object -Unique).Count -eq 1)

$results = @()
foreach ($variant in $Variants) {
    $pairs = @()
    # 注意：循环变量绝不能叫 $k —— 它与参数 [double]$K 在 PowerShell 里大小写不敏感、是同一个变量，
    # 参数类型约束会把键强制转成 Double，导致 Int32 键的 hashtable 查找返回空（实测 type=Double / contains=False）。
    foreach ($pairIndex in ($basePaths.Keys | Sort-Object)) {
        $variantPath = Join-Path $RunDir "pair${pairIndex}-${variant}.json"
        if (-not (Test-Path -LiteralPath $variantPath)) {
            Write-Output "paired-ab: 缺第 $pairIndex 对的 $variant 侧，跳过该对（不补单侧）"
            continue
        }
        $basePath = $basePaths[$pairIndex]
        $b = Read-RunJson -Path $basePath
        $v = Read-RunJson -Path $variantPath
        $bWrite = (Get-Item -LiteralPath $basePath).LastWriteTime
        $vWrite = (Get-Item -LiteralPath $variantPath).LastWriteTime
        $pairs += [pscustomobject]@{
            index          = $pairIndex
            baseMedianMs   = $b.intervals.medianMs
            variantMedianMs = $v.intervals.medianMs
            gapMs          = [math]::Round($b.intervals.medianMs - $v.intervals.medianMs, 2)
            baseP95Ms      = $b.intervals.p95Ms
            variantP95Ms   = $v.intervals.p95Ms
            gapP95Ms       = [math]::Round($b.intervals.p95Ms - $v.intervals.p95Ms, 2)
            baseIntervals  = $b.intervals.count
            variantIntervals = $v.intervals.count
            baseFirst      = ($bWrite -le $vWrite)
        }
        $mapLines += $v.mapLine
    }
    if ($pairs.Count -eq 0) {
        Write-Output "paired-ab: $variant 无完整配对"
        continue
    }
    # 全部配对统计（含两侧顺序偏置自检）由共享 lib 产出；gap 在 lib 内按 base − 变体 现算。
    $a = Get-PairedAnalysis -Pairs $pairs -K $K -SigMinRatio $SigMinRatio
    $n = $a.n
    $meanBase = $a.meanBaseMs
    $meanBaseP95 = $a.meanBaseP95Ms
    $medStats = $a.medianStats
    $p95Stats = $a.p95Stats
    $baseFirstGaps = @($a.pairs | Where-Object { $_.baseFirst } | ForEach-Object { $_.gapMs })
    $orderBiasOut = $a.orderBias

    $results += [pscustomobject]@{
        variant           = $variant
        pairs             = $n
        # 以下左侧字段名与含义保持与旧版逐字一致（回归判据要求），值改由共用函数产出。
        meanGapMs         = $medStats.meanMs
        gapRelPct         = $medStats.relPct
        stdGapMs          = $medStats.stdMs
        thresholdMs       = $medStats.thresholdMs
        sameSign          = $medStats.sameSign
        significant       = $medStats.significant
        meanBaseMedianMs  = $meanBase
        meanVariantMedianMs = $a.meanVariantMs
        meanGapP95Ms      = $p95Stats.meanMs
        medianStats       = $medStats
        p95Stats          = $p95Stats
        meanBaseP95Ms     = $meanBaseP95
        # orderBias 由 lib 产出；旧键名全部保留（baseFirstN/variantFirstN/meanBaseFirstMs/
        # meanVariantFirstMs/deltaMs/detected/p95MeanBaseFirstMs/p95MeanVariantFirstMs/
        # p95DeltaMs/p95Detected），并新增 p95BaseFirstN/p95VariantFirstN 两个计数键（只增不改）。
        orderBias         = $orderBiasOut
        perPair           = @($pairs)
    }
}

Write-Output ("paired-ab: run={0} base对数={1} mapLine一致={2}" -f $RunDir, $basePaths.Count, $mapConsistent)
if (-not $mapConsistent) {
    Write-Output "paired-ab: 语料规模行不一致，配对前提不成立，不产出结论"
    exit 1
}
Write-Output ""
Write-Output ("{0,-14} {1,3} {2,9} {3,9} {4,9} {5,9} {6,7} {7,8} {8,9}" -f `
        "variant", "N", "mean(gap)", "rel%", "std(gap)", "门槛", "同号", "显著", "gap(p95)")
foreach ($r in $results) {
    Write-Output ("{0,-14} {1,3} {2,9} {3,9} {4,9} {5,9} {6,7} {7,8} {8,9}" -f `
            $r.variant, $r.pairs, $r.meanGapMs, $r.gapRelPct, $r.stdGapMs, $r.thresholdMs, `
            $r.sameSign, $r.significant, $r.meanGapP95Ms)
}
Write-Output ""
Write-Output "  （上表为 median 侧；下表为 p95 侧，同一判据式 Get-PairedStats，rel% 分母取 base 的 p95 均值）"
Write-Output ("{0,-14} {1,3} {2,9} {3,9} {4,9} {5,9} {6,7} {7,8}" -f `
        "variant(p95)", "N", "mean(gap)", "rel%", "std(gap)", "门槛", "同号", "显著")
foreach ($r in $results) {
    $s = $r.p95Stats
    Write-Output ("{0,-14} {1,3} {2,9} {3,9} {4,9} {5,9} {6,7} {7,8}" -f `
            $r.variant, $s.count, $s.meanMs, $s.relPct, $s.stdMs, $s.thresholdMs, $s.sameSign, $s.significant)
}
Write-Output ""
foreach ($r in $results) {
    $o = $r.orderBias
    $flag = if ($o.detected) { "顺序效应检出（判据 P3 不成立，本口径无效，如实报告不调判据）" } else { "无" }
    Write-Output ("  {0}: 方向 base-first={1} 对 mean={2}ms / variant-first={3} 对 mean={4}ms，|Δ|={5}ms vs std={6}ms => 顺序效应: {7}" -f `
            $r.variant, $o.baseFirstN, $o.meanBaseFirstMs, $o.variantFirstN, $o.meanVariantFirstMs, $o.deltaMs, $r.stdGapMs, $flag)
    Write-Output ("    每对 gap(ms): " + (($r.perPair | ForEach-Object { $_.gapMs }) -join ", "))
    Write-Output ("    每对间隔数: base " + (($r.perPair | ForEach-Object { $_.baseIntervals }) -join "/") +
            " | variant " + (($r.perPair | ForEach-Object { $_.variantIntervals }) -join "/"))
    Write-Output ("    每对 gapP95(ms): " + (($r.perPair | ForEach-Object { $_.gapP95Ms }) -join ", "))
    $o95 = if ($r.orderBias.p95Detected) { "顺序效应检出" } else { "无" }
    Write-Output ("    p95 方向自检: base-first={0} 对 mean={1}ms / variant-first={2} 对 mean={3}ms，|Δ|={4}ms vs std={5}ms => {6}" -f `
            $r.orderBias.baseFirstN, $r.orderBias.p95MeanBaseFirstMs, $r.orderBias.variantFirstN, `
            $r.orderBias.p95MeanVariantFirstMs, $r.orderBias.p95DeltaMs, $r.p95Stats.stdMs, $o95)
}
Write-Output ""
Write-Output "  说明：-Skip 全程生效 ≠ 路线 C 的「只在 SearchMode::Input 期间生效」；本读数为成本上界筛查，不作路线 C 验收（判据 P5）。"

if ($OutJson) {
    $outDir = Split-Path -Parent $OutJson
    if ($outDir -and -not (Test-Path -LiteralPath $outDir)) {
        New-Item -ItemType Directory -Force -Path $outDir | Out-Null
    }
    [pscustomobject]@{
        runDir         = $RunDir
        k              = $K
        sigMinRatio    = $SigMinRatio
        basePairCount  = $basePaths.Count
        mapLineConsistent = $mapConsistent
        results        = @($results)
    } | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $OutJson -Encoding utf8
    Write-Output ("  wrote {0}" -f $OutJson)
}
exit 0
