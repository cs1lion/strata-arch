# drift 门禁：判断「哪些改动需要跑漂移检测、跑哪个基线」。
#
# 两类基线（不混用）：
#   - self：仓库自身可提交的 fixture 树 `tests/fixtures/drift/` 与仓库内基线
#     `tests/fixtures/drift/drift-baseline.json`。目标与基线都在仓内，CI 一定能跑。
#     Strata 自身是 Rust workspace，漂移检测只索引 TS/JS；fixture 树是仓内唯一
#     有提交基线、且随架构规则变化而变化的 TS/JS 样本，因此「仓库自身」指它，
#     而不是扫描整个仓库（后者会把 callgraph/history fixture 混进来，噪声不可解释）。
#   - corpus：外部固定语料 `eval/corpus.toml` 钉住的 tsx / excalidraw 快照，
#     基线在 `eval/baseline/drift-<name>.json`。语料缓存在 `tests/corpus/`
#     （.gitignore 排除）；CI 用 eval/fetch-corpus.ps1 + pin 校验拉取并缓存。
#
# 触发判据（按改动路径，任一命中即 required）：
#   self   = engine + `tests/fixtures/drift/**` + 本脚本
#   corpus = engine + `eval/corpus.toml` + `eval/baseline/drift-*.json`
#   engine = `crates/arch-core/**/{src,Cargo.toml}`、`crates/arch-cli/**/{src,Cargo.toml}`、
#            `Cargo.lock`
#   engine 之外的改动（文档、视图层、其它 fixture、CI 配置）不触发 drift。
#
# 语料缺失时的语义（分两档，不一刀切）：
#   - 基线改动（`eval/corpus.toml` 或 `eval/baseline/drift-*.json`）且语料不可用：
#     直接判红（退出 1），提示「基线改动必须在有语料的环境验证，本地跑 -RequireCorpus」。
#   - 其它需要 corpus 的改动：corpus 标 skipped，写进 coverage.missing，退出码 0；
#     缺的是外部语料上的四类漂移回归（tsx 136 模块 / excalidraw 802 模块），
#     fixture 小样本无法暴露大仓库上的解析/分层退化。
#   - 评估环境要 fail closed 时加 `-RequireCorpus`，语料缺失按配置错误退出 2。
#
# 退出码：
#   0 = 所需门禁全部通过（或未触发；非基线改动时语料缺失按降级跳过）
#   1 = 任一已执行的 drift 门禁判红，或基线改动 + 语料不可用
#   2 = 配置/输入错误（二进制或基线缺失、git 基线无法解析、-RequireCorpus 且语料缺失）
#
# 用法：
#   pwsh eval/drift-gate.ps1                                   # 与 origin/main 比较
#   pwsh eval/drift-gate.ps1 -BaseRef origin/main
#   pwsh eval/drift-gate.ps1 -ChangedFiles a.rs,b.ts           # 覆盖改动集，便于复现
#   pwsh eval/drift-gate.ps1 -CorpusRoot D:\project\1107\tests\corpus
#   pwsh eval/drift-gate.ps1 -CorpusUnavailable                # CI pin 校验失败时强制视为缺失
#   pwsh eval/drift-gate.ps1 -PlanOnly                         # 只输出计划，不跑检测
#   pwsh eval/drift-gate.ps1 -ForceFull                        # 保守全跑（workflow_dispatch 用）
#   pwsh eval/drift-gate.ps1 -RequireCorpus                    # 评估环境：语料缺失即失败

