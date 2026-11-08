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
    [switch]$RequireCorpus,
    [switch]$CorpusUnavailable
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $repoRoot

$selfTarget = "tests/fixtures/drift"
$selfBaseline = "tests/fixtures/drift/drift-baseline.json"

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
    $violations = @()
    if ($null -ne $report) {
        $checks = $report.gate.checks
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
        newViolations = $violations
        stderr       = ($stderrText.Trim())
        reportPath   = $reportPath
    }
}

Write-Output ("drift gate: base={0} changed={1} ({2})" -f $BaseRef, $change.Files.Count, $reason)

if ($selfRequired) {
    Write-Output ("  self: required -> {0} (base {1})" -f $selfTarget, $selfBaseline)
    $result = Invoke-DriftGate -Name "self" -Target $selfTarget -Baseline $selfBaseline -OutDir $outDir
    $gates += $result
    if ($result.status -eq "passed") {
        $covered += "self"
        Write-Output ("self: passed（四类 newCount=0；基线 {0}）" -f $selfBaseline)
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
                Write-Output ("corpus/{0}: passed（四类 newCount=0）" -f $repo)
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
