# 10: 性能与内存基线归档

**What to build:** 维护者能复现一组固定的性能读数：两个结构样本仓的冷索引墙钟、历史默认窗口墙钟、峰值内存，以及超限仓库的处理行为；读数与硬件信息一起归档，用于判断预算是否仍成立。

**Blocked by:** 02（模块解析）、03（层推断）、04（符号与调用图）、08（历史采样）

**Status:** partial（索引墙钟、超限拒绝与 512MB 内存门禁实测通过；历史窗口读数未取得、硬件基线未确认，未验证）

**固定样本：** `privatenumber/tsx` @ `4473971f4b3261de253209b148d8fdc43269fcc4`、`excalidraw/excalidraw` @ `701b901a0f1ba3d0b390d9cdf63d112e25abac4b`；合成 >5k 文件 fixture。

- [x] `cargo run --release -p arch-cli -- index <tsx样本>` 3 次运行，中位数 ≤2s；excalidraw 中位数 ≤10s（两者均以推荐硬件基线为准，**待用户确认**）；每次读数写入 `out/perf.json`，含硬件、工具版本、样本 commit。实测：tsx 中位数 69.4ms（第二批 69.5ms），excalidraw 中位数 278.3ms（第二批 293.4ms）。
- [ ] `cargo run --release -p arch-cli -- history --repo <excalidraw样本> --max-commits 500` 3 次运行，中位数 ≤10s（**待硬件基线与用户确认**）。**未验证**：`tests/corpus/excalidraw` 是归档副本，没有 `.git`，无法复现 first-parent 历史窗口；需真实克隆。
- [x] 峰值内存门禁：`eval/perf-baseline.ps1` 以 OS `PeakWorkingSet64`（`osPeakWsMb`，进程驻留集/RSS 峰值）为门禁口径，默认硬上限 512MB，超限退出 1；轮询 `WorkingSet64`（`polledWsMb`）只作回退，因为它是采样下界（实测 tsx 首轮轮询 5.5MB、OS 峰值 6.6MB）。实测 Green：`-Runs 3` → exit 0，tsx 6.6MB / excalidraw 4.9MB；实测 Red：`-MaxPeakMb 1` → exit 1，逐样本报超限。未用 Linux `/usr/bin/time -v` 复核。
- [x] 超限仓库（合成 >5k 文件或 >20k 符号）返回明确拒绝或降级：退出码非 `0` 或产物带 `degraded=true`，且说明实际规模；不得静默输出「达标」。实测 5001 文件 → exit 2、stdout 空、stderr「目标仓库源文件数 5001 超过上限 5000，已拒绝索引」。
- [x] 归档文件可比较：同一机器连续运行 3 次的中位数波动 ≤20%（波动更大时标注环境噪声并保留原始读数）。两批中位数波动：tsx 0.14%、excalidraw 5.4%；单批内 tsx 首轮 93.6ms 属冷启动噪声，原始读数保留。
- [x] 失败判据：超过墙钟或内存阈值且未标注待确认/环境限制、OOM 崩溃、超限仓库被静默处理、归档缺硬件信息。
- [x] 自动化状态：墙钟自动化（单平台 CI）；Windows 内存门禁已自动化（`eval/perf-baseline.ps1`，超标 exit 1）；Linux/macOS 峰值内存未复核，不设跨平台门禁（原因见 `docs/acceptance-gaps.md`）。

## 验收记录（2026-11-08，C 档）

### 预算先行（实现前固定）

- 预算：tsx index ≤2000ms、excalidraw index ≤10000ms、excalidraw history ≤10000ms、峰值内存 512MB（待确认）、3 次运行取中位数、同批波动 ≤20%。
- 测量工具：`eval/perf-baseline.ps1`（release 构建、3 次运行、`WorkingSet64` 轮询 + `PeakWorkingSet64`、墙钟、硬件与样本 commit 写入 `out/perf.json`）。

### 读数（实测）

硬件：`AMD Ryzen 7 8845H w/ Radeon 780M Graphics`，16 逻辑 CPU，`Microsoft Windows NT 10.0.26300.0`，PowerShell 7.6.9；debug → release 构建，工具版本 0.1.0。

- **构建类型：release**（本票全部墙钟/内存读数；跨票比较性能时必须对齐 release）。

