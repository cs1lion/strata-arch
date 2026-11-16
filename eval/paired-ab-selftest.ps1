# paired-ab 判据式的 CI 自测。
#
# 目的：让「判据式、减法方向、舍入位数、配对聚合」被改坏时 CI 直接变红，
# 而不是只靠某个人的本机复算记忆。夹具存的是**每对原始读数**（不是结果数字），
# 因此校验时必须用与 paired-ab 相同的实现（eval/lib-paired-stats.ps1）重算。
#
# 覆盖边界（如实声明，勿当作全套守护）：
#   - 守住：Get-PairedStats / Get-PairedAnalysis 的判据式、gap 现算方向、Round 位数、
#           顺序偏置的分组与阈值比较、两份脚本的语法与参数面。
#   - 守不住：paired-ab 读取 out/**/pair*.json 的路径与守卫逻辑、frame-timing 的真实测量、
#             像素证据——那些仍需可见 Windows 桌面会话，属本机人工项（docs/acceptance-gaps.md）。
#
# 用法：
#   本机生成夹具（一次性）：
#     pwsh -File eval/paired-ab-selftest.ps1 -Generate `
#          -GenerateFrom out/paired-regression-<ts> -OutFixture eval/baseline/paired-ab-selftest.json
#   CI / 本机校验：
#     pwsh -File eval/paired-ab-selftest.ps1 -Fixture eval/baseline/paired-ab-selftest.json
#
# 夹具必须由 -Generate 生成，禁止手抄或手调数字（§防作弊第 3 条）。
# 退出码：0 = 全部重算与期望一致且冒烟通过；1 = 存在 DIFF 或冒烟失败。

