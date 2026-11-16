# PowerShell 参数面静态门禁（待决第 4 项落地，第 10 道门禁）
#
# 守的是 ENGINEERING-DISCIPLINE.md §5 第 7 条登记的**第九类取证坑**：
#   `Func $x -split $y` 会被 PowerShell 解析成「把 -split 当作 Func 的命名参数」，
#   语法完全合法、不报错、静默退化成单值。写这类代码的人拿到的不是异常，而是错的读数。
#
# 两条规则（均为 fail-closed）：
#   D1 undeclared-local-arg    调用仓库内定义的函数时，携带了该函数自身 param 未声明的命名参数。
#   D2 operator-as-argument    任何调用携带了「运算符关键字」形状的命名参数（-split / -replace / -eq …）。
#
# 三条实现约束（都是开发本门禁时实测踩到的，改动前请先读）：
#   1. 参数声明有**两种形式**，都必须建索引，否则 D1 会静默只覆盖一部分函数：
#        形式 A `function F { param([string]$P) }` → ParameterAst → ParamBlockAst → ScriptBlockAst → FunctionDefinitionAst
#        形式 B `function F([string]$P) { }`       → ParameterAst → FunctionDefinitionAst（**不经 ParamBlockAst**）
#      实测：只认形式 A 时索引 56/66 个函数，那 10 个（Median / P95 / Resolve-Exe …）永不触发 D1 且全绿。
#   2. 参数名必须取 `Name.VariablePath.UserPath`。`Name.Name` 返回**空串而不报错**，
#      `VariableName` 属性在本类型上不存在；用错会造出空声明表，使 D1 恒假——即"看起来在跑"。
#   3. 两遍法：先全库建表再检测。单遍会漏掉「定义在 A 文件、调用在 B 文件」的函数（lib-* 即此类）。
#
# 不检（登记为覆盖边界，见 docs/acceptance-gaps.md §8）：
#   - 外部命令与未收录名字（rg / git / 原生 cmdlet）：拿不到声明表，D1 不适用；D2 仍检。
#   - 双横线参数 `--foo`：其 ParameterName 自带前导 `-`，不会与运算符表相撞。
#   - 自身没有 param 的函数（decl 为空）：D1 按 `decl.Count -gt 0` 跳过。
#
# 退出码：0 干净；1 命中（有违规）；2 自检失败（无法解析全部脚本 / 索引为空 / 文件集为空）——
# 自检失败按"门禁失明"处理，绝不当作通过。

[CmdletBinding()]
param(
    # 仅注入自证用：显式给定待检文件集（相对仓库根），绕过 git ls-files 发现。
    [string[]]$FileList
)

$ErrorActionPreference = 'Stop'

# True when the file set comes from `git ls-files` (CI mode); false when -FileList was given.
$DiscoveryMode = -not ($FileList -and @($FileList).Count -gt 0)

$opKeywords = @(
    'split', 'join', 'replace', 'ireplace', 'creplace', 'match', 'notmatch',
    'ceq', 'ieq', 'ne', 'cne', 'ine', 'lt', 'gt', 'le', 'ge', 'like', 'notlike',
    'clike', 'inotlike', 'contains', 'notcontains', 'in', 'notin', 'band', 'bor',
    'bxor', 'bnot', 'is', 'isnot', 'as', 'f', 'shl', 'shr'
)
$opSet = @{}
foreach ($k in $opKeywords) { $opSet[$k.ToLowerInvariant()] = $true }

$root = (git rev-parse --show-toplevel).Trim()
if (-not $root) { "SELF-CHECK FAIL: cannot resolve repo root via git rev-parse"; exit 2 }

$FnMap = @{}      # function name (lower) -> declared parameter names (lower)
$FnSeen = @{}     # every function name seen, whether or not it declares params
$FormA = 0        # params indexed via form A (param() inside braces)
$FormB = 0        # params indexed via form B (positional params after the name)
$DupNames = 0     # names defined more than once (params are unioned, see below)

