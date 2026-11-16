# 26: 文档引用一致性检查（把「数字必须脚本回读」落成工具）

**What to build:** 新增 `eval/cite-check.ps1` + 入仓登记表 `eval/baseline/citations.json` + CI job `cite-check`，把 `ENGINEERING-DISCIPLINE.md` §5 第 7 条（登记类数字必须先脚本回读，目前是**人工判据**）变成机器检查：登记表逐条声明「文档中的某段字面 ↔ 源数据字段/算式」，工具读源、按规则规范化、再断言文档字面与之相符。同时给工具自身做 negative control。

**动机:** §5 第 7 条立项的直接原因是本会话实测的四类取证失效，全部会伪装成「已核对」：

1. **批次/目录归属误记（两例）**：把 c1 `out/paired-c-20261116-173507`（17:35）的读数写成 c2 `out/paired-c2-20261116-190406`（19:04）——而 c2 批实测**不含 labels 档**。一例落在票 23 表格（已由 `0ecbbc8` 追加更正），一例落在票 24 负对照归档（`afb09fe` 提交信息内纠正，正文未记）。
2. **相对幅度写成区间估计**：把噪声地板漂移写成「30–80%」，脚本实算为 medianMean **+29.9%**、段间 std **+82.9%**、地板 **+103.7%**（1.9→3.87ms）——上限被低估且形态错误。
3. **文档内行号引用一经编辑即漂移**：`c965f13` 初稿引用「§10.6 裁决 A 在本文件第 267–268 行」，而同一笔插入 8 行后该裁决实际位于第 **275** 行——自己造成的引用失效。故检查工具与文档一律只用节指针、不用文档行号。
4. **取证命令自身给出假读数（四种形态，本票要求工具内部规避）**：`rg -q` 无 stdout 使 `[bool](rg -q …)` 恒为 False；`Measure-Object -Line` 少报（把 20 笔未推送提交报成 7）；**PowerShell 解析错误导致整条命令的所有语句都不执行**（曾使 `git checkout` 回滚未发生而后续检查仍显示 PASS，构成假安全状态）；**git／外部命令的中文输出经 PowerShell 管道按系统码页解码变乱码，使中文模式的 `-match`／`Select-String` 恒 0 命中（假阴性，会让人误判「证据不存在」而错删本来正确的登记）**。因此 `cite-check.ps1` **内部一律用 `rg --pcre2` 直接读文件取命中与计数，禁止用 PowerShell 字符串匹配中文内容**；数值比较走 `ConvertFrom-Json` + 规范化字符串，不靠中文文本比对。

**Blocked by:** 纪律 §5 第 7 条（`c965f13`）、§8 第 14 条登记该规则未工具化（`4cb8931`）、`eval/baseline/paired-ab-selftest.json`（`a7a093f`，提供 CI 可见的源数据）、`eval/paired-ab-selftest.ps1`（`97e8817`，同源结构可复用）。

**Status:** proposal-only（2026-11-16 用户批准开票与实现方案；本票实现分 4 笔，登记范围见下）。

## 范围声明（必须写在票首，防止工具自身变成假绿来源）

- 首批只登记**本会话新写入的高风险数字**（票 23「筛查读数更正与补全」14 组两侧聚合值、若干结构断言与 SHA 引用），约 30 条，**不补历史文档**。
- 因此 cite-check 的通过**只声称**「已登记项一致」，**绝不声称**「所有文档数字都可信」。未登记项不受保护；该限制同步写入 CI job 注释与本票验收五项第 5 条。
- 新增高风险登记数字时须同步加登记项（人工判据，源自 §5 第 7 条），漏加不算 cite-check 失败。

## 验收五项

1. **命令**
   - 工具自检：`pwsh -NoProfile -File eval/cite-check.ps1`（默认读 `eval/baseline/citations.json`）
   - 自身 negative control（必做）：`pwsh -NoProfile -File eval/cite-check.ps1 -SelfTest` → 三种预置坏登记（`json-field` 的 needle 改成错值、`derived` 的 formula 换错、`pattern-count` 的 expect 加 1）**全部必须判 FAIL**，任一未抓到即 exit 1
   - 本机全量（含 out/ 源）：`pwsh -NoProfile -File eval/cite-check.ps1` → 期望 FAIL=0，SKIP 数必须打印
   - CI 等价：同一条命令在 ubuntu runner 上执行，`out/**` 源条目判 **SKIP 而非 PASS**；**不启用** `-RequireAllResolved`（启用会令 job 上线即红）
   - 升级口验证（本机）：`pwsh -File eval/cite-check.ps1 -RequireAllResolved` → 期望 exit 1 且打印 SKIP 计数，证明该开关语义正确
   - 每笔提交前：`cargo test --workspace`、`cargo fmt --all --check`、`cargo clippy --locked --workspace --all-targets -- -D warnings`
