# 配对 A/B 判据式的**唯一实现处**。
#
# 谁在用：
#   - eval/paired-ab.ps1            真实 run 目录汇总（读 out/**/pair*.json）
#   - eval/paired-ab-selftest.ps1   CI 自测（读 eval/baseline/paired-ab-selftest.json 夹具）
# 两侧共用同一实现，防止「文档说一套、脚本算一套」或两处口径漂移
# （docs/process/eval-harness.md §6.1 第 6 条）。
#
# 判据（不得在此之外另写一份）：
#   显著 = |mean(gap)| > K × (2 × std(gap) / √N)  且  与 mean 同号的对数 / N >= SigMinRatio
#   同号定义 = 与 mean 同向的对数，**不是** max(正,负)；两者在 mean≈0 时给出不同数字。
#   gap = base − 变体，正数表示变体更优（更小的 ms）。

function Get-StdDev {
    param([double[]]$Values)
    $n = $Values.Count
    if ($n -lt 2) { return $null }
    $mean = ($Values | Measure-Object -Average).Average
    $sumSq = 0.0
    foreach ($v in $Values) { $sumSq += ($v - $mean) * ($v - $mean) }
    return [math]::Round([math]::Sqrt($sumSq / ($n - 1)), 4)
}

# 配对差统计：判据式子的唯一实现处，median 与 p95 两侧共用。
# 同号定义固定为「与 mean 同向」（不是 max(正,负)）——已登记的读数一律按此定义，勿混用。
function Get-PairedStats {
    param([double[]]$Gaps, [double]$BaseMean, [double]$K, [double]$SigMinRatio)
    $n = $Gaps.Count
    if ($n -eq 0) { return $null }
    $mean = [math]::Round(($Gaps | Measure-Object -Average).Average, 3)
    $std = Get-StdDev $Gaps
    $threshold = if ($std -ne $null) { [math]::Round($K * 2 * $std / [math]::Sqrt($n), 3) } else { $null }
    $sameSign = if ($mean -ge 0) { @($Gaps | Where-Object { $_ -ge 0 }).Count } else { @($Gaps | Where-Object { $_ -le 0 }).Count }
    $ratio = [math]::Round($sameSign / $n, 4)
    return [pscustomobject]@{
        count       = $n
        meanMs      = $mean
        stdMs       = $std
        thresholdMs = $threshold
        sameSign    = "$sameSign/$n"
        signRatio   = $ratio
        significant = ($null -ne $threshold) -and ([math]::Abs($mean) -gt $threshold) -and ($ratio -ge $SigMinRatio)
        relPct      = if ($BaseMean -gt 0) { [math]::Round($mean / $BaseMean * 100, 2) } else { $null }
        valuesMs    = @($Gaps)
    }
}

