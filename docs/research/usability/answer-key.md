# 标准答案 key（仅主控；参与者不可见）

**Key 版本：** `UK-2026-11-08-01`
**样本：** `privatenumber/tsx` @ `4473971f4b3261de253209b148d8fdc43269fcc4`
**历史窗口：** first-parent，最近 500 个提交（截至钉住 commit）
**冻结时间：** 2026-11-08（首位参与者前）
**SHA256：** 待首位参与者前写入 `out/usability/key.sha256`（本文件完成后由主控计算）

> 使用规则：本文件只能出现在主控的另一台设备/扣面纸面。任何参与者可见的屏幕、录屏、共享剪贴板都不得出现本文件。
> 若本文件在首位参与者后被修改，本轮结果按票 12 失败判据作废；修正必须另起一轮并保留旧 key。

> **复核口径（2026-11-08 修正）：** 本文件所有答案在 `docs/usability-kit` HEAD `63ea209` 的二进制上实测。合并到主线后，需在**改后二进制上复核**。其中 T4 依赖 `arch-view diff` 的界面/输出行为，T5 依赖地图的边高亮与步进行为，属于必须复核项；T1–T3 的读数来自 `arch-cli` 产物，同样建议复跑。答案本身不因这条口径修正而改动；只有复核实测推翻时，才按变更日志修正并另起一轮。
> 本文件未记录该批读数采集时的构建类型；涉及启动耗时/界面表现的读数按“构建类型不明，已失效，需重测”处理。

---

## 0. 复现环境与读数说明

本次读数使用：

```
d:\project\1107\target\debug\arch-cli.exe
d:\project\1107\target\debug\arch-view.exe
```

对应 `origin/main` `63ea209` 的代码（该提交相对二进制构建提交只多一个 docs 提交）。样本仓放在：

```
d:\project\1107-ux\out\usability-corpus\tsx
```

`out/` 被 `.gitignore` 忽略，不进主仓。样本仓为完整 clone，`git rev-parse HEAD` = `4473971f4b3261de253209b148d8fdc43269fcc4`，`git rev-list --count --first-parent HEAD` = `617`。

**重建 key 产物的命令：**

```powershell
$UX   = "D:\project\1107-ux"
$REPO = "$UX\out\usability-corpus\tsx"
$OUT  = "$UX\out\usability-corpus\artifacts"
$BIN  = "$UX\target\debug"
New-Item -ItemType Directory -Force -Path $OUT | Out-Null

& "$BIN\arch-cli.exe" index   $REPO --out "$OUT\index.json"
& "$BIN\arch-cli.exe" layers  $REPO --out "$OUT\layers.json"
& "$BIN\arch-cli.exe" history --repo $REPO --max-commits 500 --out "$OUT\history-500.json"
& "$BIN\arch-cli.exe" path    $REPO --from "src/cli.ts#module-toplevel" --to "src/utils/ipc/get-pipe-path.ts#function:getPipePath" --out "$OUT\path-t1.json"
```

实测统计：`index.json` 136 个模块、379 条边；`history-500.json` 扫描 500 / 可用 617 个提交、159 个历史模块；`path-t1.json` 3 步。

---

## T1：入口到目标的路径与层

### 取答案命令

```powershell
$UX   = "D:\project\1107-ux"
$REPO = "$UX\out\usability-corpus\tsx"
$BIN  = "$UX\target\debug"
& "$BIN\arch-cli.exe" path $REPO `
  --from "src/cli.ts#module-toplevel" `
  --to "src/utils/ipc/get-pipe-path.ts#function:getPipePath" `
  --narrative
