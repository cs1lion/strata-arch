# 文档引用一致性检查（票 26）。
#
# 把 ENGINEERING-DISCIPLINE.md §5 第 7 条「登记类数字必须先脚本回读」做成机器检查：
# 登记表逐条声明「文档中的字面 ↔ 源数据字段/算式」，本工具读源、规范化、再断言文档相符。
#
# 硬约束（票 26 判据，不可在实现中放宽）：
#   - 文档取证一律走 `rg`，正则用 `--pcre2`、字面用 `-F`；**禁止**用 PowerShell 字符串匹配中文内容
#     （git／外部命令的中文输出经管道按系统码页解码会变乱码，使 `-match`／`Select-String` 恒 0 命中）。
#   - `derived` 类登记源字段与算式，不登记结果数字；结果必须由本工具现算。
#   - SKIP 不计入通过，每次输出必须打印 SKIP 条数；`-RequireAllResolved` 下 SKIP>0 判失败。
#   - 通过只声称「已登记项一致」，不声称「所有文档数字可信」。
#
# 实现注意：PowerShell 的 `switch` 块内 `continue` **不会**跳向外层 foreach（实测：同一条目会既计 PASS 又计 FAIL）。
# 因此本工具不用 continue 控制流程，改为「每个条目算出一个判定结果，统一在末尾结算」。
#
# 用法：
#   pwsh -File eval/cite-check.ps1                        # 校验登记表
#   pwsh -File eval/cite-check.ps1 -SelfTest              # 工具自身 negative control：三种坏登记必须全被抓
#   pwsh -File eval/cite-check.ps1 -RequireAllResolved    # 要求所有条目源可见
# 退出码：0 = 无 FAIL（且开关条件满足）；1 = 有 FAIL / 自测未全抓 / RequireAllResolved 下存在 SKIP。