| 样本 | 批次 | 三次墙钟 (ms) | 中位数 (ms) | 最大值 (ms) | 峰值 WorkingSet (MB) | 预算 |
| --- | --- | --- | --- | --- | --- | --- |
| tsx index | 1 | 93.6 / 69.4 / 54.7 | 69.4 | 93.6 | 5.7 | ≤2000 |
| tsx index | 2 | 101.4 / 69.5 / 66.5 | 69.5 | 101.4 | 6.7 | ≤2000 |
| excalidraw index | 1 | 278.3 / 288.8 / 277.9 | 278.3 | 288.8 | 5.9 | ≤10000 |
| excalidraw index | 2 | 337.9 / 293.4 / 289.8 | 293.4 | 337.9 | 6.0 | ≤10000 |

- 批次间中位数波动：tsx `(69.5-69.4)/69.4 = 0.14%`；excalidraw `(293.4-278.3)/278.3 = 5.4%`，均 ≤20%。
- 单批内 tsx 首轮 93.6ms（第二批 101.4ms）明显高于后两轮；保留原始读数，标注为首次运行冷启动/缓存噪声，不据此改判据。
- 超限拒绝：合成 5001 个 `.ts` 文件 → exit 2、stdout 空、stderr 报 `5001` 与上限 `5000`。
- 归档：`out/perf.json`（第一批）、`out/perf-run2.json`（第二批）；摘要入 `eval/baseline/current.json`。

### 512MB 内存门禁（2026-11-08 补测）

- 口径差异：Windows 轮询 `WorkingSet64` 只拿到采样点，可能漏掉短峰；改为断言 OS 生命周期峰值 `PeakWorkingSet64`（`osPeakWsMb`），它代表进程驻留集/RSS 的峰值。实测 tsx 首轮 `polledWsMb=5.5` 而 `osPeakWsMb=6.6`，两者确实不一致，因此门禁只认 `osPeakWsMb`，轮询值仅在没有 OS 峰值时兜底。
- Green：`pwsh -File eval/perf-baseline.ps1 -OutJson out/perf.json -Runs 3` → exit 0，`memory gate: passed (all peaks < 512Mb)`；tsx 6.6MB、excalidraw 4.9MB。
- Red（反向用例）：`pwsh -File eval/perf-baseline.ps1 -MaxPeakMb 1 -OutJson out/perf-gate-red.json -Runs 1` → exit 1，逐条报 `tsx/index: peak 5Mb > 1Mb`、`excalidraw/index: peak 4.8Mb > 1Mb`。
- 产物：`out/perf.json` 新增 `memoryGate.limitMb/metric/checked/passed/violations`；摘要同步到 `eval/baseline/current.json`（`memoryCapConfirmed=true`）。

### 已知偏差与未验证

- **历史窗口未测**：excalidraw 的 L2 缓存是无 `.git` 的归档副本，`history --max-commits 500` 无法运行；没有真实克隆，故该项不按通过计。
- **内存门禁只在 Windows 实测**：门禁口径为 OS `PeakWorkingSet64`，已在 Windows 得到 Green/Red 两向读数；Linux `/usr/bin/time -v` 与 macOS 未复核，跨平台硬门禁仍未建立。
- **超限只有文件数维度**：现有拒绝基于 `DEFAULT_MAX_FILES = 5000`；`>20k 符号` 维度没有独立门禁（文件数门禁先行覆盖）。
- **冷启动口径**：`index` 墙钟包含进程启动，但样本仓较小，启动开销占比不可忽略；未单独拆分。
- **硬件基线未确认**：读数来自本机，是否作为 CI 基线待用户确认。

## 硬化第二件：四条用户可见路径端到端读数（2026-11-09）

### C 档预算（在改动前固定）

| 路径 | 口径 | 预算（median） | p95 预算 |
| --- | --- | --- | --- |
| `index`（纯 core，CLI） | 进程墙钟 | ≤500ms | ≤800ms |
| `map` 首屏 | `main:start → frame:presented`（窗口内首帧） | ≤1200ms | ≤2000ms |
| `map` 内容 | `main:start → content:presented` | ≤3000ms | ≤4000ms |
| `timeline` 首屏 | `main:start → frame:presented`（加载态首帧） | ≤1500ms | ≤2500ms |
| `timeline` 内容 | `main:start → content:presented` | ≤3000ms | ≤4000ms |
| `diff` 首屏 | `main:start → frame:presented`（加载态首帧） | ≤1500ms | ≤2500ms |
| `diff` 内容 | `main:start → content:presented` | ≤12000ms（本轮不做算法优化） | ≤16000ms |
| 取消 | 点击「取消准备」到进程退出 | ≤一个阶段收尾（当前最长单阶段 <2s） | — |

