# 票 28：把 `search-clear` 的标签判据从绝对上限改为"与同 run `off` 基线的相对差"

**What to build:** 让 `eval/pixel-evidence.ps1` 的 `search-clear-removes-panel-and-label` 只判定它真正想守的事实——"清除后标签不再存在"——而不再把"场景散点计数"这一与清除无关的量纳入阈值。形式：标签底色改为与**同一次运行**的 `search-off` 基线比较增量（`|label(clear) − label(off)| ≤ SearchClearedMaxBgDelta`），面板腿保留原强判据 `panelGatedRows == 0`，并按用户勾选补上 `off` 侧停稳先决。本票由票 27 §6.2 列出的三条待裁决路径中的**选项 ①** 落地而来，用户勾选参数：S1 容差 = **10**、S2 = **乙方案（原理由注释保留不删，只追加更正）**、S3 = **加 `settledOk['off']`**、S4 = **本轮离线重放 S1–S5**。

## 1. 五要素

| 要素 | 内容 |
| --- | --- |
| 命令 | `pwsh -NoProfile -File eval/pixel-evidence.ps1`（需可见桌面与按键注入；本票落地时**未执行**，验证改走离线重放） |
| 固定样本 | `tests/corpus/excalidraw` 的五个搜索态帧（`search-off / results / nomatch / focus / clear`，各取最后一个有效帧）；本票 S1–S4 用的是既有产物 `out/edge-after2`（票 27 对照 run，exe SHA256 前缀 `D2D09E12`） |
| 期望读数或行为 | `panelGatedRows = 0`、`labelDelta = abs(label(clear) − label(off)) = 0`（≤ 容差 10）、`settled(clear=True, off=True)`，断言汇总回到 27/27 且退出码 0；旧常量 `SearchClearedMaxBgPixels` 残留引用 = 0 |
| 失败判据 | 出现下列任一即失败：`labelDelta > 10`；或 `panelGatedRows > 0`；或 `settled` 任一腿为 False；或 `cite-check` 报 `pe-crit-clear-relative`（相对腿被删/掏空）、`pe-crit-clear-offsettled`（off 侧先决丢失）、`pe-thr-clearedmaxdelta`（容差常数漂移）；或离线重放的保真自证与 `report.json` 读数不相等（说明重放失真，不得据其下结论） |
| 自动化状态 | 判据本身在 `eval/pixel-evidence.ps1`（本机人工项，不进 CI）；其**文本形态**由上述三条 registry 守护在 `cite-check` 中自动守住（S5 已实证能咬）。**端到端读数待一次授权抓屏**，本票不声称已复测 |

---

## 2. 为什么要改（实测依据，非推断）

回读两份既有产物（`out/edge-before` = `2c8f830^` 二进制、`out/edge-after2` = 含票 27 改动的二进制，两者 exe SHA256 前缀 `87605C90` / `D2D09E12` 不同）：

| 量 | `edge-before` | `edge-after2` | 含义 |
| --- | --- | --- | --- |
| `labelBgClear` | 51 | 151 | 清除态右下标签区底色计数 |
| `labelBgOff` | 51 | 151 | **同 run 无搜索基线** |
| `labelBgClear − labelBgOff` | **0** | **0** | 判据真正想守的事实：标签未残留 |
| `labelBgFocus` | 33201 | 33201 | 标签**真实存在**时的规模 |
| `panelGatedRowsClear` | 0 | 0 | 面板连贯行数（强腿） |
| `panelGatedRowsResults` | 159 | 159 | 面板打开时的连贯行数 |

旧判据用的是绝对上限 `SearchClearedMaxBgPixels = 100`，而该上限约束的是"标签底色 **+ 场景散点**"的合计计数。票 27 把弱边 alpha `1.0→0.35`、线宽 `1.25→0.95px` 后，散点计数从 51 抬到 151，上限被撑破，**但 `label − off = 0`、`gatedRows = 0`、`settled = True` 三项同时成立**——即面板与标签确已消失。这与 `151a49c` 当年判定面板区绝对上限"结构性临界（散点地板 110 > 上限 100）"是同一种病，只是标签区此前未触发。⇒ **坏的是判据形式（绝对上限），不是数值**；因此处置方式是改形式，不是调数值。

