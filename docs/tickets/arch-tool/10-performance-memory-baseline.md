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

**结论：已尝试并回退**

- 回退提交 `12bff53`（revert `15cfe4d`）：文件内容硬链接复用与第二套策略测试全部撤回，`DiffSnapshotStrategy` / 参照实现不再保留。理由：等价性成立，但收益仅 4.4%（332ms），低于预期一半（1150ms），且 `MaterializingTo` 阶段硬链接（1160ms）比全量导出（971ms）更慢；第二套策略不值得为噪声级收益承担维护成本。
- 回退后回归基线：`cargo test -p arch-core --test commit_diff` 恢复为回退前用例集，工作区可干净编译。
- 预算 5000ms 判红保留，`eval/path-timing.ps1` 的 diff 预算未改（仍 12000/16000），不把失败预算悄悄改绿。
- **票 10 的 C 档产品判据未破**：diff 内容 ≤12000ms median / ≤16000ms p95 仍通过；5000ms 是本轮新立的优化目标，不替代原 C 档。
- 保留上一节「原判断被推翻」的推断原文与实测分项，供后续方案引用。

### facts 级复用：判据、实测与回退（2026-11-09）

**判据（实现前按上一轮实测分项预估；文本后补，读数为本轮首次测量）**

- 复用条件：`to` 与 `from` **path + blob oid 相同**，且用 `from` 侧导入列表在 to 快照上重新解析出的依赖边集合也相同；任一不满足就回填真实内容重算。解析配置（`package.json` / `tsconfig.json` / `jsconfig.json` / `pnpm-workspace.yaml`）变化时整体退回全量构建。
- 实现方式：未变化源文件先写空占位（保留路径存在性），变化/新增文件写真实内容，删除路径不写；在混合快照上跑 index + callgraph，可复用路径的符号数与符号名、依赖边从 `from` 事实覆盖，其余用混合快照结果。
- 等价性：沿用逐字节判据（混合 fixture、纯重命名/删除/新增、workspace 对、excalidraw 对），并新增三类对抗用例：未变化文件导入目标被删除 / 被重命名 / 新增文件改变解析顺序（`./run` 从 `run/index.ts` 切到 `run.ts`）。
- 收益门槛：同进程 3 轮交替对照，median 节省 ≥20%（约 1.4s）；达不到就回退，不调阈值。
- 预期分项（依据上一轮实测）：`from` 导出约 1.0s、`from` facts 约 1.8–1.9s（不变）；`to` 物化约 1.2–1.4s（空占位 + 边复核 + 少量回填）、`to` facts 约 0.2s（只为变化文件建符号）、build + 清理约 0.6s。预期总耗时 4.8–5.3s，对照 7.2–7.4s，预计节省 1.8–2.6s。

**等价性：PASS**

- `cargo test -p arch-core --test commit_diff`：18 passed / 0 failed / 2 ignored。混合 fixture、纯重命名/删除/新增、workspace `2b66deb → 29f8e27` 均逐字节相等；三类对抗用例也都逐字节相等（未变化导入方的边被正确重算）。
- excalidraw 语料（`cargo test --release ... --ignored --nocapture`）：Reuse 与 Full 序列化产物逐字节相等。

**实测（release，同进程交替，两批各 3 轮）**

| 批次 | Reuse 总耗时 (ms) | Full 总耗时 (ms) | 逐轮节省 (ms / %) | median 节省 |
| --- | --- | --- | --- | --- |
| 1 | 5657 / 5982 / 5774 → 5774 | 7177 / 7266 / 7380 → 7266 | 1520 (21.2%) / 1284 (17.7%) / 1606 (21.8%) | 1492ms（20.5%） |
| 2 | 6862 / 6782 / 6184 → 6782 | 8240 / 8132 / 8099 → 8132 | 1378 (16.7%) / 1350 (16.6%) / 1915 (23.6%) | 1350ms（16.6%） |