[CmdletBinding()]
param(
    [string]$Registry = "eval/baseline/citations.json",
    [switch]$SelfTest,
    [switch]$RequireAllResolved
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

function Get-RgCount {
    # 返回匹配行数；rg 无命中时无输出且退出码 1，统一折算成 0（不依赖 rg -q 的假布尔）。
    param([string]$Pattern, [string]$Path, [switch]$Fixed)
    if (-not (Test-Path -LiteralPath $Path)) { return $null }
    $rgArgs = @('-c')
    if ($Fixed) { $rgArgs += '-F' } else { $rgArgs += @('--pcre2', '--no-heading') }
    $rgArgs += @($Pattern, $Path)
    $out = & rg @rgArgs 2>$null
    if ($LASTEXITCODE -eq 1) { return 0 }
    if ($LASTEXITCODE -ne 0) { return $null }
    $first = @($out | Where-Object { $_ -and "$_".Trim() -ne '' }) | Select-Object -First 1
    if (-not $first) { return 0 }
    $n = 0
    if (-not [int]::TryParse("$first".Trim(), [ref]$n)) { return $null }
    return $n
}

function Format-CiteValue {
    # 数值 → 文档字面。signed3 的负号用 U+2212（文档实测写 −0.622，而非 ASCII -0.622）。
    param([double]$Value, [string]$Format)
    switch ($Format) {
        'signed3' {
            $s = [math]::Round($Value, 3).ToString('0.000', [System.Globalization.CultureInfo]::InvariantCulture)
            return ($s -replace '-', [string][char]0x2212)
        }
        'plain2'  { return [math]::Round($Value, 2).ToString('0.00', [System.Globalization.CultureInfo]::InvariantCulture) }
        'plain3'  { return [math]::Round($Value, 3).ToString('0.000', [System.Globalization.CultureInfo]::InvariantCulture) }
        'plain'   { return $Value.ToString([System.Globalization.CultureInfo]::InvariantCulture) }
        default   { throw "未知 format: $Format" }
    }
}

function Get-JsonField {
    param($Object, [string]$Path)
    $cur = $Object
    foreach ($seg in ($Path -split '\.')) {
        if ($null -eq $cur) { return $null }
        if ($seg -match '^\d+$') { $cur = $cur[[int]$seg] } else { $cur = $cur.$seg }
    }
    return $cur
}

function Resolve-CiteSourceValue {
    # 由源数据算出该条登记的「应有字面」；返回 @{ ok; value; why }
    param($Cite, $RegistryObject)
    switch ($Cite.kind) {
        'json-field' {
            if (-not (Test-Path -LiteralPath $Cite.source)) { return @{ ok = $false; why = "源不存在 $($Cite.source)" } }
            $src = Get-Content -LiteralPath $Cite.source -Raw | ConvertFrom-Json
            $node = $src
            if ($Cite.selector) {
                $hits = @($src.groups | Where-Object { $_.batch -eq $Cite.selector.batch -and $_.variant -eq $Cite.selector.variant })
                if ($hits.Count -ne 1) { return @{ ok = $false; why = "selector 命中 $($hits.Count) 组（应为 1）" } }
                $node = $hits[0]
            }
            $v = Get-JsonField -Object $node -Path $Cite.field
            if ($null -eq $v) { return @{ ok = $false; why = "字段 $($Cite.field) 取不到" } }
            return @{ ok = $true; value = (Format-CiteValue -Value ([double]$v) -Format $Cite.format) }
        }
        'derived' {
            $vals = @()
            foreach ($s in $Cite.sources) {
                if (-not (Test-Path -LiteralPath $s.path)) { return @{ ok = $false; why = "源不存在 $($s.path)" } }
                $j = Get-Content -LiteralPath $s.path -Raw | ConvertFrom-Json
                $v = Get-JsonField -Object $j -Path $s.field
                if ($null -eq $v) { return @{ ok = $false; why = "字段 $($s.field) 取不到" } }
                $vals += [double]$v
            }
            switch ($Cite.formula) {
                'pct_change' {
                    if ($vals[0] -eq 0) { return @{ ok = $false; why = '除数为 0' } }
                    $p = [math]::Round(($vals[1] / $vals[0] - 1) * 100, 1)
                    $sign = if ($p -ge 0) { '+' } else { [string][char]0x2212 }
                    return @{ ok = $true; value = $sign + [math]::Abs($p).ToString('0.0', [System.Globalization.CultureInfo]::InvariantCulture) + '%' }
                }
                'diff' { return @{ ok = $true; value = (Format-CiteValue -Value ($vals[1] - $vals[0]) -Format 'signed3') } }
                default { return @{ ok = $false; why = "未知 formula $($Cite.formula)" } }
            }
        }
        'sha' {
            git rev-parse --verify "$($Cite.ref)^{commit}" > $null 2>&1
            if ($LASTEXITCODE -ne 0) { return @{ ok = $false; why = "SHA 不可解析 $($Cite.ref)" } }
            return @{ ok = $true; value = $Cite.ref }
        }
        'file-exists' {
            if (-not (Test-Path -LiteralPath $Cite.source)) { return @{ ok = $false; why = "路径不存在 $($Cite.source)" } }
            return @{ ok = $true; value = $Cite.source }
        }
        default { return @{ ok = $false; why = "未知 kind $($Cite.kind)" } }
    }
}

function Invoke-CiteCheck {
    param([object[]]$Items, [string]$Label)

    $pass = 0; $fail = 0; $skip = 0
    $lines = @()
    $failIds = @()

    foreach ($c in $Items) {
        $verdict = $null   # 'PASS' | 'FAIL' | 'SKIP'
        $note = ''

        if ($c.kind -eq 'pattern-count') {
            # 该类断言就是「某正则/字面在文档中命中的行数」，不涉及源值比对。
            if (-not (Test-Path -LiteralPath $c.doc)) {
                $verdict = if ($c.ciVisible -eq $false) { 'SKIP' } else { 'FAIL' }; $note = "文档不存在 $($c.doc)"
            } else {
                $n = Get-RgCount -Pattern $c.needle -Path $c.doc -Fixed:([bool]$c.literal)
                if ($n -eq $null) {
                    $verdict = 'FAIL'; $note = 'rg 执行异常'
                } elseif ($n -ne $c.expect) {
                    $verdict = 'FAIL'; $note = ("命中 {0} 行，期望 {1}（{2}）" -f $n, $c.expect, $c.doc)
                } else {
                    $verdict = 'PASS'
                }
            }
        }
        else {
            $r = Resolve-CiteSourceValue -Cite $c -RegistryObject $null
            if (-not $r.ok) {
                $verdict = if ($c.ciVisible -eq $false) { 'SKIP' } else { 'FAIL' }
                $note = $r.why
            }
            elseif ("$($c.needle)" -ne "$($r.value)") {
                # 登记表自身写错：登记字面与源算值不符。
                $verdict = 'FAIL'; $note = ("登记字面 [{0}] 与源算值 [{1}] 不符（kind={2}）" -f $c.needle, $r.value, $c.kind)
            }
            else {
                $hit = Get-RgCount -Pattern $c.needle -Path $c.doc -Fixed:$true
                if ($hit -eq $null) { $verdict = 'FAIL'; $note = 'rg 执行异常' }
                elseif ($hit -lt 1) { $verdict = 'FAIL'; $note = ("文档未出现登记字面 [{0}]（{1}，源算值 [{2}]）" -f $c.needle, $c.doc, $r.value) }
                else { $verdict = 'PASS' }
            }
        }

        switch ($verdict) {
            'PASS' { $pass++ }
            'SKIP' { $skip++; $lines += ("SKIP  {0}  {1}" -f $c.id, $note) }
            default {
                $fail++
                if ($verdict -eq 'FAIL') { $failIds += $c.id }
                $lines += ("FAIL  {0}  {1}" -f $c.id, $note)
            }
        }
    }

    if ($Label -eq 'registry' -and $RequireAllResolved -and $skip -gt 0) {
        $lines += ("  RequireAllResolved 已启用：存在 {0} 条 SKIP ⇒ 整体判失败" -f $skip)
        $fail += 1
    }
    # summary 必须在 RequireAllResolved 结算之后格式化，否则会出现「summary 行 FAIL=0、合计行 FAIL=1」的自相矛盾。
    $lines += ("[{0}] 条目={1} PASS={2} FAIL={3} SKIP={4}" -f $Label, @($Items).Count, $pass, $fail, $skip)
    return [pscustomobject]@{ pass = $pass; fail = $fail; skip = $skip; failIds = $failIds; lines = @($lines) }
}

if ($SelfTest) {
    if (-not (Test-Path -LiteralPath $Registry)) { Write-Output "selftest: 登记表缺失 $Registry"; exit 1 }
    $reg = Get-Content -LiteralPath $Registry -Raw | ConvertFrom-Json
    $items = @($reg.citations)
    $bad = @()
    $want = @()

    $jf = @($items | Where-Object { $_.kind -eq 'json-field' -and $_.ciVisible }) | Select-Object -First 1
    if ($jf) {
        $b = $jf | ConvertTo-Json -Depth 8 | ConvertFrom-Json
        $b.needle = '−9.999'
        $bad += $b; $want += $b.id
    }
    $pc = @($items | Where-Object { $_.kind -eq 'pattern-count' }) | Select-Object -First 1
    if ($pc) {
        $b = $pc | ConvertTo-Json -Depth 8 | ConvertFrom-Json
        $b.expect = $pc.expect + 1
        $bad += $b; $want += $b.id
    }
    $dv = @($items | Where-Object { $_.kind -eq 'derived' }) | Select-Object -First 1
    if ($dv) {
        $b = $dv | ConvertTo-Json -Depth 8 | ConvertFrom-Json
        $b.needle = '+88.8%'
        $bad += $b; $want += $b.id
    }
    if ($bad.Count -lt 3) {
        Write-Output ("selftest: 无法构造三类坏登记（需 json-field / pattern-count / derived 各 1），实得 {0}" -f $bad.Count)
        exit 1
    }
    $r = Invoke-CiteCheck -Items $bad -Label 'selftest'
    $r.lines | ForEach-Object { Write-Output $_ }
    $missed = @($want | Where-Object { $r.failIds -notcontains $_ })
    Write-Output ("  预置坏登记 {0} 处，被抓 id={1}，漏抓={2}" -f $want.Count, (@($r.failIds) -join ','), (@($missed) -join ','))
    if ($missed.Count -eq 0) {
        Write-Output ("selftest: PASS（{0} 处坏登记全部被抓）" -f $want.Count)
        exit 0
    }
    Write-Output "selftest: FAIL（有坏登记未被抓到 ⇒ 工具自身无效，不得声称已守护）"
    exit 1
}

if (-not (Test-Path -LiteralPath $Registry)) {
    Write-Output "cite-check: 登记表不存在 $Registry"
    exit 1
}
$reg = Get-Content -LiteralPath $Registry -Raw | ConvertFrom-Json
Write-Output ("cite-check: registry={0} 条目={1}" -f $Registry, @($reg.citations).Count)
Write-Output ("说明: {0}" -f $reg.note)
$res = Invoke-CiteCheck -Items @($reg.citations) -Label 'registry'
Write-Output ""
$res.lines | ForEach-Object { Write-Output $_ }
Write-Output ("合计 PASS={0} FAIL={1} SKIP={2}（SKIP 不计入通过；未登记项不受保护）" -f $res.pass, $res.fail, $res.skip)
if ($res.fail -gt 0) { exit 1 }
exit 0