- 测量工具：`eval/path-timing.ps1`（release、默认 3 次、逐次读数 + median/min/max/p95；硬件与命令写入 `out/path-timing-*.json`）。
- 统一口径：时间戳由外部进程记录 `phase:` 行到达时间；视图源码不读墙钟。`index` 只测 CLI 进程墙钟，UI 路径测窗口内标记。
- 跨票比较限制：本表全部为 **release**。票 17 的 `10589ms` 是 debug `cargo test` 的单次读数，不能与本表 release 读数混算。

### 读数（实测，release，3 次取中位数）

硬件：`AMD Ryzen 7 8845H`，16 逻辑 CPU，`Windows NT 10.0.26300.0`，PowerShell 7.6.9；工具 0.1.0。
样本：`tests/corpus/excalidraw`（1328 文件）；timeline 用 `$env:TEMP\tsx-hist`（617 first-parent 提交真实克隆）。

| 路径 | 命令 | 逐次读数 → median / p95 | 预算结论 |
| --- | --- | --- | --- |
| index（最终热批次） | `arch-cli index tests/corpus/excalidraw` | 320 / 267 / 301 → 301ms / 320ms | 通过 |
| index（release 构建后冷批次，保留） | 同上 | 984 / 863 / 306 → 863ms / 984ms | **FAILED**（见下） |
| index（冷批次后 5 次热跑） | 同上 | 292 / 293 / 316 / 305 / 343 → 305ms | 通过（环境噪声对照） |
| map 首屏 | `arch-view map tests/corpus/excalidraw --auto-close-ms 4000` | 622 / 446 / 644 → 622ms / 644ms | 通过 |
| map 内容 | 同上 | 1664 / 1399 / 1595 → 1595ms / 1664ms | 通过 |
| timeline 首屏 | `arch-view timeline $env:TEMP\tsx-hist --auto-close-ms 6000` | 778 / 509 / 457 → 509ms / 778ms | 通过 |
| timeline 内容 | 同上 | 1735 / 1331 / 1277 → 1331ms / 1735ms | 通过 |
| diff 首屏 | `arch-view diff $env:TEMP\excalidraw-diff --from HEAD~1 --to HEAD --auto-close-ms 12000` | 529 / 550 / 383 → 529ms / 550ms | 通过 |
| diff 内容 | 同上 | 6978 / 6890 / 6769 → 6890ms / 6978ms | 通过（预算内） |

- 表中 map/timeline/diff/index 为**同一最终构建**的连续批次；各路径仍是独立进程，不跨样本混算。
- **index 冷批次失败如实保留**：release 构建后立刻跑 3 次，前两次 984/863ms、第三次 306ms，median 863ms > 500ms、p95 984ms > 800ms，`-EnforceBudget` exit 1。随后 5 次热跑 292–343ms。判为文件系统/杀软冷缓存噪声，不是索引结构性变慢；**不因此调高预算**。自动化门禁目前会把这种冷启动判红，这是预期的保守行为。
- `-EnforceBudget` 对最终批次的 index/map/timeline/diff 均 exit 0；冷批次原始读数仍保留。

### 超 2 秒路径与性质判断

- **`diff` 内容 6.9s**：结构性慢。分项证据：单快照 `index` 301ms + `callgraph` 683ms ≈ 0.98s，两个快照约 1.97s；其余约 4.9s 是两次完整 tree 导出（`ls-tree` + `cat-file` + 1328 文件写盘 ×2）。规模对照：95 文件的 `tsx-hist` 双 commit diff 中位数 2108ms，1328 文件的 excalidraw 本轮 6890ms，随文件数增长。首轮冷缓存可到 16.2s，但改动前第 2/3 轮稳定在 9.4–9.6s，不是「首次开销」单独造成。
- **`map` 内容 1.6s（改动前首帧 2.85s）**：混合。固定 GPUI 启动/开窗约 0.4–0.62s（3 文件 fixture 内容首帧 484ms 对照）；其余是 excalidraw 的 layers+callgraph 结构成本（callgraph 568–622ms、small fixture 16ms）。改动前 >2s 的首帧主要是「准备完才建窗」，现在首帧 0.62s，内容 1.6s 已在 3s 预算内。
- **`timeline` 内容 1.33s**：617 刻度采样阶段约 504ms，结构随刻度数增长；加载态首帧 0.51s，用户不会面对静止画面。