- 分项：`to` facts 从全量 1.79–2.05s 降到 0.17–0.27s（省约 1.6–1.85s）；但 `to` 物化从 1.03–1.33s 升到 1.26–1.65s（空占位写盘 + 边复核，多约 0.25–0.4s）。
- 合并 6 轮：成对 median 节省 **1449ms（19.5%）**；按中位数之比（Reuse 6083 vs Full 7734）为 1651ms（21.4%）。两批 median 分别为 20.5% 与 16.6%。
- UI 读数（`eval/path-timing.ps1 -Path diff -Runs 3 -AutoCloseMs 20000`）：first visible 1197 / 1059 / 1082 → 1082ms；prep 8138 / 7112 / 7344 → 7344ms；total to content 8162 / 7139 / 7372 → **7372ms median、8162ms p95**。5000ms 优化预算仍判红；票 10 的 C 档 diff ≤12000 / ≤16000 **仍通过**。

**结论：已尝试并回退**

- 按门槛「成对 median ≥20%」判定：合并 6 轮 19.5% < 20%，第二批仅 16.6%，未稳定达标；按「约 1.4s」判定合并 median 1449ms 刚过线。为避免凑数，按更严格口径判失败并回退事实复用实现与其对抗测试。
- 回退后保留本节全部读数、等价性结论与对抗用例设计，供后续方案复用；说明：真正的成本下降来自 `to` facts（约 1.8s），`to` 空占位物化的额外开销（约 0.3s）抵消了部分收益，若要再试应先把占位物化成本压下去。
- 试跑分支未提交；最终工作区回到回退后的基线（`12bff53` + 本节文档）。
- 回退后基线复测（`12bff53` 代码 + release 重建，`eval/path-timing.ps1 -Path diff -Runs 3 -AutoCloseMs 20000 -EnforceBudget`）：first visible 1062 / 1262 / 1306 → **1262ms median**；prep `diff_commits` 9730 / 10172 / 9696 → **9730ms median**；total to content 9757 / 10190 / 9724 → **9757ms median、10190ms p95**；`budget: passed`。本轮 app init 1.24s 高于上节读数环境（约 1.0s），故不与上节 7.3–8.2s 跨批次比较，仅作为回退后同机基线。
- **最终结论：diff 热路径停在约 9.7–9.8s（median 9757ms，p95 10190ms），不保留本轮任何复用优化**；票 10 的 C 档 diff ≤12000 / ≤16000 仍通过，5000ms 优化目标仍判红。

**方法论说明：批次间漂移与 20% 门槛的可判定性**

- 同一实现、同一语料的两批读数差异已达 20% 量级：`Full` 批 1 median 7266ms vs 批 2 median 8132ms（漂移约 12%），`Reuse` 5774ms vs 6782ms（漂移约 17.5%）；同一 UI 基线两轮读数 7372ms（上节）vs 9757ms（回退后复测）为 32%。批次间漂移可达 20% 以上、与待测效应同量级，**20% 门槛在该环境下不可判定**（无法从单批读数可靠区分「真实收益」与「环境漂移」）。
- 因此本轮**回退是基于复杂度判断**（第二套物化/复核路径 + 空占位写盘带来的维护成本与额外风险），**不是「收益不存在」的正面证据**。读数原样保留，若后续在更稳定的测量环境（固定负载、更长采样）重测，应重新判定而非引用本条作为收益结论。
- 本节不修改任何实测读数与判据；`5000ms` 优化预算仍判红，票 10 C 档产品判据不受影响。

### 未做 / 未验证

- 本轮 diff 算法优化均未保留：内容级硬链接复用（收益 4.4%）与 facts 级复用（合并成对 median 19.5%，未稳定达 20% 门槛）均已回退，读数与复现方式留在上两节。仍未做：持久化 facts 缓存（跨进程复用）、只导出变化文件的快照方案；两者都需另行立项并先写判据。
- 取消按钮的鼠标点击未做自动化像素/交互测试；当前证据是模型测试 + 代码路径 + 关闭窗口即退出的行为。跨平台（Linux/macOS）四条路径未测。

## 大图帧预算（802 模块，2026-11-09）