容差取 10 的**尺度分离**依据（用户勾选，先定依据再取值）：正常清除后差值实测恒为 **0**；标签真实存在时差值为 **33050**（`33201 − 151`）⇒ 相差约 3300 倍；即使 5% 的标签泄漏（约 1660px）也远超 10。取 10 而非 20 是因为实测余量为 0，收紧不损失安全性。

## 3. 改动内容

- `eval/pixel-evidence.ps1`（提交 `cc00e9a`，numstat **17 insertions / 5 deletions**，hunk `@@ -103 +103,8 @@`、`@@ -1062,4 +1069,9 @@`）：
  - 阈值 `SearchClearedMaxBgPixels = 100` 退役，新增 `SearchClearedMaxBgDelta = 10`，尺度分离依据写入紧邻注释。
  - 布尔式：绝对腿 → `([Math]::Abs($clearLabel[2] - $offLabel[2])) -le SearchClearedMaxBgDelta`；强腿 `($clearPanel[3] -eq 0)` **原样保留**；追加 `$settledOk['off']`。
  - 读数新增 `labelDelta`，`settled` 拆成 `settled(clear=..,off=..)` 两分量，criterion 文案随之更新。
  - 原三行"标签区不存在地板高于上限的结构性问题"的理由注释**保留未删**，其后追加带日期的证伪更正块（S2 乙方案）。
  - 旧常量残留引用 = 0，Parser 错误数 = 0。
- `eval/baseline/citations.json`（提交 `9df0345`，numstat **7 / 3**，hunk `@@ -257 +257 @@`、`@@ -355,2 +355,6 @@`；A 口径 156 → 158）：
  - `pe-thr-clearedmax` 就地改写为 `pe-thr-clearedmaxdelta`（needle `    SearchClearedMaxBgDelta = 10`，expect 1）——不改写就会因前提消失而永久红。
  - 新增 `pe-crit-clear-relative`（needle `[Math]::Abs($clearLabel[2] - $offLabel[2])`，**expect 2**：读数与布尔式各一次；取 2 是为了"只留读数、掏空判据"也会报警）。
  - 新增 `pe-crit-clear-offsettled`（needle `$settledOk['clear'] -and $settledOk['off']`，expect 1）。
  - `pe-crit-clear-coherent`（needle `(($clearPanel[3] -eq 0)`，expect 1）不动。

## 4. 能变红的证明（S1–S5，本轮未抓屏）

| # | 构造 | 实测读数 | 结论 |
| --- | --- | --- | --- |
| 保真自证 | 离线重算 6 个读数 vs 同 run `report.json` | `labelBgClear 151/151`、`labelBgOff 151/151`、`labelBgFocus 33201/33201`、`gatedRows` `0/0`、`0/0`、`159/159` ⇒ **6/6 相等** | 重放用的是脚本里切出的真实度量函数（`PixelMetrics` C# + `Load-Pixels` + `Get-ColorExtent/Get-PanelExtent/Get-LabelExtent`，按行区间取自同一已提交文件，未复制实现），读数可用于下结论 |
| S1 正对照 | 真实 `clear` vs 真实 `off` | `gatedRows=0`、`delta=0` ⇒ **PASS** | 不是把判据调松成必然通过 |
| S2 卡住的标签 | `clear` 换成 `focus` 帧 | `delta=33050 > 10` ⇒ **FAIL** | 容差 10 不是橡皮图章，真缺陷照样抓得住 |
| S3 基线侧异常 | `off` 换成 `focus` 帧 | `delta=33050` ⇒ **FAIL** | `Abs` 的**双侧**性成立，非单向放行 |
| S4 面板未关 | `clear` 换成 `results` 帧 | `gatedRows=159 ≠ 0` ⇒ **FAIL**（此时 `delta=0` 仍为 True） | 强腿独立生效 ⇒ 相对化不削弱"面板没关"的防护 |
| S5 掏空反证 | 把布尔式里的相对腿临时替换为 `$true`（对已提交文件），跑 `cite-check` | 失败项**恰为** `pe-crit-clear-relative`（`条目=158 PASS=157 FAIL=1`）；`git checkout --` 还原后回到 `158/0` 且相对腿命中回 2 | 守护能咬，判据不会被悄悄掏空 |