function Get-RelList {
    if ($FileList -and @($FileList).Count -gt 0) {
        return @($FileList | ForEach-Object { $_ -replace '\\', '/' })
    }
    $z = @(git -C $root ls-files '*.ps1')
    if ($LASTEXITCODE -ne 0) { throw "git ls-files failed with exit $LASTEXITCODE" }
    return @($z | ForEach-Object { $_ -replace '\\', '/' })
}

function Get-Abs([string]$rel) {
    # 读写同一个绝对路径：§5 第 7 条第八类坑（.NET 相对路径落在 .NET CWD，与 PS 位置不同源）。
    return [IO.Path]::GetFullPath((Join-Path $root $rel))
}

function Parse-Rel([string]$rel) {
    $abs = Get-Abs $rel
    if (-not (Test-Path -LiteralPath $abs)) { return $null }
    $toks = $null; $errs = $null
    $ast = [System.Management.Automation.Language.Parser]::ParseFile(
        $abs, [ref]$toks, [ref]$errs)
    if (@($errs).Count -gt 0) { return 'PARSE_ERROR' }
    return $ast
}

function Index-Ast($ast) {
    if ($null -eq $ast -or $ast -is [string]) { return }

    foreach ($fd in $ast.FindAll(
        { param($x) $x -is [System.Management.Automation.Language.FunctionDefinitionAst] }, $true)) {
        $script:FnSeen[$fd.Name.ToLowerInvariant()] = $true
    }

    foreach ($p in $ast.FindAll(
        { param($x) $x -is [System.Management.Automation.Language.ParameterAst] }, $true)) {

        $fd = $null
        if ($p.Parent -is [System.Management.Automation.Language.FunctionDefinitionAst]) {
            $fd = $p.Parent          # form B
            $script:FormB++
        }
        elseif ($p.Parent -is [System.Management.Automation.Language.ParamBlockAst] -and
                $p.Parent.Parent -is [System.Management.Automation.Language.ScriptBlockAst] -and
                $p.Parent.Parent.Parent -is [System.Management.Automation.Language.FunctionDefinitionAst]) {
            $fd = $p.Parent.Parent.Parent   # form A
            $script:FormA++
        }
        else {
            # Nested scriptblock param(), class methods, workflow params: not a function's own param.
            continue
        }

        $up = $null
        if ($p.Name -and $p.Name.VariablePath) { $up = $p.Name.VariablePath.UserPath }
        if (-not $up) { continue }        # never fall through to an empty name

        $key = $fd.Name.ToLowerInvariant()
        if (-not $script:FnMap.ContainsKey($key)) {
            $script:FnMap[$key] = (New-Object System.Collections.Generic.List[string])
        }
        if ($script:FnMap[$key].Contains($up.ToLowerInvariant())) { $script:DupNames++ }
        else { $script:FnMap[$key].Add($up.ToLowerInvariant()) }
    }
}

function Scan-Rel([string]$rel) {
    $ast = Parse-Rel $rel
    $out = New-Object System.Collections.Generic.List[object]
    if ($null -eq $ast -or $ast -is [string]) { return $out }

    foreach ($cmd in $ast.FindAll(
        { param($x) $x -is [System.Management.Automation.Language.CommandAst] }, $true)) {
        $name = $cmd.GetCommandName()
        if (-not $name) { continue }
        $key = $name.ToLowerInvariant()
        $known = $FnMap.ContainsKey($key)

        foreach ($el in $cmd.CommandElements) {
            if ($el -isnot [System.Management.Automation.Language.CommandParameterAst]) { continue }
            $raw = $el.ParameterName
            if (-not $raw) { continue }
            $pn = $raw.ToLowerInvariant()

            $d2 = $opSet.ContainsKey($pn)
            $d1 = $false
            if ($known) {
                $decl = $FnMap[$key]
                if ($decl.Count -gt 0 -and -not $decl.Contains($pn)) { $d1 = $true }
            }
            if ($d1 -or $d2) {
                $rules = @()
                if ($d1) { $rules += 'D1-undeclared-local-arg' }
                if ($d2) { $rules += 'D2-operator-as-argument' }
                $out.Add([pscustomobject]@{
                    file = $rel
                    line = $el.Extent.StartLineNumber
                    cmd  = $name
                    param = $raw
                    rules = ($rules -join '+')
                    text = $el.Extent.Text.Trim()
                })
            }
        }
    }
    return $out
}