### 判据（测量前固定，先于本轮读数）

- **测量对象**：`tests/corpus/excalidraw`（802 模块）release 构建的 `arch-view map` 窗口。
- **判据性质**：**目标档**（先验目标，不是观测值）。动机：现有揭示动画约 12fps，同一绘制成本会落在拖拽、缩放、悬停上。
- **目标**：802 模块平移（持续重绘）帧间隔中位数 ≤33ms（≥30fps）、p95 ≤50ms；揭示动画期间同口径 ≤33ms（p95 ≤50ms）；静态首帧后空闲期不得持续重绘（观察窗口内新增帧数 = 0）。
- **基线（已观测，仅作对照，不参与判定）**：揭示 80 帧耗时 6.3–8.0s，帧间隔 78–100ms（约 12fps）。
- **失败判据**：平移中位数 >33ms 或 p95 >50ms；或定位为 GPUI 0.2.2 绘制路径固有限制且投入产出不划算时，必须给出证据并登记为已知限制，**不得为达标砍功能或降精度**；也不得事后修改本判据。
- **测量方法**：外部进程读 `phase: frame` 时间戳算帧间隔；`STRATA_AUTOPAN_FRAMES=N` 驱动确定性平移；`STRATA_RENDER_SKIP`（`bands,edges,nodes,labels` 子集）做受控实验，用差值定位成本来源。子阶段无法在 `arch-view/src` 内取墙钟（源码时间源判据），因此布局/绘制拆分以受控实验差值 + 每帧绘制计数为准。

### 读数（实测，2026-11-09，release）

工具：`eval/frame-timing.ps1`（外部读 `phase: frame` 时间戳）；语料 `tests/corpus/excalidraw`（802 模块 / 4363 边 / 6 层 / 802 标签），release 构建。

| 场景 | 窗口帧数 | 间隔中位数 | p95 | 约合帧率 | 备注 |
| --- | --- | --- | --- | --- | --- |
| 静态首帧后空闲 5000ms | 0（进程总共 1 帧） | — | — | — | 无操作不持续重绘，判据通过 |
| 揭示动画 | 81 帧 / 78 个间隔 | 83.94ms | 94.55ms | 11.9fps | 揭示总时长 6767ms；prep 1834ms、content 2006ms |
| 平移（batch 1） | 121 帧 / 118 个间隔 | 65.82ms | 81.67ms | 15.2fps | 自动平移 120 帧、8px/帧 |
| 平移（batch 2） | 121 / 118 | 53.50ms | 63.29ms | 18.7fps | 同机、同参数，批次间漂移约 23% |
| 平移（batch 3） | 121 / 118 | 81.54ms | 104.52ms | 12.3fps | 批次间漂移进一步扩大，读数保留不取均值 |

受控实验（同为平移 120 帧，`STRATA_RENDER_SKIP` 关闭对应绘制）：

| 配置 | batch 1 median / p95 | batch 2 median / p95 | batch 3 median / p95 |
| --- | --- | --- | --- |
| 全量 | 65.82 / 81.67 | 53.50 / 63.29 | 81.54 / 104.52 |
| 跳过 labels | 52.19 / 66.91 | 39.64 / 48.79 | 63.73 / 78.34 |
| 跳过 edges | 26.95 / 35.07 | 28.85 / 35.34 | 40.13 / 46.86 |
| 跳过 nodes | 22.28 / 28.18 | 26.94 / 33.36 | — |
| 空场景（nodes+bands+edges 全跳） | 14.72 / 19.46 | 14.90 / 20.27 | — |

### 定位证据