[CmdletBinding()]
param(
    [string]$Fixture = "eval/baseline/paired-ab-selftest.json",
    [switch]$Generate,
    [string]$GenerateFrom = "",
    [string]$OutFixture = "",
    [string]$PairsScript = "eval/paired-ab.ps1",
    [string]$LibScript = "eval/lib-paired-stats.ps1"
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
. (Join-Path $PSScriptRoot "lib-paired-stats.ps1")

# 夹具的组清单写死在此处：新增批次要改脚本（可审），不在命令行里靠逗号传数组
# （`pwsh -File` 下含逗号的参数会绑成单个字符串，不拆数组）。
$GroupSpecs = @(
    @{ batch = "paired-c-20261116-173507";  variant = "labels" },
    @{ batch = "paired-c-20261116-173507";  variant = "edges-labels" },
    @{ batch = "paired-c2-20261116-190406"; variant = "edges" },
    @{ batch = "paired-c2-20261116-190406"; variant = "edges-labels" },
    @{ batch = "paired-c3-20261116-191156"; variant = "labels" },
    @{ batch = "paired-c3-20261116-191156"; variant = "edges" },
    @{ batch = "paired-c3-20261116-191156"; variant = "edges-labels" }
)
$StatKeys = @('count', 'meanMs', 'stdMs', 'thresholdMs', 'sameSign', 'signRatio', 'significant', 'relPct')

function Test-ScriptSyntax {
    param([string]$Path)
    $errs = $null; $tok = $null
    [System.Management.Automation.Language.Parser]::ParseFile((Resolve-Path $Path).Path, [ref]$tok, [ref]$errs) | Out-Null
    return [pscustomobject]@{ path = $Path; errors = @($errs).Count }
}

# 性质断言（property checks）：不比对成品数字，而是断言判据函数**必须尊重入参与方向定义**。
# 为什么必须存在：夹具 14 个「组×侧」里，过门槛的 6 组 ratio 全为 6/6，ratio 不足者全未过门槛，
# 两条件交集为空 ⇒ 有人把同号阈值从 5/6 放宽到 1/2，夹具重算结果一字不变（2026-11-16 注入实测）。
# 期望值是语义规则，不引入新数字，也不依赖合成数据。
function Invoke-PropertyChecks {
    param([object]$Groups, [ref]$Checked, [ref]$Diffs)

    $SidePairs = @(@('medianStats', 'meanBaseMs'), @('p95Stats', 'meanBaseP95Ms'))

    # 受试组：取第一个「median 侧判为显著」的组（需同时满足过门槛与全同号，才能暴露入参被忽略）。
    $subj = @($Groups | Where-Object { $_.expected.median.significant -eq $true }) | Select-Object -First 1
    if (-not $subj) {
        $Diffs.Value += "性质断言无法执行：夹具里没有 median 侧判为显著的组（不可执行按失败计，不得静默跳过）"
        $Checked.Value++
        return 1   # 早退路径的计划项数为 1（就是上面这条不可执行断言）
    }
    $tag = "{0}/{1}" -f $subj.batch, $subj.variant
    $base = Get-PairedAnalysis -Pairs $subj.pairs -K 2 -SigMinRatio 0.8334

    # PC1 同号阈值敏感性（打穿 B 类盲区）
    $hi = Get-PairedAnalysis -Pairs $subj.pairs -K 2 -SigMinRatio 1.001
    foreach ($sp in $SidePairs) {
        $side = $sp[0]
        $Checked.Value++
        if ($base.$side.significant -ne $true) {
            $Diffs.Value += "PC1 前置 $tag [$side] 规范阈值下 significant 应为 True，实得 $($base.$side.significant)"
        }
        $Checked.Value++
        if ($hi.$side.significant -ne $false) {
            $Diffs.Value += "PC1 $tag [$side] SigMinRatio=1.001 时 significant 必须为 False（实得 True ⇒ 判据忽略入参，同号阈值被写死）"
        }
    }

    # PC2 k 敏感性
    $kk = Get-PairedAnalysis -Pairs $subj.pairs -K 1000 -SigMinRatio 0.8334
    $Checked.Value++
    if ($kk.medianStats.thresholdMs -le $base.medianStats.thresholdMs) {
        $Diffs.Value += "PC2 $tag K=1000 的门槛必须大于 K=2（$($kk.medianStats.thresholdMs) <= $($base.medianStats.thresholdMs) ⇒ k 被写死）"
    }
    $Checked.Value++
    if ($kk.medianStats.significant -ne $false) {
        $Diffs.Value += "PC2 $tag K=1000 时不应仍判显著（实得 True ⇒ k 未参与门槛计算）"
    }

    # PC3 方向性：base 与变体互换 ⇒ meanMs 反号、同号计数不变
    $swap = @($subj.pairs | ForEach-Object { [pscustomobject]@{
        baseMedianMs    = $_.variantMedianMs
        variantMedianMs = $_.baseMedianMs
        baseP95Ms       = $_.variantP95Ms
        variantP95Ms    = $_.baseP95Ms
        baseFirst       = $_.baseFirst
    } })
    $sw = Get-PairedAnalysis -Pairs $swap -K 2 -SigMinRatio 0.8334
    foreach ($sp in $SidePairs) {
        $side = $sp[0]
        $Checked.Value++
        if ([math]::Abs($sw.$side.meanMs + $base.$side.meanMs) -gt 0.005) {
            $Diffs.Value += ("PC3 {0} [{1}] 互换后 meanMs 必须反号：原 {2} 互换 {3}" -f $tag, $side, $base.$side.meanMs, $sw.$side.meanMs)
        }
        $Checked.Value++
        if ("$($sw.$side.sameSign)" -ne "$($base.$side.sameSign)") {
            $Diffs.Value += ("PC3 {0} [{1}] 互换后同号计数应不变：{2} -> {3}" -f $tag, $side, $base.$side.sameSign, $sw.$side.sameSign)
        }
    }

    # PC4 rel% 分母定义：必须用同侧 base 均值（p95 侧误用 median 均值即失败）
    foreach ($sp in $SidePairs) {
        $side = $sp[0]; $meanField = $sp[1]
        $want = if ($base.$meanField -gt 0) { [math]::Round($base.$side.meanMs / $base.$meanField * 100, 2) } else { $null }
        $Checked.Value++
        if ("$($base.$side.relPct)" -ne "$want") {
            $Diffs.Value += ("PC4 {0} [{1}] relPct 必须等于 meanMs/同侧 base 均值：脚本 {2} 应为 {3}（base 字段 {4}）" -f `
                    $tag, $side, $base.$side.relPct, $want, $meanField)
        }
    }
    # 纪律 §5 第 7 条坑③：函数内不得再 Write-Output——它会并入返回值，使本函数回报的项数被污染，
    # 表现为「合计行仍对、明细却丢」。受试组信息由调用方打印（见主脚本的 性质断言 段落）。
    return 12   # PC1 4 项 + PC2 2 项 + PC3 4 项 + PC4 2 项
}

if ($Generate) {
    if (-not $OutFixture) {
        Write-Output "selftest: -Generate 需要同时给 -OutFixture"
        exit 1
    }
    if (-not (Test-Path -LiteralPath $GenerateFrom)) {
        Write-Output "selftest: 源目录不存在 $GenerateFrom"
        exit 1
    }
    $groups = @()
    foreach ($spec in $GroupSpecs) {
        $src = Join-Path $GenerateFrom "$($spec.batch)-$($spec.variant).json"
        if (-not (Test-Path -LiteralPath $src)) {
            Write-Output "selftest: 缺源文件 $src"
            exit 1
        }
        $r = (Get-Content -LiteralPath $src -Raw | ConvertFrom-Json).results[0]
        $groups += [pscustomobject]@{
            batch   = $spec.batch
            variant = $spec.variant
            pairs   = @($r.perPair | ForEach-Object { [pscustomobject]@{
                baseMedianMs    = $_.baseMedianMs
                variantMedianMs = $_.variantMedianMs
                baseP95Ms       = $_.baseP95Ms
                variantP95Ms    = $_.variantP95Ms
                baseFirst       = $_.baseFirst
            } })
            # expected 取生成当时脚本自身的产出：本夹具的作用是钉住行为，
            # 不是引入第三方权威数字。生成后仍需人工核对一次与文档读数一致（见笔 2 判据）。
            expected = [pscustomobject]@{
                median = [pscustomobject]@{
                    count = $r.medianStats.count; meanMs = $r.medianStats.meanMs; stdMs = $r.medianStats.stdMs
                    thresholdMs = $r.medianStats.thresholdMs; sameSign = $r.medianStats.sameSign
                    signRatio = $r.medianStats.signRatio; significant = $r.medianStats.significant
                    relPct = $r.medianStats.relPct
                }
                p95 = [pscustomobject]@{
                    count = $r.p95Stats.count; meanMs = $r.p95Stats.meanMs; stdMs = $r.p95Stats.stdMs
                    thresholdMs = $r.p95Stats.thresholdMs; sameSign = $r.p95Stats.sameSign
                    signRatio = $r.p95Stats.signRatio; significant = $r.p95Stats.significant
                    relPct = $r.p95Stats.relPct
                }
            }
        }
    }
    $fixtureObj = [pscustomobject]@{
        schemaVersion = 1
        kind          = "paired-ab-criteria-regression"
        generatedAt   = (Get-Date).ToString("o")
        generator     = "eval/paired-ab-selftest.ps1 -Generate"
        sourceRuns    = @($GroupSpecs | ForEach-Object { Join-Path $GenerateFrom "$($_.batch)-$($_.variant).json" })
        originCommits = @('4443f3c', 'e5dbbae')
        k             = 2
        sigMinRatio   = 0.8334
        groups        = @($groups)
    }
    $outDir = Split-Path -Parent $OutFixture
    if ($outDir -and -not (Test-Path -LiteralPath $outDir)) {
        New-Item -ItemType Directory -Force -Path $outDir | Out-Null
    }
    $json = $fixtureObj | ConvertTo-Json -Depth 8
    Set-Content -LiteralPath $OutFixture -Value $json -Encoding utf8
    Write-Output ("selftest: 生成夹具 {0} 组={1} 字节={2}" -f $OutFixture, $groups.Count,
        (Get-Item -LiteralPath $OutFixture).Length)
    exit 0
}

# ---- 校验模式（CI 走这里）----
if (-not (Test-Path -LiteralPath $Fixture)) {
    Write-Output "selftest: 夹具不存在 $Fixture"
    exit 1
}
$fx = Get-Content -LiteralPath $Fixture -Raw | ConvertFrom-Json
Write-Output ("selftest: fixture={0} groups={1} k={2} sigMinRatio={3} generatedAt={4}" -f `
        $Fixture, $fx.groups.Count, $fx.k, $fx.sigMinRatio, $fx.generatedAt)

$diffs = @()
$checked = 0
foreach ($g in $fx.groups) {
    $a = Get-PairedAnalysis -Pairs $g.pairs -K $fx.k -SigMinRatio $fx.sigMinRatio
    foreach ($side in 'median', 'p95') {
        $actual = if ($side -eq 'median') { $a.medianStats } else { $a.p95Stats }
        $want = $g.expected.$side
        foreach ($key in $StatKeys) {
            $checked++
            if ("$($actual.$key)" -ne "$($want.$key)") {
                $diffs += ("{0}/{1} [{2}] {3}: 期望 [{4}] 实得 [{5}]" -f $g.batch, $g.variant, $side, $key, $want.$key, $actual.$key)
            }
        }
    }
    $nPairs = @($g.pairs).Count
    $checked++
    if ($a.n -ne $nPairs) { $diffs += ("{0}/{1} 配对数 {2} != {3}" -f $g.batch, $g.variant, $a.n, $nPairs) }
}

Write-Output ""
Write-Output "性质断言（判据必须尊重入参、方向与分母定义）："
# 计划项数独立推导，不采用被检函数回报的数字：回报值与实得同源，PC 内部漏加一次
# $Checked.Value++ 会让「计划」与「实得」一起变小、等式照样成立，抓不到漏计。
$subjExists = (@($fx.groups | Where-Object { $_.expected.median.significant -eq $true }).Count -gt 0)
$plannedPcItems = if ($subjExists) { 12 } else { 1 }
$expectedChecks = 17 * @($fx.groups).Count + $plannedPcItems + 7
$pcItems = Invoke-PropertyChecks -Groups $fx.groups -Checked ([ref]$checked) -Diffs ([ref]$diffs)
Write-Output ("  性质断言受试组: {0}（计划项数 {1}，函数回报 {2}）" -f `
        $(if ($subjExists) { '存在' } else { '无 ⇒ 按失败计' }), $plannedPcItems, $pcItems)
if ($pcItems -ne $plannedPcItems) {
    $diffs += ("PC 回报项数 {0} != 计划 {1}" -f $pcItems, $plannedPcItems)
}

# 冒烟：两份脚本语法 + paired-ab 参数面（覆盖「脚本坏了但没人跑它」的盲区）
$synLib = Test-ScriptSyntax -Path $LibScript
$synMain = Test-ScriptSyntax -Path $PairsScript
foreach ($s in @($synLib, $synMain)) {
    $checked++
    if ($s.errors -ne 0) { $diffs += ("语法错误 {0} = {1}" -f $s.path, $s.errors) }
}
$paramNames = (Get-Command $PairsScript).Parameters.Keys
foreach ($p in @('RunDir', 'Variants', 'K', 'SigMinRatio', 'OutJson')) {
    $checked++
    if ($paramNames -notcontains $p) { $diffs += ("paired-ab 缺参数 {0}" -f $p) }
}

Write-Output ""
Write-Output ("检查项总数 = {0} 不匹配数 = {1}" -f $checked, $diffs.Count)
foreach ($d in $diffs) { Write-Output ("  DIFF {0}" -f $d) }

# 计数不变量（纪律 §5 第 7 条坑②的通用尺子）：实执行项数必须等于计划项数。
# 不成立意味着有分支被跳过或重复计数——那种情况下「不匹配数=0」本身不可信。
$invariantOk = ($checked -eq $expectedChecks)
Write-Output ("  不变量 实际检查项={0} 计划={1}（17×{2}组 + PC{3} + 冒烟7）=> {4}" -f `
        $checked, $expectedChecks, @($fx.groups).Count, $plannedPcItems,
        $(if ($invariantOk) { '成立' } else { '不成立 ⇒ 有分支被跳过或重复计数' }))
if (-not $invariantOk) {
    Write-Output "selftest: FAIL（计数不变量不成立，逐字段比对的结论不予采信）"
    exit 1
}
if ($diffs.Count -gt 0) {
    Write-Output "selftest: FAIL"
    exit 1
}
Write-Output "selftest: PASS（判据式重算与夹具期望逐字段一致）"
exit 0