# ---------- pass 1 ----------
$files = Get-RelList
"ps-param-lint: repo files to scan = {0}" -f @($files).Count
"ps-param-lint: mode = {0}" -f $(if ($DiscoveryMode) { 'discovery (git ls-files)' } else { 'subset (-FileList)' })
if (@($files).Count -eq 0) { "SELF-CHECK FAIL(coverage-empty-fileset): empty file set (gate blind)"; exit 2 }

$unparseable = @()
foreach ($f in $files) {
    $abs = Get-Abs $f
    if (-not (Test-Path -LiteralPath $abs)) { $unparseable += "$f (missing)"; continue }
    $a = Parse-Rel $f
    if ($a -is [string]) { $unparseable += "$f (parse error)"; continue }
    if ($null -eq $a) { $unparseable += "$f (unreadable)"; continue }
    Index-Ast $a
}

# Fail closed on anything we could not parse: a skipped file is a blind spot, not a pass.
# Applies in both modes - per-file verifiability is not a subset-dependent notion.
if (@($unparseable).Count -gt 0) {
    "SELF-CHECK FAIL(blind-unparsed): {0} file(s) not parsed" -f @($unparseable).Count
    foreach ($u in $unparseable) { "  BLIND {0}" -f $u }
    exit 2
}

$noParamFns = @($FnSeen.Keys | Where-Object { -not $FnMap.ContainsKey($_) })
"index: functions seen = {0}  with own params = {1}  without params = {2}" -f `
    $FnSeen.Count, $FnMap.Count, $noParamFns.Count
"index: param slots by form: A={0} B={1}  duplicate-name param slots={2}" -f $FormA, $FormB, $DupNames

# Coverage invariants below apply ONLY to the whole-repo discovery mode.
# Reason (found by testing, not by reading): an explicit -FileList may legitimately name a
# single probe file that contains just one declaration form, or no function at all. Applying
# these there turns every injection case into exit 2 for the wrong reason, which would mask
# whether D1/D2 actually fired. The CI mode always sees both forms present in this repo, so the
# regression guard stays live where it matters.
if ($DiscoveryMode) {
    # Both declaration forms must be represented, otherwise the walker silently lost one.
    # Regression guard for the exact defect found while building this gate (56/66 indexed).
    if ($FormA -eq 0 -or $FormB -eq 0) {
        "SELF-CHECK FAIL(coverage-form-index): one param-declaration form is unindexed (A={0} B={1}); D1 coverage incomplete" -f $FormA, $FormB
        exit 2
    }
    if ($FnMap.Count -eq 0) {
        "SELF-CHECK FAIL(coverage-empty-index): no function declarations indexed (gate blind)"
        exit 2
    }
} else {
    "index: coverage invariants skipped (explicit -FileList subset, discovery mode off)"
}

# ---------- pass 2 ----------
$hits = @()
foreach ($f in $files) { $hits += @(Scan-Rel $f) }

foreach ($h in $hits) {
    "HIT {0}:{1} cmd={2} param={3} rules={4} text=[{5}]" -f `
        $h.file, $h.line, $h.cmd, $h.param, $h.rules, $h.text
}

$d1 = @($hits | Where-Object { $_.rules -like '*D1*' }).Count
$d2 = @($hits | Where-Object { $_.rules -like '*D2*' }).Count
"VERDICT files={0} functions={1} hits={2} D1={3} D2={4}" -f `
    @($files).Count, $FnMap.Count, @($hits).Count, $d1, $d2

if (@($hits).Count -gt 0) {
    "RESULT: FAIL ({0} violation(s)) - operator may be swallowed as argument, or a call passes an undeclared parameter" -f @($hits).Count
    exit 1
}
"RESULT: PASS"
exit 0