- **判据现状**：平移中位数 53.5–81.5ms、p95 63.3–104.5ms，未达 33/50ms 目标；揭示 83.9/94.6ms 亦未达。基线 12fps 与目标差约 2.5 倍。
- **固定开销地板**：空场景 14.7–14.9ms（约 67fps），即 GPUI 呈现/合成管线本身的每帧固定成本；这是任何优化的下限。
- **标签（802 个文本 run）**：批 1/2/3 的差值分别为 13.6 / 13.9 / 17.8ms，是稳定可归因的一项。
- **边（4363 条 path）**：差值 38.8 / 24.7 / 41.4ms，量最大且受批次波动影响最大。
- **节点盒（802 个 div）**：去掉 nodes 后（剩 bands+edges）只有 22.3–26.9ms，比空场景仅高 7–12ms；说明 802 个节点盒单独绘制不贵。
- **超加性交互**：节点+标签单独约 12–25ms（跳过 edges 时），边单独约 7–12ms（跳过 nodes 时），但两者同时存在时总成本 35–65ms，大于两部分之和。证据指向「802 个带裁剪/透明度/圆角的节点层叠加在边画布上」的合成成本，而非单纯的元素数量；需用下一轮受控实验（去掉节点 opacity/裁剪、边批处理）确认。
- **每帧重复工作（代码事实，配合计数）**：每帧克隆整个 map（802 节点 + 4363 边）、重建 4363 个 `EdgeDraw`、重建 `stage_of_layer` BTreeMap、重建 802 个节点元素树与 802 个文本 run；无按视口裁剪。首帧计数输出为 `nodes=802 edges=4363 bands=6 labels=802`。
- **未取得的分项**：GPUI 0.2.2 无公开的 layout/prepaint/paint 分项计时入口，且 `arch-view/src` 禁用时间源，故无法给出三段墙钟；以上归因全部来自受控实验差值与绘制计数。

### 候选方案（按改动量 / 预期收益 / 风险）

| # | 方案 | 改动量 | 预期收益 | 风险 |
| --- | --- | --- | --- | --- |
| A | 边批处理：同色/同宽/同 dash 的边合并进一个 PathBuilder（揭示关闭且 opacity 均匀时用 3 组：normal/adjusted/strong） | 中 | 高（4363 个 path → 3 个） | dash 与逐边 opacity 组合需保持；播放态仍需逐边透明度，需按条件分支 |
| B | 视口裁剪：只为与视口相交的节点/边建元素，标签只在 zoom ≥ 阈值时显示 | 中 | 高（默认 fit 缩放下大幅减少 802 节点与 802 标签） | 截图/像素基线会变；需要保留可点击命中（状态层不变） |
| C | 去掉每帧 map 克隆与重建：用借用/索引，缓存 stage_of_layer 与边几何 | 小 | 中（可能只省几 ms） | 播放态下几何随相机变化，缓存需按 viewport 失效 |
| D | 节点层降本：opacity=1 时不加 `.opacity()`；低缩放时省略标签与边框/圆角裁剪 | 小 | 中（待测，针对超加性交互） | 低缩放视觉变化；需像素/截图证据 |
| E | 文本布局缓存：同一文本 run 复用 shapeless 的排版结果 | 大 | 中（标签约 14ms） | GPUI 0.2.2 公开 API 是否支持复用未知，风险高 |

优先建议先做 A + D（改动可控、可做 A/B 受控实验），再评估 B；若 A+D 后仍达不到 33ms，则按判据登记为 GPUI 绘制路径限制并停止。

### A/D 视觉等价容差（测量前固定）

> 历史记录：D1/D2 已按结论回退（见「默认策略」），本节读数保留不删。

- **像素口径**：同进程同窗口尺寸，静态最终态截图（无揭示、无播放），通道差 >6 记为变化像素；比较面积默认整帧。
- **A（边批处理，默认候选）**：`tests/corpus/excalidraw` 与 `tests/fixtures/layers` 两个场景，changed% **≤0.50%** 且单像素最大通道差 **≤24**。依据：整帧约 5.1M 像素，0.50% ≈ 2.5 万像素，足以覆盖 4363 条 1px 线因绘制顺序变化产生的抗锯齿差异；最大通道差 24（约 9.4%）仅允许落在这些窄条带上，大面积色块变化会被 changed% 拦住。
- **D1（`opacity=1` 不设 `.opacity()`）**：预期纯优化，判据 changed% **≤0.01%** 且最大通道差 **≤2**；另需目视确认无差别。
- **D2（低缩放省略裁剪 `overflow_hidden`）**：属真实视觉改动，单独对照；判据 changed% **≤0.50%** 且最大通道差 **≤24**，并附目视结论。若超出或目视可辨，则作为可选开关（默认关），不作为默认行为。
- **纪律**：以上阈值先于本轮测量写入；任何超出即判失败并说明原因，不得事后放宽。A 的等价性判据不适用于文字/命中逻辑，只用于像素外观。