# 配对分析：输入每对的原始读数，产出两侧完整检验 + 顺序偏置自检。
#
# 输入 $Pairs 每项需含：baseMedianMs / variantMedianMs / baseP95Ms / variantP95Ms / baseFirst
# （历史字段 gapMs / gapP95Ms 即便存在也**不使用**——gap 一律现算，
#  这样「减法方向与舍入位数」本身也在自测覆盖范围内，改坏会被 CI 抓到。）
function Get-PairedAnalysis {
    param([object[]]$Pairs, [double]$K, [double]$SigMinRatio)

    $rows = @($Pairs | ForEach-Object { [pscustomobject]@{
        baseMedianMs    = [double]$_.baseMedianMs
        variantMedianMs = [double]$_.variantMedianMs
        baseP95Ms       = [double]$_.baseP95Ms
        variantP95Ms    = [double]$_.variantP95Ms
        baseFirst       = [bool]$_.baseFirst
        gapMs           = [math]::Round([double]$_.baseMedianMs - [double]$_.variantMedianMs, 2)
        gapP95Ms        = [math]::Round([double]$_.baseP95Ms - [double]$_.variantP95Ms, 2)
    } })

    $n = $rows.Count
    $meanBase = [math]::Round(($rows | Measure-Object -Property baseMedianMs -Average).Average, 3)
    # p95 侧 rel% 的分母取 base 的 p95 均值（同量纲才可比；用户裁决 2026-11-16）。
    $meanBaseP95 = [math]::Round(($rows | Measure-Object -Property baseP95Ms -Average).Average, 3)

    $medStats = Get-PairedStats -Gaps @($rows | ForEach-Object { $_.gapMs }) -BaseMean $meanBase -K $K -SigMinRatio $SigMinRatio
    $p95Stats = Get-PairedStats -Gaps @($rows | ForEach-Object { $_.gapP95Ms }) -BaseMean $meanBaseP95 -K $K -SigMinRatio $SigMinRatio

    # 顺序偏置自检：两种先后方向各算一次 mean(gap)，差值超过该侧 std 即判该批不可信。
    $baseFirstGaps = @($rows | Where-Object { $_.baseFirst } | ForEach-Object { $_.gapMs })
    $variantFirstGaps = @($rows | Where-Object { -not $_.baseFirst } | ForEach-Object { $_.gapMs })
    $meanBaseFirst = if ($baseFirstGaps.Count) { [math]::Round(($baseFirstGaps | Measure-Object -Average).Average, 3) } else { $null }
    $meanVariantFirst = if ($variantFirstGaps.Count) { [math]::Round(($variantFirstGaps | Measure-Object -Average).Average, 3) } else { $null }
    $orderDelta = if ($null -ne $meanBaseFirst -and $null -ne $meanVariantFirst) {
        [math]::Round([math]::Abs($meanBaseFirst - $meanVariantFirst), 3)
    } else { $null }
    $orderEffect = ($null -ne $orderDelta) -and ($null -ne $medStats.stdMs) -and ($orderDelta -gt $medStats.stdMs)

    $baseFirstP95 = @($rows | Where-Object { $_.baseFirst } | ForEach-Object { $_.gapP95Ms })
    $variantFirstP95 = @($rows | Where-Object { -not $_.baseFirst } | ForEach-Object { $_.gapP95Ms })
    $meanP95BaseFirst = if ($baseFirstP95.Count) { [math]::Round(($baseFirstP95 | Measure-Object -Average).Average, 3) } else { $null }
    $meanP95VariantFirst = if ($variantFirstP95.Count) { [math]::Round(($variantFirstP95 | Measure-Object -Average).Average, 3) } else { $null }
    $orderDeltaP95 = if ($null -ne $meanP95BaseFirst -and $null -ne $meanP95VariantFirst) {
        [math]::Round([math]::Abs($meanP95BaseFirst - $meanP95VariantFirst), 3)
    } else { $null }
    $orderEffectP95 = ($null -ne $orderDeltaP95) -and ($null -ne $p95Stats.stdMs) -and ($orderDeltaP95 -gt $p95Stats.stdMs)

    return [pscustomobject]@{
        pairs           = $rows
        n               = $n
        meanBaseMs      = $meanBase
        meanBaseP95Ms   = $meanBaseP95
        meanVariantMs   = [math]::Round(($rows | Measure-Object -Property variantMedianMs -Average).Average, 3)
        medianStats     = $medStats
        p95Stats        = $p95Stats
        orderBias       = [pscustomobject]@{
            baseFirstN          = $baseFirstGaps.Count
            variantFirstN       = $variantFirstGaps.Count
            meanBaseFirstMs     = $meanBaseFirst
            meanVariantFirstMs  = $meanVariantFirst
            deltaMs             = $orderDelta
            detected            = $orderEffect
            p95BaseFirstN       = $baseFirstP95.Count
            p95VariantFirstN    = $variantFirstP95.Count
            p95MeanBaseFirstMs  = $meanP95BaseFirst
            p95MeanVariantFirstMs = $meanP95VariantFirst
            p95DeltaMs          = $orderDeltaP95
            p95Detected         = $orderEffectP95
        }
    }
}