```

### 实测输出（原文）

```text
路径 src/cli.ts#module-toplevel → src/utils/ipc/get-pipe-path.ts#function:getPipePath：3 步
- src/cli.ts#module-toplevel 是起点（层：unclassified，模块：src/cli.ts）
- src/cli.ts#module-toplevel 通过 call 到达 src/utils/ipc/server.ts#function:createIpcServer（src/cli.ts:237:20；层：unclassified → utils）
- src/utils/ipc/server.ts#function:createIpcServer 通过 call 到达 src/utils/ipc/get-pipe-path.ts#function:getPipePath（src/utils/ipc/server.ts:37:19；层：utils → utils）
```

### 标准答案

| 步 | 符号 ID | 模块 | 层 | 入边 |
| --- | --- | --- | --- | --- |
| 1 | `src/cli.ts#module-toplevel` | `src/cli.ts` | `unclassified` | 起点 |
| 2 | `src/utils/ipc/server.ts#function:createIpcServer` | `src/utils/ipc/server.ts` | `utils` | `call`（`src/cli.ts:237:20`） |
| 3 | `src/utils/ipc/get-pipe-path.ts#function:getPipePath` | `src/utils/ipc/get-pipe-path.ts` | `utils` | `call`（`src/utils/ipc/server.ts:37:19`） |

- **层序列：** `unclassified → utils → utils`；去重后 `{unclassified, utils}`。
- **模块序列：** `src/cli.ts → src/utils/ipc/server.ts → src/utils/ipc/get-pipe-path.ts`。
- **路径长度：** 3 步（状态栏 `k/N` 会泄露 N；见 README §4.1）。

**通过条件：** 三步符号、模块、层全部正确且顺序正确；起点/终点不得互换；不得漏掉中间步。
**部分分（仅诊断）：**

```
symbol_score = 按位置匹配的步数 / 3          （位置 i 要求符号 + 模块都对）
layer_score  = 1 if 层序列或去重集合正确 else 0
partial      = 0.7 * symbol_score + 0.3 * layer_score
success      = (symbol_score == 1.0) and (layer_score == 1.0) and not timeout
```

**常见错误与归因：**

| 观察 | 归因 |
| --- | --- |
| 只报起点/终点，漏掉 `createIpcServer` | `V-PATH`（路径播放没有让他看到中步）或 `F-CONTROL`（不会前进） |
| 把起点层说成 `utils` | `V-MAP`（层带归属没看清） |
| 把路径说成运行时执行顺序 | 记录；T5 正式评 `S-EXEC` |
| 直接读状态栏 `3 步` 但说不出符号 | `ui_leak_t1_step_count=true`；不算通过，记录泄露 |
| 找不到“前进/播放”按钮 | `F-CONTROL` |

---

## T2：窗口内被改得最频繁的 3 个模块

### 取答案命令

```powershell
$UX   = "D:\project\1107-ux"
$OUT  = "$UX\out\usability-corpus\artifacts"
$h    = Get-Content "$OUT\history-500.json" -Raw | ConvertFrom-Json
$idx  = Get-Content "$OUT\index.json" -Raw | ConvertFrom-Json
$keys = @($idx.modules | ForEach-Object { $_.id })

# 只看当前 HEAD 仍存在的模块；按修改次数（commitCount）降序；tie-break: churn 降序，再模块 ID 升序
$h.modules |
  Where-Object { $keys -contains $_.id } |
  Sort-Object @{Expression={-$_.commitCount}},@{Expression={-$_.churn.total}},@{Expression={$_.id}} |
  Select-Object -First 6 |
  ForEach-Object { "{0}`tcount={1}`tchurn={2}" -f $_.id, $_.commitCount, $_.churn.total }
```

### 实测读数（前 6）

| 排名 | 模块 | `commitCount` | `churn.total` |
| --- | --- | --- | --- |
| 1 | `src/esm/hook/resolve.ts` | 64 | 3073 |
| 2 | `tests/specs/smoke.ts` | 56 | 3226 |
| 3 | `tests/specs/api.ts` | 48 | 3202 |
| 4 | `tests/specs/cli.ts` | 39 | 2093 |
| 5 | `src/esm/hook/load.ts` | 34 | 1809 |
| 6 | `tests/specs/watch.ts` | 31 | 2163 |

### 标准答案（集合，顺序不计）

```
{ src/esm/hook/resolve.ts, tests/specs/smoke.ts, tests/specs/api.ts }
```

**关键稳健性：** 按 `churn.total` 降序的前 3 名也是同一集合，只是顺序不同：

```
tests/specs/smoke.ts (3226) → tests/specs/api.ts (3202) → src/esm/hook/resolve.ts (3073)
```

所以任务只要求“哪 3 个”，不要求排序；任一口径（修改次数或改动量）都应接受同一集合。