### A/D 实测（2026-11-09，release）

复跑命令（固定样本 `tests/corpus/excalidraw`，802 模块 / 4363 边 / 6 层）：

```
pwsh -File eval/frame-timing.ps1 -Mode ab -AbCycle "base,D1,A,AD1,base,D1,A,AD1,base,D1,A,AD1" -AbSegmentFrames 60
pwsh -File eval/map-pixel-compare.ps1 -Repo tests/corpus/excalidraw -OutDir out/map-pixel-excalidraw
pwsh -File eval/map-pixel-compare.ps1 -Repo tests/fixtures/layers -OutDir out/map-pixel-layers
```

测量口径（本轮修正，写在此处以固定方法）：

- 同进程按帧段轮换，段长固定 60 帧；按**帧下标**切段（第 i 段覆盖帧 `[60i, 60(i+1))`，段内 59 个间隔），不跨段混算，末尾多跑一段补齐。
- 自动平移改为 0/8px **往返**（原为单向 8px/帧）：避免后段内容移出屏幕导致帧成本虚假下降。
- 跨批汇总取「各批中位数的中位数」；本机存在外部 CPU 负载波动（26%–70%），因此以**同一进程内的批间对照**为准，绝对值仅作量级参考。

同进程交替对照（run `out/frame-timing-ab-default.json`）：

| 段 | 批次数 | median（ms，批中位数中位数） | p95（ms） | 相对全量 |
| --- | --- | --- | --- | --- |
| 全量 `base` | 4 | 69.13 | 84.68 | — |
| 仅 `D1` | 3 | 68.82 | 86.78 | −0.4%（噪声内） |
| 仅 `A` | 3 | 48.62 | 54.38 | −29.7% |
| `A+D1` | 3 | 46.76 | 58.90 | −32.4% |

第二组（run `out/frame-timing-ab-3x-c.json`，同样交替）：`base` 62.80 / 71.36，`A` 47.79 / 54.62，`AD1` 46.82 / 54.17。两组相对关系一致（A ≈ 全量 × 0.73，AD1 ≈ 全量 × 0.75）。

判据核对（票内先写的目标档：median ≤33ms、p95 ≤50ms）：

- `A+D1` median 46.76ms、p95 58.90ms，**未达标**；第二组 46.82 / 54.17 同样未达标。
- 结论不回调：33/50 保持原样，不做「接近即通过」。

视觉等价（先定容差后测；跨进程同配置重绘噪声实测 **0 像素**，见 `-RepeatBase`）：

| 场景 | 对照 | changed% | maxDelta | 超限像素 | 判定 |
| --- | --- | --- | --- | --- | --- |
| excalidraw | base vs A | 0.1686%（≤0.50%） | 89（>24） | 229 | **FAIL** |
| excalidraw | A vs AD1 | 0% | 0 | 0 | PASS |
| excalidraw | AD1 vs ADclip | 0% | 0 | 0 | PASS |
| layers | base vs A | 0.0227% | 21 | 0 | PASS |
| layers | A vs AD1 | 0% | 0 | 0 | PASS |
| layers | AD1 vs ADclip | 0% | 0 | 0 | PASS |