[CmdletBinding()]
param(
    [string]$BaseRef = "origin/main",
    [string[]]$ChangedFiles = @(),
    [string]$Exe = "target/debug/arch-cli.exe",
    [string]$CorpusRoot = "tests/corpus",
    [string]$CorpusManifest = "eval/corpus.toml",
    [string]$OutJson = "out/drift-gate.json",
    [switch]$PlanOnly,
    [switch]$ForceFull,
    [switch]$RequireCorpus,
    # 负控自证（票 24 阶段 1）：故意注入一处结构违规，断言本脚本作为 CI 门禁必然判红、复原必然回绿。
    [switch]$SelfTest,
    [switch]$CorpusUnavailable
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $repoRoot

$selfTarget = "tests/fixtures/drift"
$selfBaseline = "tests/fixtures/drift/drift-baseline.json"

# ---------- -SelfTest：负控自证（票 24 阶段 1，headless）----------
#
# 为什么需要：`arch-cli drift` 的 gate 已被 rust 集成测试证红
# （`crates/arch-cli/tests/drift.rs::drift_gate_failure_exits_one_with_a_full_report`），
# 但**本脚本作为 CI 门禁**从未在故意注入下变红过一次。若有人把 `Invoke-DriftGate` 的状态映射
# 改成 `1 { "passed" }`，现有任何检查都不会报警——「一个从未变红过的门禁与坏掉的门禁，在证据上
# 长得一样」正是本票要消除的状态。
#
# 为什么父进程在这里提前接管：若把自测放在计划判定之后，`-PlanOnly` 与 not-required 分支会提前
# `exit 0`，自测整段被跳过却仍返回 0——那是「空跑也算过」的失效形态。父进程只做编排，真正的
# 门禁判定交给子进程跑**完整脚本**，走的是 `Invoke-DriftGate`、`$selfRequired` 与汇总退出的真实路径。
#
# 为什么每段子运行都要校验哨兵字样：子进程退出码不足以证明它真跑了 gate。故每次断言输出含
# `drift gate: base=`（主流程起点字样），否则按空跑失败计。
#
# 为什么脚本副本必须落在 eval/ 而不是 out/：副本自身会执行
# `$repoRoot = Split-Path -Parent $PSScriptRoot`；放到 out/<runId>/ 会把仓库根算成 run 目录，
# 相对基线与 `$Exe` 全部错位，产出的读数是假的。副本命名 `eval/tmp-driftself-*`，退出前删除并校验残留。
#
# 判据（与票 24 阶段 1 更正段一致）：
#   S1 绿基线     未改动的 fixture 副本 → 退出码 0 + `self: passed` + 报告 newViolations 为空
#   S2 注入必红   副本 `utils/format.ts` 追加一条指向 `server/api.ts` 的反向 import →
#                 退出码 1 + `self: FAILED` + 报告含 {kind=reverseEdge, source=utils/format.ts,
#                 target=server/api.ts}。**不断言条数恰为 1**：实测反向边会伴生 cycle。
#   S3 脚本聚合   同一次子运行必须给出脚本级判红字样 `drift gate: FAILED`，
#                 证明红来自本脚本的汇总，而不只是 arch-cli 自己非零。
#   S4 复原回绿   把被注入的文件按原文覆盖回去再跑 → 退出码 0 + `self: passed` + 无违规，
#                 并断言仓内基线文件 SHA256 前后一致（本自测对仓库内文件严格只读）。
#
# 退出码：0 = 四例全部符合；1 = 任一例不符合（门禁失效或本自测自身失真）；
#         2 = 前置条件不满足（缺 arch-cli / 缺 fixture / 缺基线 / 缺锚点），**绝不表示报警**。
if ($SelfTest) {
    $stExeAbs = Join-Path $repoRoot $Exe
    $stFixture = Join-Path $repoRoot "tests/fixtures/drift"
    $stBaselineAbs = Join-Path $repoRoot $selfBaseline
    if (-not (Test-Path -LiteralPath $stExeAbs)) {
        Write-Output "drift-gate selftest: PRECHECK FAIL 缺少 $Exe（先运行 cargo build -p arch-cli）。退出码 2 不表示报警。"
        exit 2
    }
    if (-not (Test-Path -LiteralPath $stFixture) -or -not (Test-Path -LiteralPath $stBaselineAbs)) {
        Write-Output "drift-gate selftest: PRECHECK FAIL fixture 或基线缺失。退出码 2 不表示报警。"
        exit 2
    }

    $lf = [string][char]10
    $stRunId = Get-Date -Format 'yyyyMMdd-HHmmss'
    $stOut = Join-Path $repoRoot ("out/drift-selftest-" + $stRunId)
    New-Item -ItemType Directory -Force -Path $stOut | Out-Null
    $greenDir = Join-Path $stOut "fixture-green"
    $injDir = Join-Path $stOut "fixture-injected"
    New-Item -ItemType Directory -Force -Path $greenDir, $injDir | Out-Null
    Write-Output ("drift-gate selftest: run 目录 {0}（每次新目录，遵守 ADR 0004）" -f $stOut)

    # 从「当前正在运行的这份脚本」取文本，而不是硬编码 eval/drift-gate.ps1：
    # 正常跑两者同一文件；但做削弱反证时（喂入一份被改坏的本脚本副本）必须让子进程继承同一处削弱，
    # 否则副本派生出的是未改坏的原件，反证会以假绿的方式失效。
    $gateText = [IO.File]::ReadAllText($PSCommandPath).Replace("`r`n", $lf)
    $assignAnchor = '$selfTarget = "tests/fixtures/drift"'
    if (-not $gateText.Contains($assignAnchor)) {
        Write-Output "drift-gate selftest: PRECHECK FAIL 找不到 `$selfTarget 赋值锚点（脚本结构已变，负控失效）"
        exit 2
    }
    $baselineHashBefore = (Get-FileHash -Path $stBaselineAbs -Algorithm SHA256).Hash

    $script:stCases = @()
    $script:stTmpCopies = @()

    function Add-StCase {
        param([string]$Id, [string]$Expect, [bool]$Ok, [string]$Detail)
        $mark = if ($Ok) { 'PASS' } else { 'FAIL' }
        $script:stCases += [pscustomobject]@{ id = $Id; ok = $Ok }
        Write-Output ("  [{0}] {1,-14} 期望: {2,-34} 实得: {3}" -f $mark, $Id, $Expect, $Detail)
    }

    function Invoke-GateChild {
        param([string]$Label, [string]$TargetDir)
        # 只重定向 self 目标到副本；`$selfBaseline` 保持指向仓内基线，以证明基线被只读使用。
        $copyAbs = [IO.Path]::GetFullPath(
            (Join-Path $repoRoot ("eval/tmp-driftself-" + $stRunId + "-" + $Label + ".ps1")))
        [IO.File]::WriteAllText($copyAbs, $gateText.Replace(
            $assignAnchor, ('$selfTarget = "' + ($TargetDir -replace '\\', '/') + '"')))
        $script:stTmpCopies += $copyAbs

        $summary = Join-Path (Join-Path $stOut $Label) "summary.json"
        $argv = @('-NoProfile', '-File', $copyAbs,
                  '-ChangedFiles', 'tests/fixtures/drift/utils/format.ts',
                  '-OutJson', $summary)
        # 完整消费管道：用 Select-Object -First 截断会提前终止子进程并污染 $LASTEXITCODE（见票 24 更正段）。
        $out = (& pwsh @argv 2>&1 | Out-String)
        $code = $LASTEXITCODE

        $rep = $null
        $repDir = Join-Path $stOut $Label
        $rf = @(Get-ChildItem -LiteralPath $repDir -File -Filter 'drift-gate-report-self-*.json' -ErrorAction SilentlyContinue)
        if ($rf.Count -gt 0) {
            try { $rep = Get-Content -LiteralPath $rf[0].FullName -Raw -Encoding utf8 | ConvertFrom-Json }
            catch { $rep = $null }
        }
        return [pscustomobject]@{
            code = $code; out = $out; report = $rep
            reached = ($out -match 'drift gate: base=')
        }
    }

    Copy-Item -Path (Join-Path $stFixture '*') -Destination $greenDir -Recurse -Force
    Copy-Item -Path (Join-Path $stFixture '*') -Destination $injDir -Recurse -Force

    # S1 绿基线
    $r1 = Invoke-GateChild -Label 'green' -TargetDir $greenDir
    $v1 = @($r1.report.newViolations)
    $o1 = ($r1.out -match 'self: passed')
    Add-StCase -Id 'S1 绿基线' -Expect 'exit 0 + self: passed + 0 违规' -Ok `
        ($r1.reached -and $r1.code -eq 0 -and $o1 -and $null -ne $r1.report -and $v1.Count -eq 0) `
        -Detail ("reached={0} exit={1} passed字样={2} newViolations={3}" -f $r1.reached, $r1.code, $o1, $v1.Count)

    # S2 注入必红 + S3 脚本聚合
    $injFile = Join-Path $injDir 'utils/format.ts'
    $injOrig = [IO.File]::ReadAllText($injFile)
    [IO.File]::WriteAllText($injFile, ('import { ping } from "../server/api";' + $lf + $injOrig))
    $r2 = Invoke-GateChild -Label 'injected' -TargetDir $injDir
    $v2 = @($r2.report.newViolations)
    $hit = @($v2 | Where-Object {
        $_.kind -eq 'reverseEdge' -and $_.source -eq 'utils/format.ts' -and $_.target -eq 'server/api.ts' })
    $o2 = ($r2.out -match 'self: FAILED')
    Add-StCase -Id 'S2 注入必红' -Expect 'exit 1 + 报告含该反向边' -Ok `
        ($r2.reached -and $r2.code -eq 1 -and $o2 -and $hit.Count -ge 1) `
        -Detail ("reached={0} exit={1} failed字样={2} 命中该边={3} newViolations={4}" -f $r2.reached, $r2.code, $o2, $hit.Count, $v2.Count)
    Write-Output ("      伴生违规构成（供核，不作断言）: " + (($v2 | ForEach-Object { $_.kind }) -join ','))
    $agg2 = ($r2.out -match 'drift gate: FAILED')
    Add-StCase -Id 'S3 脚本聚合判红' -Expect '脚本级 drift gate: FAILED + exit 1' -Ok `
        ($agg2 -and $r2.code -eq 1) `
        -Detail ("脚本级判红字样={0} exit={1}" -f $agg2, $r2.code)

    # S4 复原回绿 + 基线只读
    [IO.File]::WriteAllText($injFile, $injOrig)
    $r4 = Invoke-GateChild -Label 'restored' -TargetDir $injDir
    $v4 = @($r4.report.newViolations)
    $o4 = ($r4.out -match 'self: passed')
    $hashOk = ((Get-FileHash -Path $stBaselineAbs -Algorithm SHA256).Hash -eq $baselineHashBefore)
    Add-StCase -Id 'S4 复原回绿' -Expect 'exit 0 + 基线 SHA256 未变' -Ok `
        ($r4.reached -and $r4.code -eq 0 -and $o4 -and $v4.Count -eq 0 -and $hashOk) `
        -Detail ("reached={0} exit={1} passed字样={2} newViolations={3} 基线未变={4}" -f $r4.reached, $r4.code, $o4, $v4.Count, $hashOk)

    foreach ($p in $script:stTmpCopies) {
        Remove-Item -LiteralPath $p -Force -ErrorAction SilentlyContinue
    }
    $stLeft = @(Get-ChildItem -LiteralPath (Join-Path $repoRoot 'eval') -File -Filter 'tmp-driftself-*' -ErrorAction SilentlyContinue)
    Write-Output ("  临时脚本副本: 计划 {0} 个, 残留 {1} 个" -f $script:stTmpCopies.Count, $stLeft.Count)

    $stBad = @($script:stCases | Where-Object { -not $_.ok }).Count + $stLeft.Count
    Write-Output ("drift-gate selftest: verdict={0} cases={1} 不符合={2} 归档={3}" -f `
        $(if ($stBad -eq 0) { 'PASS' } else { 'FAIL' }), $script:stCases.Count, $stBad, $stOut)
    if ($stBad -gt 0) { exit 1 }
    exit 0
}

# engine：扫描 / 解析 / 分层 / 漂移 / CLI 行为。改了这些，两类基线的读数都可能变。
$enginePatterns = @(
    '^crates/arch-core/(src/|Cargo\.toml$)',
    '^crates/arch-cli/(src/|Cargo\.toml$)',
    '^Cargo\.lock$'
)
$selfPatterns = $enginePatterns + @(
    '^tests/fixtures/drift/',
    '^eval/drift-gate\.ps1$'
)
$corpusPatterns = $enginePatterns + @(
    '^eval/corpus\.toml$',
    '^eval/baseline/drift-[^/]+\.json$'
)
# 基线改动：动到钉住的语料 pin 或语料基线，必须由真实语料复核，不能降级跳过。
$baselinePatterns = @(
    '^eval/corpus\.toml$',
    '^eval/baseline/drift-[^/]+\.json$'
)

function Test-AnyPattern {
    param([string]$Path, [string[]]$Patterns)
    foreach ($pattern in $Patterns) {
        if ($Path -match $pattern) { return $true }
    }
    return $false
}

function Get-TrackedAndUntrackedChanges {
    param([string]$Ref)

    if ($Ref) {
        $mergeBase = (& git merge-base $Ref HEAD 2>$null)
        if ($LASTEXITCODE -ne 0 -or -not $mergeBase) {
            return @{ Known = $false; Files = @() }
        }
        $diff = @(& git diff --name-only --diff-filter=ACMRDTUXB $mergeBase 2>$null)
    } else {
        return @{ Known = $false; Files = @() }
    }

    $untracked = @(& git ls-files --others --exclude-standard 2>$null)
    $files = @($diff + $untracked | Where-Object { $_ } | Sort-Object -Unique)
    return @{ Known = $true; Files = $files }
}

function Get-CorpusRepos {
    param([string]$Manifest)
    if (-not (Test-Path -LiteralPath $Manifest)) { return @() }
    $text = Get-Content -LiteralPath $Manifest -Raw -Encoding utf8
    return @([regex]::Matches($text, '(?m)^name\s*=\s*"([^"]+)"') |
        ForEach-Object { $_.Groups[1].Value })
}

function Test-CorpusRepo {
    param([string]$Root, [string]$Name)
    $target = Join-Path $Root $Name
    return (Test-Path -LiteralPath $target) -and
        ((Get-ChildItem -LiteralPath $target -Force | Measure-Object).Count -gt 0)
}

# 计算计划：哪些门禁 required，以及语料是否可用。
$change = @{ Known = $true; Files = @() }
if ($ChangedFiles.Count -gt 0) {
    $change.Files = @($ChangedFiles | ForEach-Object { $_.Trim() } | Where-Object { $_ } |
        Sort-Object -Unique)
    $reason = "由 -ChangedFiles 指定"
} else {
    $change = Get-TrackedAndUntrackedChanges -Ref $BaseRef
    $reason = if ($change.Known) { "git diff $BaseRef...HEAD + 工作区" } else { "基线 $BaseRef 无法解析，保守全跑" }
}

$selfRequired = $false
$corpusRequired = $false
$baselineTouched = $false
if (-not $change.Known) {
    $selfRequired = $true
    $corpusRequired = $true
    # 基线无法解析时查不出是否动了基线，保守全跑但保持降级语义（不误杀）。
} else {
    foreach ($file in $change.Files) {
        if (Test-AnyPattern $file $selfPatterns) { $selfRequired = $true }
        if (Test-AnyPattern $file $corpusPatterns) { $corpusRequired = $true }
        if (Test-AnyPattern $file $baselinePatterns) { $baselineTouched = $true }
    }
}

# 手动触发没有可靠的 before/base 事件字段；CI 用 origin/<默认分支> 作为 diff 基线
# 以便仍能识别基线改动，同时用 -ForceFull 保持「dispatch 保守全跑」语义。
if ($ForceFull) {
    $selfRequired = $true
    $corpusRequired = $true
}

$corpusRepos = Get-CorpusRepos -Manifest $CorpusManifest
if ($CorpusUnavailable) {
    $corpusAvailable = @()
    $corpusMissing = @($corpusRepos)
} else {
    $corpusAvailable = @($corpusRepos | Where-Object { Test-CorpusRepo -Root $CorpusRoot -Name $_ })
    $corpusMissing = @($corpusRepos | Where-Object { $corpusAvailable -notcontains $_ })
}
$corpusUsable = ($corpusRepos.Count -gt 0) -and ($corpusMissing.Count -eq 0)

$outDir = Split-Path -Parent $OutJson
if ($outDir -and -not (Test-Path -LiteralPath $outDir)) {
    New-Item -ItemType Directory -Force -Path $outDir | Out-Null
}

$plan = [pscustomobject]@{
    schemaVersion  = 1
    reason         = $reason
    changedFiles   = @($change.Files)
    baselineTouched = $baselineTouched
    self           = [pscustomobject]@{ required = $selfRequired }
    corpus         = [pscustomobject]@{
        required  = $corpusRequired
        usable    = $corpusUsable
        repos     = @($corpusRepos)
        available = @($corpusAvailable)
        missing   = @($corpusMissing)
    }
}

if ($PlanOnly) {
    $plan | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $OutJson -Encoding utf8
    $cacheState = if ($CorpusUnavailable) { "unverified" } elseif ($corpusMissing.Count -eq 0) { "present" } else { "missing" }
    Write-Output ("plan: changed={0} self={1} corpus={2} corpusCache={3} baselineTouched={4}" -f `
            $change.Files.Count, $selfRequired, $corpusRequired, $cacheState, $baselineTouched)
    if ($selfRequired -or $corpusRequired) {
        Write-Output "required=true"
    } else {
        Write-Output "required=false"
    }
    if ($corpusRequired) {
        Write-Output "corpus_required=true"
    } else {
        Write-Output "corpus_required=false"
    }
    if ($baselineTouched) {
        Write-Output "baseline_touched=true"
    } else {
        Write-Output "baseline_touched=false"
    }
    exit 0
}

if (-not $selfRequired -and -not $corpusRequired) {
    Write-Output "drift gate: NOT REQUIRED（改动未触及 drift 输入面）"
    Write-Output ("  changed: {0}" -f (($change.Files | Select-Object -First 8) -join ", "))
    $summary = [pscustomobject]@{
        schemaVersion = 1
        generatedAt   = (Get-Date).ToString("o")
        baseRef       = $BaseRef
        plan          = $plan
        gates         = @()
        coverage      = [pscustomobject]@{
            verdict = "not-required"
            covered = @()
            missing = @()
            note    = "没有改动落在 drift 输入面，未执行任何漂移检测"
        }
        passed        = $true
    }
    $summary | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $OutJson -Encoding utf8
    Write-Output "coverage: not-required（未验任何 drift 门禁；改动不触及输入面）"
    exit 0
}

# 核心判据：基线/pin 改动必须由真实语料复核，语料不可用时直接判红，不降级成警告。
if ($baselineTouched -and -not $corpusUsable) {
    Write-Output "drift gate: FAILED（基线改动必须在有语料的环境验证）"
    Write-Output ("  baselineTouched: {0}" -f (($change.Files | Where-Object { Test-AnyPattern $_ $baselinePatterns }) -join ", "))
    if ($CorpusUnavailable) {
        Write-Output "  corpus: unavailable（pin 校验未通过或被 -CorpusUnavailable 强制视为缺失）"
    } elseif ($corpusRepos.Count -eq 0) {
        Write-Output ("  corpus: unavailable（pin 清单缺失或没有仓库条目: {0}）" -f $CorpusManifest)
    } else {
        Write-Output ("  corpus: unavailable（缺失: {0}；缓存根 {1}）" -f ($corpusMissing -join ", "), $CorpusRoot)
    }
    Write-Output "  修复: pwsh eval/fetch-corpus.ps1 && pwsh eval/drift-gate.ps1 -RequireCorpus"
    $summary = [pscustomobject]@{
        schemaVersion = 1
        generatedAt   = (Get-Date).ToString("o")
        baseRef       = $BaseRef
        plan          = $plan
        gates         = @()
        coverage      = [pscustomobject]@{
            verdict = "blocked"
            covered = @()
            missing = @($corpusRepos | ForEach-Object { "corpus/$_（基线改动需要语料验证）" })
            note    = "基线改动不能靠降级通过；语料不可用，未执行任何漂移检测"
        }
        passed        = $false
    }
    $summary | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $OutJson -Encoding utf8
    Write-Output "coverage: blocked（未验任何门禁；基线改动需要语料，语料不可用）"
    exit 1
}

if (-not (Test-Path -LiteralPath $Exe)) {
    Write-Output "drift gate: ERROR 缺少 $Exe；先运行 cargo build -p arch-cli"
    exit 2
}

if ($corpusRequired -and $corpusRepos.Count -eq 0) {
    Write-Output "drift gate: ERROR 语料 pin 清单缺失或没有仓库条目: $CorpusManifest"
    exit 2
}

$gates = @()
$covered = @()
$missingCoverage = @()
$runId = [Guid]::NewGuid().ToString('N').Substring(0, 8)

function Invoke-DriftGate {
    param([string]$Name, [string]$Target, [string]$Baseline, [string]$OutDir)

    if (-not (Test-Path -LiteralPath $Target)) {
        return [pscustomobject]@{
            name = $Name; target = $Target; baseline = $Baseline
            status = "error"; exitCode = 2; message = "目标不存在: $Target"
            checks = $null; newViolations = @(); stderr = ""
        }
    }
    if (-not (Test-Path -LiteralPath $Baseline)) {
        return [pscustomobject]@{
            name = $Name; target = $Target; baseline = $Baseline
            status = "error"; exitCode = 2; message = "基线不存在: $Baseline"
            checks = $null; newViolations = @(); stderr = ""
        }
    }

    $reportPath = Join-Path $OutDir ("drift-gate-report-" + ($Name -replace '[\\/:*?"<>|]', '-') + "-$runId.json")
    $stderrPath = Join-Path $OutDir ("drift-gate-stderr-" + ($Name -replace '[\\/:*?"<>|]', '-') + "-$runId.txt")
    & $Exe drift $Target --base $Baseline --out $reportPath 2>$stderrPath
    $code = $LASTEXITCODE
    $stderrText = ""
    if (Test-Path -LiteralPath $stderrPath) {
        $content = Get-Content -LiteralPath $stderrPath -Raw -ErrorAction SilentlyContinue
        if ($content) { $stderrText = $content }
    }
    Remove-Item -LiteralPath $stderrPath -ErrorAction SilentlyContinue

    $report = $null
    if (Test-Path -LiteralPath $reportPath) {
        try {
            $report = Get-Content -LiteralPath $reportPath -Raw -Encoding utf8 | ConvertFrom-Json
        } catch {
            $report = $null
        }
    }

    $checks = $null
    $stats = $null
    $violations = @()
    if ($null -ne $report) {
        $checks = $report.gate.checks
        $stats = $report.stats
        $violations = @($report.newViolations)
    }

    $status = switch ($code) {
        0 { "passed" }
        1 { "failed" }
        default { "error" }
    }

    return [pscustomobject]@{
        name         = $Name
        target       = $Target
        baseline     = $Baseline
        status       = $status
        exitCode     = $code
        message      = if ($status -eq "error" -and $null -eq $report) { "arch-cli 退出 $code" } else { "" }
        checks       = $checks
        stats        = $stats
        newViolations = $violations
        stderr       = ($stderrText.Trim())
        reportPath   = $reportPath
    }
}

# passed 分支的自证行：把本次实际比较的索引规模与四类判据计数一起打印。
# `stats` 来自 drift 报告（layers 产物的 files/modules/edges/unresolved），
# `checks` 是四类判据各自的 new/resolved 计数；两者都不参与门禁判定。
function Format-DriftPassed {
    param([pscustomobject]$Result, [string]$Baseline)

    if ($null -eq $Result.stats) {
        return ("modules=? edges=?；checks=?；基线 {0}" -f $Baseline)
    }
    $checks = $Result.checks
    return ("modules={0} edges={1} files={2} unresolved={3}；checks new/resolved: " +
        "moduleChanges={4}/{5} reverseEdges={6}/{7} cycles={8}/{9} externalLeaks={10}/{11}；基线 {12}") -f `
        $Result.stats.modules, $Result.stats.edges, $Result.stats.files, $Result.stats.unresolved,
        $checks.moduleChanges.newCount, $checks.moduleChanges.resolvedCount,
        $checks.reverseEdges.newCount, $checks.reverseEdges.resolvedCount,
        $checks.cycles.newCount, $checks.cycles.resolvedCount,
        $checks.externalLeaks.newCount, $checks.externalLeaks.resolvedCount,
        $Baseline
}

Write-Output ("drift gate: base={0} changed={1} ({2})" -f $BaseRef, $change.Files.Count, $reason)

if ($selfRequired) {
    Write-Output ("  self: required -> {0} (base {1})" -f $selfTarget, $selfBaseline)
    $result = Invoke-DriftGate -Name "self" -Target $selfTarget -Baseline $selfBaseline -OutDir $outDir
    $gates += $result
    if ($result.status -eq "passed") {
        $covered += "self"
        Write-Output ("self: passed（{0}）" -f (Format-DriftPassed -Result $result -Baseline $selfBaseline))
    } elseif ($result.status -eq "failed") {
        $covered += "self"
        Write-Output ("self: FAILED（{0} 条新增违规）" -f $result.newViolations.Count)
        $result.newViolations | ForEach-Object { Write-Output ("  " + ($_ | ConvertTo-Json -Compress -Depth 6)) }
        if ($result.stderr) { Write-Output ("  stderr: " + $result.stderr) }
    } else {
        Write-Output ("self: ERROR（{0}）" -f $result.message)
    }
} else {
    Write-Output "  self: not required（改动不在 self 输入面）"
}

if ($corpusRequired) {
    Write-Output "  corpus: required（外部语料基线）"
    if ($corpusMissing.Count -gt 0 -and -not ($corpusRepos | Where-Object { $corpusAvailable -contains $_ })) {
        if ($RequireCorpus) {
            Write-Output ("corpus: ERROR 语料缓存缺失（{0}），-RequireCorpus 要求 fail closed" -f $CorpusRoot)
            $gates += [pscustomobject]@{
                name = "corpus"; target = $CorpusRoot; baseline = ""; status = "error"; exitCode = 2
                message = "语料缓存缺失"; checks = $null; newViolations = @(); stderr = ""
            }
        } else {
            Write-Output ("corpus: skipped（语料不可用: {0}；未覆盖外部语料上的四类漂移回归）" -f ($corpusMissing -join ", "))
            $missingCoverage += ("corpus（语料不可用，未执行 {0} 漂移回归）" -f ($corpusRepos -join "/"))
        }
    } else {
        foreach ($repo in $corpusRepos) {
            $baseline = "eval/baseline/drift-$repo.json"
            if ($corpusAvailable -notcontains $repo) {
                if ($RequireCorpus) {
                    Write-Output ("corpus/{0}: ERROR 语料缓存缺失（{1}）" -f $repo, (Join-Path $CorpusRoot $repo))
                    $gates += [pscustomobject]@{
                        name = "corpus/$repo"; target = (Join-Path $CorpusRoot $repo); baseline = $baseline
                        status = "error"; exitCode = 2; message = "语料缓存缺失"
                        checks = $null; newViolations = @(); stderr = ""
                    }
                } else {
                    Write-Output ("corpus/{0}: skipped（语料缓存缺失，未覆盖该语料基线）" -f $repo)
                    $missingCoverage += "corpus/$repo（语料缓存缺失）"
                }
                continue
            }
            $result = Invoke-DriftGate -Name "corpus/$repo" -Target (Join-Path $CorpusRoot $repo) `
                -Baseline $baseline -OutDir $outDir
            $gates += $result
            if ($result.status -eq "passed") {
                $covered += "corpus/$repo"
                Write-Output ("corpus/{0}: passed（{1}）" -f $repo, (Format-DriftPassed -Result $result -Baseline $baseline))
            } elseif ($result.status -eq "failed") {
                $covered += "corpus/$repo"
                Write-Output ("corpus/{0}: FAILED（{1} 条新增违规）" -f $repo, $result.newViolations.Count)
                $result.newViolations | ForEach-Object { Write-Output ("  " + ($_ | ConvertTo-Json -Compress -Depth 6)) }
                if ($result.stderr) { Write-Output ("  stderr: " + $result.stderr) }
            } else {
                Write-Output ("corpus/{0}: ERROR（{1}）" -f $repo, $result.message)
            }
        }
    }
} else {
    Write-Output "  corpus: not required（改动不在 corpus 输入面）"
}