**通过条件：** 三个模块集合完全正确，无多报、无错报；顺序不计。
**部分分（仅诊断）：**

```
intersection = |answer_set ∩ key_set|
F1           = 2 * intersection / (|answer_set| + |key_set|)
success      = (F1 == 1.0) and not timeout
```

**范围：** 所有 HEAD 仍存在、且非 `external:` 开头的仓库节点都算模块，包括 `tests/*`、`docs/*` 与类型声明文件；`external:` 节点是依赖包，没有本仓库历史；历史里出现过但 HEAD 已删除的模块不在时间线布局上，不参与 T2/T3。脚本已明确“非 `external:` 节点都算”。
**诊断变体（不接受）：** 若参与者自行限定只统计 `src/*`，会得到：

| 排名 | `src/*` 内模块（HEAD 仍存在） | `commitCount` | `churn.total` |
| --- | --- | --- | --- |
| 1 | `src/esm/hook/resolve.ts` | 64 | 3073 |
| 2 | `src/esm/hook/load.ts` | 34 | 1809 |
| 3 | `src/cjs/api/module-extensions.ts` | 24 | 760 |

这组答案按 key **不算通过**；在观察里记 `scope_reduced=true`，归因 `U` 只在确认脚本没被听清/有歧义时使用，否则记 `V-TL`（没有排序/筛选导致只看了部分）。

**常见错误与归因：**

| 观察 | 归因 |
| --- | --- |
| 只点几个模块就下结论 | `F-FLOW`（没有系统扫描的动线） |
| 想找“排序/筛选”但找不到 | `I-RANK`（信息缺失：无全局排序） |
| 颜色/大小饱和，热点看起来一样 | `V-TL`（视图编码饱和） |
| 把 churn 当“修改次数”，或反之 | 记录依据；集合相同则不影响通过 |
| 把 `tests` 排除 | `U` 仅在脚本歧义时；否则 `V-TL`/`I-RANK` |

---

## T3：窗口内很久没有被改动的 2 个模块

### 取答案命令

```powershell
$UX   = "D:\project\1107-ux"
$OUT  = "$UX\out\usability-corpus\artifacts"
$h    = Get-Content "$OUT\history-500.json" -Raw | ConvertFrom-Json
$idx  = Get-Content "$OUT\index.json" -Raw | ConvertFrom-Json
$keys = @($idx.modules | ForEach-Object { $_.id })

# 只看当前 HEAD 仍存在的模块；按 lastModified 升序，tie-break 模块 ID 升序
$h.modules |
  Where-Object { $keys -contains $_.id } |
  Sort-Object lastModified, id |
  Select-Object -First 5 |
  ForEach-Object {
    "{0}`tlast={1} ({2})`tcount={3}`tchurn={4}" -f `
      $_.id, $_.lastModified, (Get-Date -UnixTimeSeconds $_.lastModified -Format 'yyyy-MM-dd'), $_.commitCount, $_.churn.total
  }
