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
if ($diffs.Count -gt 0) {
    Write-Output "selftest: FAIL"
    exit 1
}
Write-Output "selftest: PASS（判据式重算与夹具期望逐字段一致）"
exit 0