**过程中发现并修正了量具自身的缺陷**：首版重放脚本用 `$r = Judge ...` 接收返回值，而 `Judge` 内的诊断行走的是 `Write-Output` ⇒ 被赋值吞成「字符串 + 布尔」数组，导致首跑的"4/4 符合预期"失真（逐例结论行也确实没打印出来）。改为诊断走 `Write-Host`、函数只回传 `[bool]`，并额外打印逐例布尔值与类型后再重跑，才得到上表的判定。**首跑结果不得引用**。

`settled` 两腿的来源如实声明：本轮未重算逐帧时间序列，取同 run 聚合断言 `search-states-settled` 的 `pass = True`，其定义为"五个搜索态输入后两帧的 `total/达标行数/最长行命中` 一致"⇒ 聚合为真必然蕴含 `clear` 与 `off` 两态各自为真；该推导成立的前提是聚合断言的构造方式（`eval/pixel-evidence.ps1:1039-1043`），未另做直接复测。

## 5. 本票的门禁读数（落地时实跑）

- `cite-check`：**条目=158 PASS=158 FAIL=0 SKIP=0**，退出码 0。
- `injection-selftest`：**14 例全 PASS**（C1a…C11），退出码 0，临时文件清理 11 个 0 残留。
- `ps-param-lint`：退出码 0。
- `cargo fmt --all -- --check`：退出码 0；`cargo test --workspace`：**251 passed / 0 failed / 3 ignored**（40 个 result 行，`FAILED` 命中 0）；`cargo clippy --locked --workspace --all-targets -- -D warnings`：退出码 0。
- 分层落笔造成的**预期瞬态红**（留档不掩盖）：`cc00e9a` 当时 `cite-check` 为 `156/155/1`，唯一失败项 `pe-thr-clearedmax`（needle 指向已被退役的上限行）；元测试同因这单一根因报 C1a / C3b / C4 三例 FAIL。`9df0345` 之后两者同时回绿 ⇒ 因果闭环确认，未用合并两笔或 amend 来消除中间态。

## 6. 未做与未验证

- **未抓屏**：新判据的端到端读数（`report.json` 里 `labelDelta=0`、断言回到 27/27）尚未观测，需用户另行授权一次 `pixel-evidence` 抓屏（该脚本会占用可见桌面并注入按键）。S1–S5 只证明像素腿与守护能变红，**不等于**整链已通过。
- **未验证**：`off` 基线帧若本身异常（如空白帧被判有效），相对差会失真——现仅由 `settledOk['off']` 与 `search-off-panel-floor-zero` 的 `gatedRows==0` 兜住，未构造过"基线帧空白"的实测反例。
- **未验证**：容差 10 的统计来源不是分布拟合，而是尺度分离 + 实测余量 0；若后续渲染改动使 `label`/`offLabel` 两侧同时漂移且漂移量不同向，本判据可能出现新的假红/假绿，需按同法重评而非调数值。
- **不做**：不调高绝对上限凑绿、不回退票 27 的线宽（那是"为通过而调渲染参数"）、不删除历史读数与原始理由注释。

---

## 7. 端到端复测结果（2026-11-19 用户授权抓屏；本节为追加，上文 §6 的"端到端未复测"原文保留不删，其结论已被本节推翻）

**取用二进制溯源（先看证据再读结果）**：`cargo build --release --locked -p arch-view` 后 `target/release/arch-view.exe` 的 SHA256 前缀 = `D2D09E12`，与 §2 表中产生那条红的 `out/edge-after2` 所用二进制**同一份**；`git log 2c8f830..HEAD -- crates Cargo.toml Cargo.lock` 为空 ⇒ Rust 源自票 27 落地后一字未变，本批三笔只动 eval 与文档。⇒ 读数的变化只能归因于判据形式本身。（沿用 §"capture-before/after" 教训：不看 `Finished` 字样，只看哈希。）