```

### 实测读数（最早 5 个仍存在于 HEAD 的模块）

| 排名 | 模块 | `lastModified`（epoch） | 日期 | `commitCount` | `churn.total` |
| --- | --- | --- | --- | --- | --- |
| 1 | `src/@types/es-module-lexer.d.ts` | 1695558929 | 2023-09-24 | 1 | 3 |
| 2 | `src/remove-argv-flags.ts` | 1700737314 | 2023-11-23 | 2 | 8 |
| 3 | `src/watch/utils.ts` | 1707526537 | 2024-02-10 | 4 | 30 |
| 4 | `docs/postcss.config.js` | 1713432434 | 2024-04-18 | 1 | 6 |
| 5 | `src/utils/is-windows.ts` | 1714471209 | 2024-04-30 | 1 | 1 |

### 标准答案（集合，顺序不计）

```
{ src/@types/es-module-lexer.d.ts, src/remove-argv-flags.ts }
```

**判定口径：** 在最近 500 个 first-parent 提交窗口内、且当前 HEAD 仍存在的模块中，`lastModified` 最早的两个；并列按模块 ID 升序。`lastModified` 是“最后一次被改动的提交时间”，不是 `firstSeen`，也不是 `churn`。

**通过条件：** 两个模块集合完全正确，无多报、无错报。
**部分分：** 同 T2 的 F1 公式，`key_set` 为上面两个模块。

**诊断变体（不接受）：** 若排除 `.d.ts`，最早两个会变成 `src/remove-argv-flags.ts` + `src/watch/utils.ts`。本 key 不接受该变体，因为地图把 `.d.ts` 当作仓库模块节点，脚本也要求“非 `external:` 节点都算”。记录 `excluded_dts=true`，归因 `V-TL`/`I-RECENCY`，不归 `U`，除非确认脚本歧义。

**常见错误与归因：**

| 观察 | 归因 |
| --- | --- |
| 把“颜色冷/改动少”当成“很久没动” | `S`（语义误读：热力是累计量，不是最近时间）或 `V-TL` |
| 看不懂 `lastModified` 的原始 epoch | `V-TL` / `I-RECENCY`（无格式化日期） |
| 只比较点过的几个，没系统扫描 | `F-FLOW` |
| 找到了“最后修改”但不知道要比较所有模块 | `I-RECENCY` |
| 报第 3/4 名（`watch/utils.ts`/`docs/postcss.config.js`） | 失败；记 `near_miss`，不改成通过 |

---

## T4：一次提交的模块归属

### 固定提交对

```
from: 3df00f4e64fd013ef8bc789b0be0496a033f4d3d   (2024-07-30 fix(resolver): prioritize requested path in dependencies)
to:   ca4bf11670bdbc29bd5fe413cfa3cedba02c2e4e   (2024-07-31 refactor(cjs): resolver organized in multiple files)
```

### 取答案命令

```powershell
$UX   = "D:\project\1107-ux"
$REPO = "$UX\out\usability-corpus\tsx"
$BIN  = "$UX\target\debug"

& "$BIN\arch-view.exe" diff $REPO `
  --from 3df00f4e64fd013ef8bc789b0be0496a033f4d3d `
  --to   ca4bf11670bdbc29bd5fe413cfa3cedba02c2e4e `
  --no-fade --auto-close-ms 800

git -C $REPO show --name-status --format='' ca4bf11670bdbc29bd5fe413cfa3cedba02c2e4e
```

实测 stdout（原样）：

```text
diff: from=3df00f4e64fd013ef8bc789b0be0496a033f4d3d to=ca4bf11670bdbc29bd5fe413cfa3cedba02c2e4e added=6 removed=2 modified=2 renames=0
```

> 该读数依赖 `arch-view diff` 的构建与界面行为；答案在 HEAD 上实测，合并后需在改后二进制上复核。

`git show --name-status` 实测（原样，用于核对路径与类别）：

```text
ca4bf11670bdbc29bd5fe413cfa3cedba02c2e4e 2024-07-31 refactor(cjs): resolver organized in multiple files

D	src/cjs/api/module-resolve-filename.ts
A	src/cjs/api/module-resolve-filename/index.ts
A	src/cjs/api/module-resolve-filename/interop-cjs-exports.ts
A	src/cjs/api/module-resolve-filename/is-from-cjs-lexer.ts
A	src/cjs/api/module-resolve-filename/preserve-query.ts
A	src/cjs/api/module-resolve-filename/resolve-implicit-extensions.ts
A	src/cjs/api/module-resolve-filename/resolve-ts-extensions.ts
M	src/cjs/api/register.ts
D	src/cjs/api/resolve-implicit-extensions.ts
M	src/esm/api/register.ts
M	tests/fixtures.ts
```

### 标准答案

**新增（6）**

| 模块路径 | 符号数 | 依赖边数 |
| --- | --- | --- |
| `src/cjs/api/module-resolve-filename/index.ts` | 3 | 9 |
| `src/cjs/api/module-resolve-filename/interop-cjs-exports.ts` | 3 | 1 |
| `src/cjs/api/module-resolve-filename/is-from-cjs-lexer.ts` | 2 | 0 |
| `src/cjs/api/module-resolve-filename/preserve-query.ts` | 2 | 5 |
| `src/cjs/api/module-resolve-filename/resolve-implicit-extensions.ts` | 1 | 4 |
| `src/cjs/api/module-resolve-filename/resolve-ts-extensions.ts` | 2 | 6 |