- **结论**：改动后唯一仍 >2s 的路径是 `diff` 内容（6.9s）；`map`/`timeline` 内容均已降到 2s 以下。`diff` 的结构性成本保留，本轮先用加载态 + 可取消兜住用户可见体验。

### 产品反馈修复（优先于算法优化）

- 改动前：`map` 在数据准备完成前不建窗，首帧 2.85s；`diff` 在 `diff_commits` 完成前不建窗，首帧 10.39s，期间用户看到的是**没有窗口**，不是加载态。判定为产品缺陷。
- 改动后：`map`/`diff` 都先开窗并显示 `LoadingShell`（标题 + 当前阶段 + 「可随时取消」+ 取消按钮），数据就绪后自动切到内容视图；本轮首帧分别 0.62s / 0.53s。
- 可取消：`diff_commits_cancellable` 在阶段边界与每 32 个导出文件检查取消标志；命中后返回 `CommitDiffError::Cancelled` 并正常清理临时快照。测试 `diff_honours_cancel_flag_before_work` 通过；`diff_reports_progress_stages_in_order` 锁定阶段顺序。
- 像素证据：`out/pixel-evidence/diff-loading.png`、`map-loading.png`（加载文案与取消按钮）；`diff-content.png`、`map-content.png`（切换到内容视图）。

### diff 优化判据（实现前固定，2026-11-09）

优化目标只针对「同一棵树被完整导出两次」的重复写盘，不改解析、不改 `renameHint` 阈值、不改 diff 判定。

**等价性判据（必须逐字节相等）**

- 对同一对 commit，优化路径（`DiffSnapshotStrategy::Reuse`）与旧全量导出路径（`DiffSnapshotStrategy::Full`，保留为参照实现）的 `CommitDiffArtifact` 经 `serde_json::to_string` 序列化后必须**逐字节相等**。
- 覆盖样本：合成 fixture（added / removed / modified / 疑似重命名同时出现）、workspace 自身 `2b66deb → 29f8e27`、excalidraw 语料双 commit（`#[ignore]` 手工跑）。
- 覆盖字段：`added` / `removed` / `modified` 三类记录、`renameHint`、`renames`（含 `pathSimilarity` / `symbolOverlap` 浮点值）、`stats`。
- 三类边界各设专门用例：纯重命名、纯删除、纯新增，均要求新旧路径字节相等，且 `renameHint` 与相似度判定不变。
- 取消行为一致：同一个取消标志在同一阶段边界被置位时，两条路径都返回 `CommitDiffError::Cancelled`，且临时快照照常清理。

**事实复用边界（不可越界）**

- 只复用「文件内容」：`to` 树中与 `from` 树同 path、同 mode、同 blob oid 的条目用硬链接复用（跨卷回退为从 `from` 快照复制），其余条目仍从 git 拉取。
- 不复用事实：`from` / `to` 两侧仍各自独立运行 `build_index` + `build_call_graph`，层归属与模块 ID 一律按各自 commit 的路径计算。做不到这一点就不宣称等价。

**预算（先验，非事后）**

- 热 diff 内容就绪（`main:start → content:presented`）：median ≤5000ms、p95 ≤6000ms（旧读数 median 6890ms）。
- 分项估算：`from` 全量导出 ≈2.4s + `to` 未变化文件硬链接 ≈0.15s + 两侧 facts ≈2.0s + 比对 ≈0.1s ≈4.6s。
- 冷 diff 另行计算，不与本轮热读数混算；`index` 冷缓存失败读数继续保留。
- 收益门槛：预期节省 ≈2.3s（两次导出 → 一次导出 + 链接）。实测节省若 <1.15s（预期一半），如实报失败，不调阈值、不改判据。

### diff 优化实测结论（2026-11-09）

**做到的**