**抓屏 run**：`pwsh -NoProfile -File eval/pixel-evidence.ps1 -OutDir out/opt1-verify`，退出码 **0**；`report.json` 的 `generated = 11/19/2026 01:15:15`、`exe = target/release/arch-view.exe`、`total/passed/failed = 27/27/0`；产物 48 个文件、其中 35 张 PNG。run 目录为**显式指定**（`out/` 属 gitignore，仓内不可回读，故关键读数逐字抄录如下）。

- 目标断言 `search-clear-removes-panel-and-label`：`pass = True`
  - criterion 原文：`清除后连贯面板行数必须为 0，且标签底色与同 run off 基线之差 <= 10px，两侧状态均须已稳定`
  - reading 原文：`panelGatedRows=0 panelTotal=141 label=151 offLabel=151 labelDelta=0 offGatedRows=0 settled(clear=True,off=True)`
  - ⇒ 用户要求的三项全部实测成立：**`labelDelta = 0`、`panelGatedRows = 0`、`27/27`**。
- 旧上限形式的残留断言数 = **0**（按 `criterion` 匹配"标签底色 <= 100 / 与基线一致（该态须已稳定）"）。
- 同 run 相邻断言（未回归）：`search-off-panel-floor-zero`（`offGatedRows=0 offTotal=141` 通过）、`search-focus-closes-panel`（`focusGatedRows=0`）、`search-focus-label-visible`（`bg=33201 text=2028`）、`search-states-settled`（五态 `total/gatedRows/maxRowHits` 两帧全等）。
- **P1 未被本批改动波及**：`p1-frames-distinguishable` = `changed=7.893% meanAbs=1.461 maxCell=100.00% hotCells=428 settled(off=True,on=True)`，与票 27 落地后同值；`p1-frames-settled` 两态 `0.000%`（上限 0.1%）。

**同 run 用新帧再跑离线重放**（`replay-opt1-s1s4.ps1 -Dir out/opt1-verify`，退出码 0）：保真自证 **6/6 与该 run `report.json` 逐字相等**（151 / 151 / 33201 与 0 / 0 / 159），S1 `delta=0` PASS、S2 与 S3 `delta=33050` 由相对腿 FAIL、S4 `gatedRows=159` 由强腿 FAIL（此时 `delta=0` 仍 True）⇒ 逐例布尔 `[True, True, True, True]`，4/4 符合预期。

**一次意外的可复现性观测（值得留档）**：同一二进制、两次独立 run（`edge-after2` 与 `opt1-verify`）的 search 帧哈希对比 —— `search-off-f03`、`search-clear-f03`、`search-focus-f03` **逐字节相同**，而 `search-results-f03` **不同**；但两侧的被测量完全一致（`off=141`、`results=50303`、`gatedRows=159`）。⇒ 结论：**"帧字节不同"与"读数不同"是两件事**，判据读数量一致时不必因像素哈希变化而重跑；反之若只比哈希会得出与判据无关的结论。此现象的根因（结果列表帧为何不稳定）本轮未查，登记为未验证项。

**仍未闭合（本票最终状态）**：
- 只有 **2 次**独立 run 支撑 `labelDelta = 0`（`edge-after2`、`opt1-verify`），未做过多次分布；若某次渲染改动使两侧散点**不同向**漂移，本判据仍可能假红，届时按同法重评形式而非调数值。
- `off` 基线帧自身异常（空白帧被判有效）的反例仍未构造。
- `search-results-f03` 跨 run 不稳定（字节级）根因未查。
- README 的 27/28 两行仍未加存在性守护；未建 `docs/evidence` 档案与配套 A 口径登记（超出本批批准范围），关键读数以本节约逐字入档的方式获得仓内可回读性。