2. **固定样本**：`eval/baseline/citations.json`（登记表入仓）+ `eval/baseline/paired-ab-selftest.json`（CI 可见源）；`out/t23-stage0-*`、`out/t23-stability-searchload-*`、`out/paired-regression-*` 作本机源（CI 不可见）；git 引用以 `git rev-parse --verify <sha>^{commit}` 判定。
3. **期望读数或行为**
   - 登记表 kind 五种：`json-field` / `derived` / `pattern-count` / `sha` / `file-exists`；每条含 `id`、`doc`、`needle`、`ciVisible`，数值类另含 `source`+`field`+`format` 或 `sources`+`formula`。
   - **`derived` 不登记结果数字**，只登记源字段与算式（如 `pct_change` = `(b/a-1)×100` 后 Round 1 位），由工具现算再与文档字面比较——否则等于把数字手抄第二遍。
   - `format: signed3` 必须把 ASCII 负数转成文档使用的 **U+2212** 字形（实测票 23 用 `−0.622`，夹具值为 `-0.622`），转换失败即 FAIL，不得退化为「去掉符号再比」。
   - 输出每条一行 PASS/FAIL/SKIP + 末尾三类计数；FAIL 必须打印 `id / doc / needle / 源实值`。
   - 退出码：FAIL=0 且未开 `-RequireAllResolved` → 0；有 FAIL → 1；开了该开关且 SKIP>0 → 1。SKIP 数必须出现在输出，不得静默。
   - 首批登记条数与 CI 可见比例：**登记为设计目标 30±3 条、CI 可见 22–26 条、SKIP 4–8 条；实际数字以首次执行输出为准，不得预先写入读数**。
4. **失败判据**
   - `-SelfTest` 三种坏登记中任一未被抓到 → 本票不得标完成（工具未证明能变红）。
   - 把 SKIP 计入 PASS、或输出不显示 SKIP 数 → 判实现错误。
   - 工具内部改用 PowerShell 中文 `-match`/`Select-String` 取证（第 4 类坑）→ 判实现不合规。
   - 因 FAIL 而**删除登记项**或把 `expect`/`needle` 改成「刚好等于实得的任意值」→ 判为回改判据；文档改写文案导致的字面不符，处置只能是同步更新该条 `needle` 并在提交信息写明。
   - 用 `derived` 之外的方式登记百分比（即写死结果数字）。
   - 声称 cite-check 通过 = 文档数字全部可信（范围越界）。
   - 登记表命名成 `eval/baseline/drift-*.json`（会被 `drift-gate.ps1:74-81` 的 `corpusPatterns`/`baselinePatterns` 当作语料基线改动，改变 CI 行为）。
5. **自动化状态**：`已自动化（CI）`，覆盖范围限定为已登记项；`out/**` 源条目在 CI 记 SKIP（本机可 PASS），未登记数字不受保护。登记表自身的完整性（是否漏登）仍是人工判据，不做自动检查——此点必须保留在报告中，不得由 CI 绿掩盖。

## 实现顺序（按层提交，4 笔）

1. **票 + README 同步**（本文件与 `docs/tickets/arch-tool/README.md`）：判据先于代码。
2. **`eval/cite-check.ps1` + `eval/baseline/citations.json`**：工具与输入同笔生效（否则工具无输入，属§6 强耦合例外，须在提交信息写明）。含 `-SelfTest`。
3. **`.github/workflows/ci.yml` 新增 `cite-check` job**：ubuntu、纯 pwsh、无 cargo 无桌面依赖；注释写明「只守已登记项 + SKIP 必须露出 + 未启用 RequireAllResolved」。
4. **`docs/acceptance-gaps.md` §8 追加更正段**：不回改第 14 条原文，追加带日期段落改注为「已工具化 + 剩余边界（out/ SKIP、未登记项不受保护、登记表完整性靠人工）」。

## 未验证的

- 本票实现尚未落地，无任何 PASS/FAIL/SKIP 读数。
- needle 与文档文案强耦合的维护成本未实测（改一句话引发多少 FAIL 未知）。
- 历史文档中既有数字的错误率未审计（首批不覆盖，属已知盲区）。
- `-RequireAllResolved` 何时可启用（取决于 `out/**` 源是否入仓或由夹具替代覆盖），未决定。
