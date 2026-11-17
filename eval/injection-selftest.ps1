# 元测试（injection selftest）：断言我们的检查器确实抓得住破坏，而不是只断言「当前全绿」。
#
# 为什么需要它：eval/cite-check.ps1 与 eval/paired-ab-selftest.ps1 只能证明「现状一致」，
# 不能证明「检查器有效」。若有人（或未来重构）悄悄削弱了检查器——例如追加一行
# `$invariantOk = $true`、把 Get-RgCount 改成恒返 1、或清空 -SelfTest 的 failIds——
# 现有登记项与逐字段比对都无法报警，CI 反而更绿。
#
# 核心设计（2026-11-16 实验结论，见本文件末尾注释与本仓库提交 445c5b6 之后的 §8 第 17 条）：
#   语句存在性守护抓不到「叠加式短路」，因此本元测试的抓手是行为断言：
#   「对活路径施加已知漏计后，检查器必须变红」。健全代码必然变红；被短路则不变红 ⇒ 本测试失败。
#
# 手法约束（纪律 §5 第 7 条，本会话实测过的坑）：
#   1. 注入前必须回读锚点行号与原文，注入后必须回读改动确已落盘——否则退出码是假证据。
#   2. 路径一律绝对化：.NET File API 按 .NET CurrentDirectory 解析相对路径，而 Set-Location
#      不改变它，混用会造出「写 A 跑 B」的不自洽读数（本会话真实踩过）。
#   3. 副本行尾归一 LF：Set-Content 在 Windows 写 CRLF，会让带 `$` 尾锚的正则静默失配。
#   4. 断言顺序：先看「差异是否出现」，最后才看退出码；只比退出码不算通过。
#   5. 异常路径也必须清理临时文件（try/finally）。
#
# 退出码：0 = 全部用例行为符合预期；1 = 有用例不符合（检查器失效或本测试自身失真）。