- A 首次实现把边按 `normal/adjusted/strong` 三组归并，改绘制顺序：excalidraw 0.9211% / maxDelta 204。改为**连续同风格合并、虚线逐条绘制**（保序）后降到 0.1686% / 89，仍超 maxDelta ≤24，按纪律判 FAIL。差异集中在 1px 边的抗锯齿覆盖（典型像素 `(46,59,72)` vs `(28,35,44)`）与少量橙色虚线的 dash 相位（`(127,93,60)` vs `(38,48,60)`），bbox 覆盖整个画布，不是局部块状变化。
- 上述 FAIL 按当时容差原样保留。2026-11-09 批准 A 默认开启时，新的**常态闸门**改为 changed% ≤0.30%、maxDelta ≤100（`eval/map-pixel-compare.ps1` 默认值），并新增几何等价断言；这不改写上面的历史判定。
- D1 两个场景都逐像素 0 差异，PASS；但帧时间收益为 0（68.82 vs 69.13ms），低于测量分辨率。
- D2 两个场景都逐像素 0 差异，PASS；仍是真实视觉改动，默认保持关闭。

默认策略（按判据执行，不放宽）：

- `A`（边批处理）：**已批准默认开启**（`STRATA_EDGE_BATCH` 默认 1）。回落开关：`STRATA_EDGE_BATCH=0` 恢复逐边绘制，用于需要精确栅格（逐条虚线相位、逐边绘制顺序）的场合。常态视觉闸门见下节：标准场景 changed% ≤0.30%、maxDelta ≤100；几何等价由 `crates/arch-view/tests/geometry_equivalence.rs` 独立断言。历史：2026-11-09 之前默认关闭，旧读数保留在「A 默认关时」。
- `D1`、`D2`：**已尝试、无收益、已回退**。两者在两个场景都是逐像素 0 差异，但帧时间收益均为 0（D1 68.82 vs 全量 69.13ms，低于测量分辨率）。代码里的 `skip_opacity` / `clip_nodes` 与对应开关已移除；A/B 读数保留在案，不做删除。
- `B`（视口裁剪）：按下一节判据验证；默认开启（`STRATA_VIEWPORT_CULL` 默认 1）。

### B 视口裁剪判据（2026-11-09，测量前固定）

- **语义**：只跳过「完全在视口外」的节点/边/标签；与视口相交（含 2px 保守外扩）的一律照原样绘制，命中测试仍用全量几何。
- **静止帧**：同一相机位、三个位置（`p1=0,0,1`、`p2=-240,-160,0.4`、`p3=-100,-400,0.25`），`base vs B` **changed% = 0 且 maxDelta = 0**。
- **叠加 A**：`A vs BA` **changed% = 0 且 maxDelta = 0**（裁剪不得新增差异；A 自身差异另计，不因 B 而放宽）。
- **平移过程**：允许边界元素进出，单独报 changed%；要求仍为逐像素 0，并目视确认无闪烁/拖影。
- **命中测试**：断言在 `crates/arch-view/tests/viewport_cull.rs`：任何视口内屏幕点都不得命中被剪节点；可命中节点不得被剪。
- **揭示/播放态**：裁剪按实际绘制位置（含揭示 offset）判定；受动画时序限制，外部无法逐帧对齐两种配置，故以「逐帧扫描 offset 的部分可见不变式」+ 最终态静止帧替代逐帧像素对照。
- **性能**：同进程交替 `base / B / BA`，≥3 批、段长 60 帧；目标 median ≤33ms、p95 ≤50ms，不达标如实报剩余。

### B 实测（2026-11-09，release）

复跑命令（固定样本 `tests/corpus/excalidraw`，802 模块 / 4363 边 / 6 层，canvas 1200x6184）：

```
pwsh -File eval/frame-timing.ps1 -Mode ab -AbCycle "base,B,BA,base,B,BA,base,B,BA" -AbSegmentFrames 60 -OutJson out/frame-timing-ab-cull.json
pwsh -File eval/map-pixel-compare.ps1 -Repo tests/corpus/excalidraw -OutDir out/map-pixel-cull -Positions "p1=0,0,1;p2=-240,-160,0.4;p3=-100,-400,0.25"
pwsh -File eval/map-pixel-compare.ps1 -Repo tests/corpus/excalidraw -OutDir out/map-pixel-pan -Positions "pan=-237,-157,0.4"
pwsh -File eval/map-pixel-compare.ps1 -Repo tests/corpus/excalidraw -OutDir out/map-pixel-reveal -Positions "p2=-240,-160,0.4" -Reveal -AutoCloseMs 20000
```