$failed = @($gates | Where-Object { $_.status -eq "failed" })
$errored = @($gates | Where-Object { $_.status -eq "error" })
$passed = ($failed.Count -eq 0) -and ($errored.Count -eq 0)

$verdict = if ($missingCoverage.Count -eq 0) { "complete" } else { "partial" }
$note = if ($verdict -eq "complete") {
    "所需门禁全部执行，无覆盖缺口"
} else {
    "部分门禁因语料不可用被跳过；见 missing"
}

$summary = [pscustomobject]@{
    schemaVersion = 1
    generatedAt   = (Get-Date).ToString("o")
    baseRef       = $BaseRef
    plan          = $plan
    gates         = $gates
    coverage      = [pscustomobject]@{
        verdict = $verdict
        covered = $covered
        missing = $missingCoverage
        note    = $note
    }
    passed        = $passed
}
$summary | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $OutJson -Encoding utf8
if ($verdict -eq "complete") {
    Write-Output ("coverage: complete（已验 {0}；无缺口）" -f ($covered -join ", "))
} else {
    Write-Output ("coverage: partial（已验 {0}；漏 {1}）" -f ($covered -join ", "), ($missingCoverage -join "; "))
}

if ($failed.Count -gt 0) {
    Write-Output ("drift gate: FAILED（判红: {0}；摘要 {1}）" -f ($failed.name -join ", "), $OutJson)
    exit 1
}
if ($errored.Count -gt 0) {
    Write-Output ("drift gate: ERROR（无法执行: {0}；摘要 {1}）" -f ($errored.name -join ", "), $OutJson)
    exit 2
}
Write-Output ("drift gate: PASSED（摘要 {0}）" -f $OutJson)
exit 0