**删除（2）**

| 模块路径 | 符号数 | 依赖边数 |
| --- | --- | --- |
| `src/cjs/api/module-resolve-filename.ts` | 7 | 10 |
| `src/cjs/api/resolve-implicit-extensions.ts` | 2 | 5 |

**修改（2）**

| 模块路径 | 符号数 | 依赖边数 | 结构变化原因 |
| --- | --- | --- | --- |
| `src/cjs/api/register.ts` | 3 → 3 | 10 → 10 | 出边集合变化（`dependency-edges`） |
| `src/esm/api/register.ts` | 2 → 2 | 6 → 6 | 出边集合变化（`dependency-edges`） |

**重命名（0）**

```
renames = 0
```

### 为什么 `tests/fixtures.ts` 不在“修改”里

`git show --name-status` 显示 `M tests/fixtures.ts`，但结构 diff 只算“符号数变化或出边集合变化”。该文件的符号数与出边集合都没变，所以 UI 的“修改”里没有它。它不是参与者漏读，而是口径：**内容改动 ≠ 结构修改**。

同理，`package.json`、`pnpm-lock.yaml` 等非源文件不在模块集合里，不应算进任何一类。

### 通过条件

- 三类集合分别与 key 完全一致，类别归属正确；无多报、无错报；`renames` 不要求报（为 0）。
- 路径必须能区分两个 `register.ts`：`src/cjs/api/register.ts` 与 `src/esm/api/register.ts`；只写 `register.ts` 视为不完整（除非同时给出了可区分的目录）。
- 新增/删除/修改的**数量**不作为得分项（标题栏已泄露）；只评路径集合与类别。

**部分分（仅诊断）：**

```
F1_added    = 2 * |A ∩ K_added|    / (|A| + |K_added|)
F1_removed  = 2 * |R ∩ K_removed|  / (|R| + |K_removed|)
F1_modified = 2 * |M ∩ K_modified| / (|M| + |K_modified|)
partial     = mean(F1_added, F1_removed, F1_modified)   # 空集时按下述规则
success     = all three F1 == 1.0 and not timeout
```

若参与者某一类为空但 key 非空，该类 F1=0。若 key 某类为空（本 T4 没有空类），该类按 1 计。

**常见错误与归因：**

| 观察 | 归因 |
| --- | --- |
| 只报标题栏的 6/2/2，不报路径 | `V-DIFF`/`F-FLOW`；`ui_leak_t4_counts=true`；不算通过 |
| 把 `tests/fixtures.ts` 报成修改 | `I-DIFF`（没有说明“内容 vs 结构”口径）或 `S` |
| 把拆分误报成“重命名” | `V-DIFF`（rename 识别/身份误读） |
| 把两个 `register.ts` 合成一个 | `V-DIFF`（同名模块不可区分） |
| 把 `package.json`/lockfile 算进来 | `S`（范围误读）或 `V-DIFF` |
| 行太多/字太小/看不清 | `V-DIFF` |

### 结构 diff 的复核方法（第三人可复现）

`arch-view diff` 只打印计数；要复核“修改”里具体是哪两个，按 commit_diff 的口径比较两棵树：

```powershell
$UX   = "D:\project\1107-ux"
$REPO = "$UX\out\usability-corpus\tsx"
$BASE = "$UX\out\usability-corpus\t4"
$BIN  = "$UX\target\debug"
New-Item -ItemType Directory -Force -Path "$BASE\from","$BASE\to" | Out-Null

git -C $REPO archive 3df00f4e64fd013ef8bc789b0be0496a033f4d3d | tar -x -C "$BASE\from"
git -C $REPO archive ca4bf11670bdbc29bd5fe413cfa3cedba02c2e4e | tar -x -C "$BASE\to"

& "$BIN\arch-cli.exe" index     "$BASE\from" --out "$BASE\index-from.json"
& "$BIN\arch-cli.exe" callgraph "$BASE\from" --out "$BASE\cg-from.json"
& "$BIN\arch-cli.exe" index     "$BASE\to"   --out "$BASE\index-to.json"
& "$BIN\arch-cli.exe" callgraph "$BASE\to"   --out "$BASE\cg-to.json"
```