同进程交替（run `out/frame-timing-ab-cull.json`；按帧下标切段，段长 60，跨批取「中位数的中位数」）：

| 段 | 批次数 | median（ms） | p95（ms） | 相对 `base` |
| --- | --- | --- | --- | --- |
| `base` | 4 | 65.44 | 76.58 | — |
| 仅 `B` | 3 | 54.30 | 61.02 | −17.0% |
| `B+A` | 3 | 28.59 | 34.09 | −56.3% |

分批中位数（漂移可见，结论只取同进程对照）：`base` 65.41 / 70.14 / 65.34 / 65.48；`B` 54.35 / 46.51 / 54.30；`BA` 28.59 / 23.69 / 29.24。

判据核对：

- **仅 B**：median 54.30ms、p95 61.02ms，**未达标**（目标 33/50）。
- **B+A**：median 28.59ms、p95 34.09ms，**达标**（median ≤33、p95 ≤50）。
- 结论：**需要 A+B 同时开启**才能进 33ms；A 的视觉差异仍需单独批准（见下）。

像素等价（先定容差后测）：

| 位置 | 对照 | changed% | maxDelta | 超限像素 | 判定 |
| --- | --- | --- | --- | --- | --- |
| p1 `0,0,1` | base vs B | 0% | 0 | 0 | PASS |
| p1 `0,0,1` | A vs BA | 0% | 0 | 0 | PASS |
| p2 `-240,-160,0.4` | base vs B | 0% | 0 | 0 | PASS |
| p2 `-240,-160,0.4` | A vs BA | 0% | 0 | 0 | PASS |
| p3 `-100,-400,0.25` | base vs B | 0% | 0 | 0 | PASS |
| p3 `-100,-400,0.25` | A vs BA | 0% | 0 | 0 | PASS |
| pan `-237,-157,0.4` | base vs B | 0% | 0 | 0 | PASS |
| pan `-237,-157,0.4` | A vs BA | 0% | 0 | 0 | PASS |
| p2 + `--reveal`（`reveal:done`） | base vs B | 0% | 0 | 0 | PASS |

- 三个静止位置与平移位置的 `base`/`B` PNG **SHA256 逐字节相同**（例：p1 `3ECC4F23…1F6B`、p2 `8B86F469…285E`、p3 `1A259218…ADFC`）；`A`/`BA` 同样逐字节相同（p3 `5E3025F0…AEA7`）。
- 同一批数据里 `base vs A` 仍为 p1 0.174% / maxDelta 90、p2 0.0137% / 70、p3 0.02% / 40，沿用 A 的历史 FAIL，不因 B 放宽。
- 目视：检查 p1（默认）与 p2（低缩放）裁剪帧，边界节点/边/标签连续，无被误剪造成的缺口；因 `base`/`B` 逐字节一致，可判定无闪烁/拖影。
- 揭示：最终态像素 0 差异；中途逐帧像素对照受两进程动画时序限制未做，改由 `viewport_cull.rs` 的 `reveal_offsets_never_cull_a_visible_node` 逐帧扫描全部层带 offset 断言「部分可见即不剪」。播放态同理：裁剪不读 opacity，几何与相机不变，逐帧像素对照**未验证**，只保留不变式断言。
- 脚本退出码为 1 由 `base vs A` 的已知超容差触发，不是 B 失败；B/B+A 的每一行都是 0/0 PASS。

### A 默认值变更（2026-11-09）

三个条件已落地：