[CmdletBinding()]
param(
    [int]$DropOccurrence = 1,
    # 受试脚本（默认为入仓 paired-ab-selftest）。元测试自身的有效性验证靠它：
    # 把一个「已叠加短路」的基准副本传进来，C2 必须变红——若仍报绿，说明元测试是摆设。
    [string]$PassScript = 'eval/paired-ab-selftest.ps1'
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$script:tmpFiles = @()
function AbsNew { param([string]$rel) return [IO.Path]::GetFullPath((Join-Path $root $rel)) }
function New-Tmp { param([string]$rel)
    $p = AbsNew $rel
    $script:tmpFiles += $p
    return $p
}
function Read-Text { param([string]$P) return [IO.File]::ReadAllText($P) }
function Write-Text { param([string]$P, [string]$T) [IO.File]::WriteAllText($P, $T.Replace("`r`n", "`n")) }

function Invoke-Capture { param([string]$ScriptPath, [string[]]$ArgList = @())
    $out = & pwsh -NoProfile -File $ScriptPath @ArgList 2>&1 | Out-String
    return @{ code = $LASTEXITCODE; out = $out }
}
function Find-FirstIndex { param([string[]]$Lines, [string]$Pattern, [int]$StartAfter = -1)
    for ($i = $StartAfter + 1; $i -lt $Lines.Count; $i++) { if ($Lines[$i] -match $Pattern) { return $i } }
    return -1
}

function Add-Case { param([string]$Id, [string]$Expect, [bool]$Ok, [string]$Detail)
    $script:cases += [pscustomobject]@{ id = $Id; expect = $Expect; ok = $Ok; detail = $Detail }
    $mark = if ($Ok) { 'PASS' } else { 'FAIL' }
    Write-Output ("  [{0}] {1,-26} 期望: {2,-40} 实得: {3}" -f $mark, $Id, $Expect, $Detail)
}

$script:cases = @()
$failures = 0

try {
    # ---------- 准备：定位活路径注入锚点 ----------
    $passRel = $PassScript
    # 注意：`Read-Text $p -split "x"` 会被解析成向函数传名为 -split 的参数（不是数组运算），
    # 必须先括住函数调用再做 -split。
    $passLines = @((Read-Text (AbsNew $passRel)) -split "`n")
    $loopIdx = Find-FirstIndex -Lines $passLines -Pattern 'foreach \(\$sp in \$SidePairs\)'
    $dropIdx = Find-FirstIndex -Lines $passLines -Pattern '^\s+\$Checked\.Value\+\+' -StartAfter $loopIdx
    if ($loopIdx -lt 0 -or $dropIdx -lt 0) { throw "活路径注入锚点定位失败 loop=$loopIdx drop=$dropIdx" }
    Write-Output ("锚点回读: PC1 循环行={0} ; 活路径计数行={1} 原文=[{2}]" -f ($loopIdx + 1), ($dropIdx + 1), $passLines[$dropIdx].Trim())

    $invAnchor = '^\$invariantOk = \(\$checked -eq \$expectedChecks\)'
    $invIdx = Find-FirstIndex -Lines $passLines -Pattern $invAnchor
    if ($invIdx -lt 0) { throw '不变量赋值锚点定位失败' }
    Write-Output ("           不变量赋值行={0} 原文=[{1}]" -f ($invIdx + 1), $passLines[$invIdx].Trim())

    # ---------- C1 基线：两个检查器当前都应为绿（否则后续红/绿对照失去意义） ----------
    $ccRel = 'eval/cite-check.ps1'
    $b1 = Invoke-Capture -ScriptPath (AbsNew $ccRel)
    Add-Case -Id 'C1a cite-check 基线' -Expect '退出码 0' -Ok ($b1.code -eq 0) -Detail "退出码 $($b1.code)"
    $b2 = Invoke-Capture -ScriptPath (AbsNew $ccRel) -ArgList @('-SelfTest')
    Add-Case -Id 'C1b -SelfTest 基线' -Expect '退出码 0' -Ok ($b2.code -eq 0) -Detail "退出码 $($b2.code)"
    $b3 = Invoke-Capture -ScriptPath (AbsNew $passRel)
    Add-Case -Id 'C1c paired-ab-selftest 基线' -Expect '退出码 0' -Ok ($b3.code -eq 0) -Detail "退出码 $($b3.code)"

    # ---------- C2 漏计注入必须变红（本元测试抓短路的抓手） ----------
    $c2Path = New-Tmp 'eval/tmp-meta-c2-pass.ps1'
    $l2 = @((Read-Text (AbsNew $passRel)) -split "`n")
    $l2[$dropIdx] = '            # (meta) 活路径漏计注入：删除一次 $Checked.Value++'
    Write-Text $c2Path ($l2 -join "`n")
    $v2 = @((Read-Text $c2Path) -split "`n")
    if ($v2[$dropIdx].Trim() -notlike '# (meta) 活路径漏计注入*') { throw 'C2 注入未落盘（回读失败）' }
    $r2 = Invoke-Capture -ScriptPath $c2Path
    Add-Case -Id 'C2 漏计注入必须被抓住' -Expect '退出码 1（健全代码必报警）' -Ok ($r2.code -eq 1) -Detail "退出码 $($r2.code)"

    # ---------- C3 短路注入：证明语句守护看不见，而 C2 能看见 ----------
    $c3Path = New-Tmp 'eval/tmp-meta-c3-pass.ps1'
    $l3 = @((Read-Text (AbsNew $passRel)) -split "`n")
    $l3[$dropIdx] = '            # (meta) 活路径漏计注入'
    $n3 = @(); for ($i = 0; $i -lt $l3.Count; $i++) { $n3 += $l3[$i]; if ($i -eq $invIdx) { $n3 += '$invariantOk = $true   # (meta) 叠加式短路' } }
    Write-Text $c3Path ($n3 -join "`n")
    $v3 = @((Read-Text $c3Path) -split "`n")
    if ($v3[$invIdx + 1].Trim() -notlike '$invariantOk = $true*') { throw 'C3 短路注入未落盘' }
    if ($v3[$invIdx].Trim() -notlike '$invariantOk = ($checked*') { throw 'C3 原语句被误删（应保留）' }
    $r3 = Invoke-Capture -ScriptPath $c3Path
    Add-Case -Id 'C3 短路后漏计逃逸(预期不报)' -Expect '退出码 0（证明只靠语句不可见）' -Ok ($r3.code -eq 0) -Detail "退出码 $($r3.code)"

    # C3 的登记项侧证据：现有 pattern-count 对短路副本仍全绿 ⇒ 该削弱对语句守护不可见
    $c3regPath = New-Tmp 'eval/tmp-meta-c3-reg.json'
    $j3 = Get-Content -LiteralPath (AbsNew 'eval/baseline/citations.json') -Raw | ConvertFrom-Json
    foreach ($c in $j3.citations) { if ($c.doc -eq $passRel) { $c.doc = 'eval/tmp-meta-c3-pass.ps1' } }
    Write-Text $c3regPath ($j3 | ConvertTo-Json -Depth 10)
    $r3reg = Invoke-Capture -ScriptPath (AbsNew $ccRel) -ArgList @('-Registry', $c3regPath)
    Add-Case -Id 'C3b 登记项看短路副本应仍全绿' -Expect '退出码 0（守护天花板留痕）' -Ok ($r3reg.code -eq 0) -Detail "退出码 $($r3reg.code)"

    # ---------- C4 掏空 Get-RgCount 必须被现有负向项抓住 ----------
    $c4Path = New-Tmp 'eval/tmp-meta-c4-cc.ps1'
    $raw4 = Read-Text (AbsNew $ccRel)
    $anchor4 = '    if ($LASTEXITCODE -eq 1) { return 0 }'
    if (-not $raw4.Contains($anchor4)) { throw 'C4 锚点缺失（Get-RgCount 早退分支已变动）' }
    $raw4b = $raw4.Replace($anchor4, '    if ($true) { return 1 }   # (meta) 掏空 rg 早退')
    Write-Text $c4Path $raw4b
    if (-not (Read-Text $c4Path).Contains('if ($true) { return 1 }')) { throw 'C4 掏空未落盘' }
    $c4regPath = New-Tmp 'eval/tmp-meta-c4-reg.json'
    $j4 = Get-Content -LiteralPath (AbsNew 'eval/baseline/citations.json') -Raw | ConvertFrom-Json
    foreach ($c in $j4.citations) { if ($c.doc -eq $ccRel) { $c.doc = 'eval/tmp-meta-c4-cc.ps1' } }
    Write-Text $c4regPath ($j4 | ConvertTo-Json -Depth 10)
    $r4 = Invoke-Capture -ScriptPath $c4Path -ArgList @('-Registry', $c4regPath)
    $r4base = Invoke-Capture -ScriptPath (AbsNew $ccRel) -ArgList @('-Registry', $c4regPath)
    Add-Case -Id 'C4 掏空rg必须被负向项抓' -Expect '掏空退出码1 且 未掏空对照0' -Ok ($r4.code -eq 1 -and $r4base.code -eq 0) -Detail "掏空 $($r4.code) / 对照 $($r4base.code)"

    # ---------- C5 掏空 -SelfTest 的 failIds 必须变红 ----------
    $c5Path = New-Tmp 'eval/tmp-meta-c5-cc.ps1'
    $raw5 = Read-Text (AbsNew $ccRel)
    $anchor5 = '    $missed = @($want | Where-Object { $r.failIds -notcontains $_ })'
    if (-not $raw5.Contains($anchor5)) { throw 'C5 锚点缺失（-SelfTest 漏抓判定已变动）' }
    $raw5b = $raw5.Replace($anchor5, ('    $r.failIds = @()   # (meta) 掏空 failIds' + "`n" + $anchor5))
    Write-Text $c5Path $raw5b
    if (-not (Read-Text $c5Path).Contains('掏空 failIds')) { throw 'C5 掏空未落盘' }
    $r5 = Invoke-Capture -ScriptPath $c5Path -ArgList @('-SelfTest')
    Add-Case -Id 'C5 掏空SelfTest必须变红' -Expect '退出码 1' -Ok ($r5.code -eq 1) -Detail "退出码 $($r5.code)"

    # ---------- C6 抬高 expect 必须变红（反向哨兵：不能永远红） ----------
    $c6regPath = New-Tmp 'eval/tmp-meta-c6-reg.json'
    $j6 = Get-Content -LiteralPath (AbsNew 'eval/baseline/citations.json') -Raw | ConvertFrom-Json
    $t6 = @($j6.citations | Where-Object { $_.kind -eq 'pattern-count' -and $_.expect -eq 1 }) | Select-Object -First 1
    if (-not $t6) { throw 'C6 无可抬高的 pattern-count 项' }
    $t6.expect = 2
    Write-Text $c6regPath ($j6 | ConvertTo-Json -Depth 10)
    $r6 = Invoke-Capture -ScriptPath (AbsNew $ccRel) -ArgList @('-Registry', $c6regPath)
    Add-Case -Id 'C6 抬高expect必须变红' -Expect '退出码 1' -Ok ($r6.code -eq 1) -Detail "退出码 $($r6.code)"

    # ---------- C7/C8/C9 ps-param-lint 门禁的行为面自证 ----------
    # 必要性：登记项（笔 2 的 13 条）只守「条文在不在」，守不住「条文仍在、行为已废」。
    # 开发期实测到正是这种失效：walker 只认 `function F { param() }` 而漏掉 `function F()`
    # 位置式声明时，索引 56/66 个函数，那 10 个函数上的坏调用永不报警且整体报绿。
    # C7 即该静默漏检的行为哨兵：形式 B 的索引一旦被改坏，C7 会从「红」退化为「绿」⇒ 本测试失败。
    # 三例分工：C7 证 D1 对形式 B 生效；C8 证 D2（运算符被吞成参数）生效；
    # C9 证已修复写法不误报——且 C9 同时断言 VERDICT 的 files=1/hits=0，
    # 使退出码 0 必须来自「扫过了且干净」，而不是「什么都没扫」。
    $lintRel = 'eval/ps-param-lint.ps1'
    $nl = [string][char]10

    $c7Path = New-Tmp 'eval/tmp-meta-c7-formB.ps1'
    Write-Text $c7Path ('function LB([string]$P) { return 1 }' + $nl + '$v = LB "x" -Foo' + $nl)
    if ((Read-Text $c7Path) -notmatch 'function LB\(\[string\]\$P\)') { throw 'C7 探针未落盘（形式 B 声明缺失）' }
    $r7 = Invoke-Capture -ScriptPath (AbsNew $lintRel) -ArgList @('-FileList', 'eval/tmp-meta-c7-formB.ps1')
    $d7 = ($r7.out -match 'cmd=LB param=Foo rules=D1-undeclared-local-arg') -and ($r7.out -match 'hits=1 D1=1 D2=0')
    Add-Case -Id 'C7 形式B坏调用必须变红' -Expect '退出码 1 且 HIT 指向 LB/D1' -Ok ($r7.code -eq 1 -and $d7) -Detail "退出码 $($r7.code) / 差异出现=$d7"

    $c8Path = New-Tmp 'eval/tmp-meta-c8-formA.ps1'
    Write-Text $c8Path ('function LA { param([string]$P) return 1 }' + $nl + '$v = LA "x" -split ","' + $nl)
    if ((Read-Text $c8Path) -notmatch 'param\(\[string\]\$P\)') { throw 'C8 探针未落盘（形式 A 声明缺失）' }
    $r8 = Invoke-Capture -ScriptPath (AbsNew $lintRel) -ArgList @('-FileList', 'eval/tmp-meta-c8-formA.ps1')
    $d8 = ($r8.out -match 'cmd=LA param=split rules=D1-undeclared-local-arg\+D2-operator-as-argument')
    Add-Case -Id 'C8 split被吞成参数须变红' -Expect '退出码 1 且 HIT 同含 D1/D2' -Ok ($r8.code -eq 1 -and $d8) -Detail "退出码 $($r8.code) / 差异出现=$d8"

    $c9Path = New-Tmp 'eval/tmp-meta-c9-good.ps1'
    Write-Text $c9Path ('function LC { param([string]$P) return 1 }' + $nl + '$y = @((LC "x") -split ",")' + $nl)
    if ((Read-Text $c9Path) -notmatch '@\(\(LC "x"\) -split') { throw 'C9 探针未落盘（正确写法缺失）' }
    $r9 = Invoke-Capture -ScriptPath (AbsNew $lintRel) -ArgList @('-FileList', 'eval/tmp-meta-c9-good.ps1')
    $d9 = ($r9.out -match 'files=1 ') -and ($r9.out -match 'hits=0 D1=0 D2=0') -and ($r9.out -notmatch 'HIT ')
    Add-Case -Id 'C9 正确写法不得误报' -Expect '退出码 0 且扫到1文件且无HIT' -Ok ($r9.code -eq 0 -and $d9) -Detail "退出码 $($r9.code) / 差异出现=$d9"

    # ---------- C10/C11 反证固化：证明 C7 是定向哨兵，不是恒红也不是恒绿 ----------
    # 单看 C7 为绿不足以说明它有牙：若门禁的形式 B 索引被改坏（开发期真实踩过——形式 B 分支失效时
    # 有声明函数只剩 56/71），C7 会由红转绿，而笔 2/4b 的文本登记项抓不到「条文仍在、行为已废」。
    # 这里主动造一个被削弱的门禁副本（把形式 B 分支改成 if ($false)）并断言破坏的形状：
    #   C10 非对称性——削弱副本对形式 B 探针不再报警(0)，对形式 A/D2 探针仍报警(1)。
    #         真门禁对两者都报警（见 C7/C8），故 C7 的绿是「有判别力的绿」，不是无条件红。
    #   C11 自检兜底——削弱副本在发现模式必须被自身 coverage-form-index 自检拦下(2)，
    #         即形式 B 索引丢失不会以「全绿」形态出厂。
    # 与 C2/C3 的 `-PassScript` 同构：断言的是「破坏必然可见」，而非「当前正确」。
    # 措辞约束：本节不得出现 landing 回读那三条 throw 文案里的同一个中文短语——它是
    # meta-landing-readback 的计数 needle（expect=3），多一处提及即 bump 成 4，会造出与削弱无关的假报警。
    $sabRel = 'eval/tmp-meta-c10-lint.ps1'
    $sabPath = New-Tmp $sabRel
    $lintRaw = Read-Text (AbsNew $lintRel)
    $formBAnchor = '        if ($p.Parent -is [System.Management.Automation.Language.FunctionDefinitionAst]) {'
    if (-not $lintRaw.Contains($formBAnchor)) { throw 'C10 锚点缺失（形式 B 分支已变动，反证不再成立）' }
    Write-Text $sabPath ($lintRaw.Replace($formBAnchor, '        if ($false) {   # (meta) 形式 B walker disabled'))
    $sabRaw = Read-Text $sabPath
    if (-not $sabRaw.Contains('(meta) 形式 B walker disabled')) { throw 'C10 削弱行未写入副本（落盘回读失败）' }
    if ($sabRaw.Contains($formBAnchor)) { throw 'C10 削弱未生效（原形式 B 分支仍在）' }
    $s10a = Invoke-Capture -ScriptPath $sabPath -ArgList @('-FileList', 'eval/tmp-meta-c7-formB.ps1')
    $s10b = Invoke-Capture -ScriptPath $sabPath -ArgList @('-FileList', 'eval/tmp-meta-c8-formA.ps1')
    $d10 = ($s10a.out -notmatch 'cmd=LB') -and ($s10b.out -match 'cmd=LA param=split')
    Add-Case -Id 'C10 削弱副本非对称报警' -Expect 'formB探针0(漏检) 且 formA探针1(仍报)' -Ok ($s10a.code -eq 0 -and $s10b.code -eq 1 -and $d10) -Detail "formB=$($s10a.code) formA=$($s10b.code) 非对称成立=$d10"
    $s11 = Invoke-Capture -ScriptPath $sabPath
    $d11 = ($s11.out -match 'SELF-CHECK FAIL\(coverage-form-index\)') -and ($s11.out -match 'B=0')
    Add-Case -Id 'C11 削弱副本发现模式须自检红' -Expect '退出码 2 且报 coverage-form-index 与 B=0' -Ok ($s11.code -eq 2 -and $d11) -Detail "退出码 $($s11.code) / 自检标签与 B=0 出现=$d11"
}
finally {
    foreach ($p in $script:tmpFiles) { if (Test-Path $p) { Remove-Item -LiteralPath $p -Force -ErrorAction SilentlyContinue } }
    $left = @(Get-ChildItem -LiteralPath (AbsNew 'eval') -File -ErrorAction SilentlyContinue | Where-Object { $_.Name -like 'tmp-meta-*' })
    Write-Output ""
    Write-Output ("临时文件清理: 计划 {0} 个, 残留 {1} 个" -f $script:tmpFiles.Count, $left.Count)
    if ($left.Count -gt 0) { $failures++ }
}

Write-Output ""
Write-Output ("===== 元测试结论（用例 {0}）=====" -f $script:cases.Count)
foreach ($cse in $script:cases) { if (-not $cse.ok) { $failures++ } }
if ($failures -eq 0) {
    Write-Output "meta: PASS — 检查器对全部注入手法均按预期报警，且基线为绿"
    exit 0
}
Write-Output ("meta: FAIL — {0} 项不符合预期 ⇒ 检查器已失效或本测试自身失真，禁止宣称已守护" -f $failures)
exit 1