然后按模块比较：源文件集合（新增/删除）、符号数（`callgraph.symbols` 按 `module` 计数）、出边集合（`index.edges` 按 `source` 分组，比较 `target|kind` 集合）。本次实测得到：

```
added=6, removed=2, modified=2, renames=0
modified rows = src/cjs/api/register.ts, src/esm/api/register.ts
```

与 `arch-view diff` 的 stdout 一致。

---

## T5：边与步进的语义

### 取答案命令

T5 不依赖仓库读数；它测的是界面语义。用 T1 的窗口：

```powershell
& "$BIN\arch-view.exe" map $REPO `
  --from "src/cli.ts#module-toplevel" `
  --to "src/utils/ipc/get-pipe-path.ts#function:getPipePath"
```

### 标准答案（契约型，不是仓库读数）

1. **被高亮/加粗的边** = 当前步的**静态调用关系**：`src/cli.ts#module-toplevel` 调用 `src/utils/ipc/server.ts#function:createIpcServer`，后者调用 `src/utils/ipc/get-pipe-path.ts#function:getPipePath`。
   - **不是**数据流（不是“数据沿边流动”）。
   - **不是**控制流/运行时执行顺序（不是“程序运行时真的先执行这个、再执行那个”）。
   - 边在静态结构里一直存在；高亮只是“当前正在讲这一步”。
2. **步进顺序** = **最短静态调用链**的展示顺序（BFS 最短路径），**不是**运行时执行顺序。

依据：

- 票 06 L34：“路径：只走调用边；BFS 最短路径；相邻目标按 `(target, kind, line, column)` 字典序排序，先到先得。”
- 票 07 L31：`current_edge` 只返回“进入当前步的那条边”；路径是静态调用链的步进。
- 研究文档 §2.4：“任何沿调用边持续运动的粒子，都会被一部分用户读成‘数据在流动’或‘控制流在传递’，而 Strata 的边只是**静态调用关系**。”

### 评分 rubric

| 分 | 判定 |
| --- | --- |
| 2（通过） | 边明确说成“调用关系/谁调用谁/静态结构”；步进明确说“不是运行时执行顺序”或“只是最短调用链的展示顺序” |
| 1（部分） | 只说“连接/依赖/关系”，经追问能澄清为静态调用；或只答对其中一问 |
| 0（失败） | 说成数据流、控制流、运行时执行顺序、参数/返回值流动；或两问都错 |

**通过条件：** 总分 2 且未超时。若使用了 A/B/C/D 选项，记 `prompted=true`；prompted 与自由回答分开统计（样本更小）。

**常见错误与归因：**

| 观察 | 归因 |
| --- | --- |
| “数据在边上流动” | `S-DATAFLOW` |
| “程序运行时按这个顺序执行” | `S-EXEC` |
| “调用顺序/依赖顺序”但能澄清静态 | 部分分；`S-AMBIG` |
| 因高亮只有一条边，误以为“只有这一个调用” | `V-PATH`（缺少全路径上下文） |
| 看不到边/看不清高亮 | `V-PATH` |

### 与原研究 T5 的差异（必须报告）

原研究文档的 T5 问法是“动画里沿边移动的点代表什么：数据流、执行顺序、还是静态调用关系”。当前 UI **没有沿边移动的粒子**（R1 明确不做，且未实现）。本包只能测“被高亮/加粗的当前边”和“步进顺序”。

因此：

- 本轮可以测：当前边高亮是否被读成数据流/执行顺序；步进是否被读成运行时顺序。
- 本轮**不能**测：沿边移动粒子造成的误读率。
- 报告里必须把“粒子误读率”列为**未验证**，不能拿本轮数据替代。

---

## 变更日志（key 冻结后只能追加，不能改写本轮）

| 日期 | 版本 | 变更 | 旧读数 | 新读数 | 理由 | 是否作废本轮 |
| --- | --- | --- | --- | --- | --- | --- |
| 2026-11-08 | UK-2026-11-08-01 | 初始冻结 | — | 见上 | 票 12 + 用户 2026-11-08 的 5 任务口径 | 否 |