- 等价性 **PASS**：`DiffSnapshotStrategy::Reuse` 与 `Full` 在混合 fixture（added 3 / removed 2 / modified 1 / renames 1）、纯重命名、纯删除、纯新增、workspace `2b66deb → 29f8e27`、excalidraw 语料双 commit 上，`serde_json` 序列化产物逐字节相等；`renameHint`、`renames` 的 `pathSimilarity` / `symbolOverlap` 与 `stats` 均未变。
- 取消行为一致：同一取消标志在 `MaterializingTo` 边界置位时，两条路径都返回 `CommitDiffError::Cancelled`（测试 `reuse_and_full_export_cancel_at_same_stage_boundary`）。
- 两侧 facts 仍独立计算：`from` / `to` 各自跑 `build_index` + `build_call_graph`，未做跨侧事实复用，层归属与模块 ID 按各自 commit 路径。
- 命令：`cargo test -p arch-core --test commit_diff`（15 passed / 0 failed / 2 ignored）；`cargo test --release -p arch-core --test commit_diff large_corpus_reuse_matches_full_export -- --ignored --nocapture`。

**没做到的（预算未达标，收益低于预期一半）**

- 热 diff 内容就绪 **FAILED**：release + `eval/path-timing.ps1 -Path diff -Runs 3 -AutoCloseMs 20000` → 8684 / 8604 / 8563ms，median 8604ms、p95 8684ms，预算 5000/6000ms。
- 同进程 3 轮交替对照（同一临时仓、同一进程，release，`#[ignore]` 手工跑）：

| 轮次 | Reuse 总耗时 (ms) | Full 总耗时 (ms) | 节省 (ms) |
| --- | --- | --- | --- |
| 1 | 7222 | 7554 | 332 |
| 2 | 7532 | 7215 | -317 |
| 3 | 7239 | 7571 | 332 |

  median：Reuse 7239ms，Full 7554ms，节省 **332ms（4.4%）**，低于预期一半（1150ms）。第 2 轮 Reuse 反而更慢，差异在噪声量级。

- 分项打点（release，一轮代表值）：`from` 全量导出 657–897ms；`to` 导出 Reuse 1160ms vs Full 971ms（**复用路径在导出阶段反而更慢**）；两侧 facts 各约 2.0s（合计约 4s）；`build_artifact` + 临时快照删除约 0.64s。
- **原判断被推翻并保留**：此前「两次完整 tree 导出约 4.9s」是「总耗时 6.9s − facts 约 2s」的推断，不是实测。实测单次导出仅约 0.65–1.16s，成本主要是每文件的 `create_dir_all` / 写入等文件系统操作，blob 读取占比很小；两次 facts（约 4s）才是最大头。因此「只导出变化文件 + 硬链接复用未变化内容」无法达到预期收益，也不足以让热 diff 进 5000ms 预算。**不调阈值、不改判据**。
- 环境噪声：本轮 UI 测量 `app init` 约 1.0s（改动前批次约 0.5s），`first visible` 约 1.0s；UI 总耗时 8.6s 与旧 6.9s 不是同一负载下的读数，故以同进程交替对照作为收益判据。

**未验证 / 待决策**

- 未改 `eval/path-timing.ps1` 的 diff 预算（仍是 12000/16000），避免把失败的预算悄悄改绿；若保留本轮判据，应另起提交把预算改为 5000/6000 并记录旧读数、新读数与理由。
- 未做 facts 级复用（跨侧复用 `snapshot_facts`）——层归属可能受其他文件变化影响，不在本轮判据允许范围。
- 当前实现保留 Reuse 为默认路径，等价性成立但收益仅 4.4%；是否保留该复杂度由维护者决策，代码与票中已如实记录收益不足。

### 未做 / 未验证

- 本轮未做 diff 算法优化。候选方案与预期收益（先立项再动手）：按 `git diff --name-status` 只导出变化文件，未变化文件复用上一快照事实或持久化 facts 缓存；预期热 diff 从 6.9s 降到 1–2s，冷 diff 仍需事实构建。收益未实测，不写成结论。该候选被本轮判据取代：本轮只做内容级硬链接去重，facts 仍两侧独立计算。
- 取消按钮的鼠标点击未做自动化像素/交互测试；当前证据是模型测试 + 代码路径 + 关闭窗口即退出的行为。跨平台（Linux/macOS）四条路径未测。