1. **常态像素闸门**：`eval/map-pixel-compare.ps1` 默认阈值改为 changed% ≤0.30%、maxDelta ≤100；标准场景（`tests/corpus/excalidraw`，`p1=0,0,1`）跑一次即判红/绿。本轮 `base vs A` = 0.174% / 90，PASS；`base vs B`、`A vs BA` 仍 0/0。
2. **几何等价独立断言**：`crates/arch-view/tests/geometry_equivalence.rs` 在三个相机位逐项比较节点中心/尺寸/屏幕矩形、标签文本与锚点、边端点/方向样式/透明度；`group_edges` 在批量与逐边两种模式下展开后必须与输入序列逐项一致。CI 常态执行 `cargo test --locked -p arch-view -p arch-core`（`.github/workflows/ci.yml`），不依赖截图，和像素断言分开失败；`arch-cli` 测试依赖 pinned 语料，不在该 job 内。
3. **回落开关**：`STRATA_EDGE_BATCH=0` 恢复逐边绘制，用于需要精确栅格（逐条虚线相位、逐边绘制顺序）的场合；A 默认值改为 1。

读数分组（旧读数原样保留，不覆盖）：

**A 默认关时（2026-11-09 上一轮，`out/frame-timing-ab-cull.json`，同进程交替）**

| 段 | median | p95 |
| --- | --- | --- |
| base | 65.44ms | 76.58ms |
| B | 54.30ms | 61.02ms |
| B+A | 28.59ms | 34.09ms |

**A 默认开后（本轮，`out/frame-timing-ab-a-default-on.json`，同进程交替）**

| 段 | median | p95 |
| --- | --- | --- |
| base | 73.26ms | 86.16ms |
| B | 56.07ms | 63.87ms |
| B+A | 31.02ms | 34.58ms |

两轮外部负载不同（本轮整体更慢），只能在同一轮内比较；B+A 仍满足 median ≤33ms、p95 ≤50ms。

**揭示动画实际时长**（excalidraw，80 帧，`reveal:start → reveal:done`，每档 3 次；帧标记 81 条）：

| 配置 | run1 | run2 | run3 | median |
| --- | --- | --- | --- | --- |
| base | 6016.6ms | 7410.8ms | 6907.5ms | 6907.5ms |
| B | 6718.7ms | 5578.4ms | 5814.5ms | 5814.5ms |
| B+A | 2582.7ms | 2607.3ms | 2462.9ms | 2582.7ms |

B+A 揭示窗口内帧间隔 median 31.82–32.53ms、p95 37.77–40.18ms。

**平移帧率**：见「A 默认开后」表（同一进程内 base/B/BA 交替，60 帧/段）。B+A median 31.02ms（≈32fps）、p95 34.58ms，与揭示路径的 31.8–32.5ms 一致，说明提速不是只在揭示路径上。

**是否默认开启揭示**：建议**默认开启**。B+A 路径实际时长 2.46–2.61s，落在 1.5–3s；最终态像素与不揭示逐像素一致（p2 + `--reveal` 0/0）。对既有截图基线的影响：画面内容不变，但抓帧时机必须从 `content:presented` 改到 `reveal:done`，否则会抓到动画中途帧导致标准闸门整体变红；`eval/map-pixel-compare.ps1` 当前默认走非揭示路径，若开启默认揭示需在同一次改动里切换标记。本轮只给建议，不改揭示默认值。

剩余在哪儿、以及 B 的评估：

- A+D 把中位数从 ~69ms 降到 ~47ms，剩余 ~47ms 仍在**每帧全量重建**：802 个节点元素 + 802 个标签 + 4363 条边（全部在画布内生成，即使视口只看到一部分），加上节点/边/标签叠加后的合成成本（诊断轮实测存在超加性交互）。D 已回退，A 单开为 ~48–54ms。
- B（视口裁剪）本轮已实现并实测：**仅 B** 把中位数从 65.44ms 压到 54.30ms（−17.0%），仍未进 33ms；**B+A** 为 28.59/34.09ms，达到 median ≤33、p95 ≤50。因此 33ms 档的结论是「需要 A+B 同时开启」。
- A 的视觉差异已按新常态闸门批准（标准场景 0.174% / maxDelta 90，≤0.30% / 100），A 默认开启；几何差异为 0，回落用 `STRATA_EDGE_BATCH=0`。B 在三个位置与平移位逐字节一致，同样默认开启。


