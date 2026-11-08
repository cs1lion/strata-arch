# 09: 演化时间线与模块热力视图

**What to build:** 用户能在桌面端看到一条提交时间线，拖动到某个历史提交时，分层地图上的模块按「热力」呈现该时刻的变更活跃度，并看到该模块的指标面板；演化视图与结构地图使用同一套模块 ID。

**Blocked by:** 03（分层地图）、08（历史采样与指标）

**Status:** done（B 档状态/热力断言全绿；截图与 excalidraw 时间线未验证；本票未取得真实 Red，已登记偏差）

**固定样本：** `tests/fixtures/history`（原描述「5 个提交」与实际不符：fixture 为 6 个 first-parent 提交（c1–c5 + 1 个合并提交），期望值按 6 个推导）与 `excalidraw/excalidraw` @ `701b901a0f1ba3d0b390d9cdf63d112e25abac4b`（L2 缓存是无 `.git` 的归档副本，无法生成时间线，本票未测）。

- [x] 时间线渲染：`--screenshot out/timeline.png` **未实现，移交 ticket 11，当前未验证**（gpui 0.2.2 无公开离屏导出入口）。替代实测：`cargo run -p arch-view -- timeline tests/fixtures/history --auto-close-ms 500` 退出码 `0`，窗口能打开并正常退出；刻度数与扫描提交数一致（6 刻度 = 6 个 first-parent 提交）。
- [x] 拖动到 fixture 第 k 个提交后，模块 `lastModified` 面板读数等于「截至该提交重新计算」的期望值（fixture 3 个选取模块逐点断言）；不得只显示最新值。
- [x] 热力映射固定且可解释：churn/commitCount 到颜色/大小的映射函数在测试中冻结；无历史模块显示为中性灰并带标记；图例可见（footer 固定文案「灰=无历史 · 蓝→橙=churn 0→11+」）。
- [x] 共享 ID：演化视图与地图视图中同一模块的 ID 完全一致（集合断言通过）。
- [x] 状态标记：`sampled`/`truncated`/`shallow` 在视图中有可见标记；标记存在时热力不得表现成全量读数。
- [x] `cargo test -p arch-view --test timeline_scrub` 通过：刻度切换、模块选择、返回地图后的状态断言。
- [x] 截图结构断言：**已移交 ticket 11 重新定义，当前未验证**；本票以布局模型与热力映射断言为主，像素差门禁随截图能力一并待定。
- [x] 失败判据：刻度读数取最新值、热力映射漂移、ID 不一致、状态标记缺失、布局模型中的刻度或节点数与产物不符。
- [x] 自动化状态：状态与结构断言自动化；像素差与视觉效果依赖硬件基线，暂不进门禁。
- [x] 动画与探索式交互不在本票范围：播放/回放动画与跨视图联动细化见 ticket 13（占位）。

## 验收记录（2026-11-08，B 档）

### 断言与 Red 状态（如实登记）

- 本票**未取得真实编译 Red**：实现（`arch-view/src/timeline.rs`）先于测试落盘，测试写好后直接编译通过并转绿。清单为测试落盘时补写，`timeline_scrub.rs` 文件头已标注该偏差，不冒充红绿证据。
- Green：`cargo test -p arch-view --test timeline_scrub` → 6 passed / 0 failed（11.76s）。

### 实现要点

- `TimelineState`：每刻度存「截至该提交」重新采样的 `HistoryArtifact`；`scrub_to(index)` 只切换读数，越界返回 `false` 且索引不变；默认停在最新提交。
- 热力映射固定：`HEAT_STOPS` 阈值 `0 / 1 / 4 / 11`（蓝→青→橙），尺寸 `100 + commitCount.min(10) * 3 + churn.min(20)`（整数百分比）；无历史模块 `HEAT_NEUTRAL` 中性灰并置 `neutral`。
- 状态标记：`sampled` / `truncated` / `shallow` 任一存在时 `heat_is_full_scale() == false`，UI 按 0.55 透明度削弱，并在 footer 显示标记。
- 共享 ID：历史产物模块 ID 直接用作地图节点匹配键；`ids_match_map_artifact_and_unknown_modules_are_neutral` 对 fixture 做集合断言。
- UI：`arch-view timeline <repo>` 渲染层带 + 热力节点；点击节点选择并显示 `lastModified / count / churn / span`；底部刻度可点击 scrub；无历史/标记在 footer 可见。

### 读数（实测）

- `cargo test -p arch-view --test timeline_scrub`：6 passed / 0 failed。
- 刻度与提交：`tests/fixtures/history` HEAD `1de5415`，first-parent 6 个；UI 输出 `timeline: 6 ticks, 3 modules, 1 bands`。
- 逐点期望（测试断言）：`src/app.ts` c1 `lastModified=1767268800 / count=1 / churn=3`；c2 `1767355200 / 2 / 5`；`src/helpers.ts` rename `1767441600`；c5 `1767614400 / 3 / 6`；`src/helpers.ts` 在 c1 不存在。
- 窗口 smoke：`target/debug/arch-view.exe timeline tests/fixtures/history --auto-close-ms 500` → exit 0，墙钟 4435ms（含 500ms 自动关闭；6 个刻度各跑一次前缀 `build_history`）。
- 全量：`cargo test --workspace` 115 passed / 0 failed；`cargo fmt --all --check` 通过。

### 已知偏差与未验证

- **截图未实现**：ticket 第一项要求的 `--screenshot out/timeline.png` 未做（gpui 0.2.2 无公开离屏导出入口），移交 ticket 11；当前只有「窗口能开 + 退出码 0」的 smoke，没有任何像素证据。
- **excalidraw 时间线未测**：`eval/corpus.toml` 的 L2 缓存是无 `.git` 的归档副本，`eval/fetch-corpus.ps1` 不保留历史，无法生成时间线；需真实克隆才能测，本票未做。
- **成本口径**：每个刻度都对该提交的前缀重跑一次 `build_history`（O(n²) 次采样）；fixture 6 刻度 4435ms，大仓不可用。增量/缓存采样未实现，作为 ticket 10 的性能输入而非本票通过项。（2026-11-08 复核：该「O(n²)」诊断只部分成立，保留原说法，见下文「诊断复核」节。）
- **交互自动化缺失**：刻度点击与节点选择只做本机 smoke，没有 `gpui::test` 事件断言；UI 视觉与截图未验证。
- **固定样本描述修正**：ticket 原文「5 个提交」与 fixture 实际 6 个 first-parent 提交不符，保留原描述并在上方标注实际读数。

## 采样性能修复（2026-11-08，C 档；判据先钉死）

**问题事实：** 每个刻度都调用一次 `build_history(until=@time+1)` 重建前缀（`arch-view/src/main.rs` 的 `build_timeline_entries`），刻度数与 git 遍历次数 1:1，历史窗口内是 O(刻度×git 遍历)。既有读数：`target/debug/arch-view.exe timeline tests/fixtures/history --auto-close-ms 500` 端到端墙钟 4435ms（含窗口启动与 500ms 自动关闭，**不能**直接当作采样耗时）。

**夹具：** `tests/fixtures/history`（6 个 first-parent 提交，HEAD `1de5415`）+ 测试内合成仓库（200 个固定日期提交，用于预算与线性度）。

**命令：**
`cargo test -p arch-core --test history_prefixes -- --nocapture --test-threads=1 --ignored`

计时测试标 `#[ignore]`：并行 `cargo test --workspace` 会把其他测试的 git 进程算进读数
（实测中位数被抬到 328.9ms）。阈值、样本、3 次口径均不变，只隔离计时环境。

**硬件：** AMD Ryzen 7 8845H（8 核，3801 MHz），32 GB；Windows；debug 构建。

### 目标（测量前钉死，读数不达标即报红，不回调）

| 编号 | 指标 | 阈值 | 测法 |
| --- | --- | --- | --- |
| B1 | 6 刻度（`tests/fixtures/history`）批式采样墙钟中位数 | ≤ 250 ms | 单次调用 `build_history_prefixes`，3 次重复 |
| B2 | 200 刻度（合成 200 提交）批式采样墙钟中位数 | ≤ 1500 ms | 同上，3 次重复 |
| B3 | 20 刻度 / 6 刻度 墙钟比（同一 200 提交合成仓） | ≤ 4.0 | 若按刻度平方增长，比值约 11.1；线性约 3.3 |
| B4 | 逐字节等价 | 每个刻度的 artifact JSON 与旧实现 `build_history(until=@time+1)` **完全一致**，SHA256 相等 | 固定样本 + 合成仓逐刻度比较 |

C 档记录：每个指标 3 次运行，报每次读数、中位数与 p95（3 样本的 p95 取最大值）。

### 失败判据

- 任一刻度 artifact 与旧实现字节不等（含 `window.until`、`stats`、`modules` 顺序）。
- B1/B2/B3 任一超阈值。
- 合成仓旧路径（未优化实现）没有取得断言级 Red。

### 反向用例（全部断言）

1. 同一模块在多个 commit 被反复修改：累计 `commitCount`/`churn` 必须随刻度单调累加，不被增量覆盖。
2. 某个 commit 只改不相关（非源码）文件：前后相邻刻度的 `modules` 数组必须逐字节相同。
3. `--max-commits` 截断：批式产物与旧实现同参数（`max_commits=2`）逐刻度字节一致，`truncated`/`commits_scanned`/`commits_available` 相同。

### 语义声明（不改采样语义）

批式实现仍沿 first-parent 链、仍按 git 的 committer date 过滤（`until=@time+1` 语义保留），仅把「每刻度一次 git 遍历」改为「一次遍历 + 内存前缀过滤」。若发现 first-parent 行走顺序会让结果偏离旧实现，必须停下说明，不得静默改语义。

### 自动化状态

已自动化：B4 等价断言、三项反向用例、B1/B2/B3 预算断言（`arch-core/tests/history_prefixes.rs`；预算断言标 `#[ignore]`，按上方固定命令单独跑）。

### 实现

- 新增 `arch_core::history::build_history_prefixes(root, version, options, tick_times)`：一次 `git log` 取全量 first-parent 提交，每个刻度只在内存按 `commit.time <= @T+1` 过滤后聚合（`crates/arch-core/src/history.rs`）。
- 仓库元信息（toplevel / shallow / HEAD）与提交列表两条独立 git 调用并发执行，摊掉进程启动成本。
- `arch-view` 的 `build_timeline_entries` 改为一次批式调用（`crates/arch-view/src/main.rs`）。
- 采样语义不变：仍 first-parent、仍按 committer date、`until=@T+1` 写回 `window.until`。

### 读数（2026-11-08，实测）

**Red（入口仍委托逐刻度 `build_history` 时，同一命令）：**

```
test batch_sampling_meets_pinned_budget ... B1 fixture 6 ticks ms: [3706.2782, 3569.9545, 3524.2641] median=3570.0 p95=3706.3
panicked: B1 超预算: 6 刻度中位数 3570.0ms > 250ms
test result: FAILED. 3 passed; 1 failed
```

**Green（优化后，同一命令）：**

```
test batch_sampling_meets_pinned_budget ... B1 fixture 6 ticks ms: [206.8808, 145.0678, 158.4952] median=158.5 p95=206.9
B3 200-commit repo: 6 ticks ms=[158.7327, 112.1115, 200.5838] median=158.7; 20 ticks ms=[223.1422, 123.4889, 171.0849] median=171.1; ratio=1.08
B2 200 ticks ms=[179.5054, 127.3947, 179.7927] median=179.5 p95=179.8
test result: ok. 4 passed; 0 failed
```

| 指标 | 旧（逐刻度 git 遍历） | 新（一次遍历 + 内存前缀） | 阈值 | 判定 |
| --- | --- | --- | --- | --- |
| B1 6 刻度中位数 | 3570.0 ms | 158.5 ms（复跑 233.6 ms） | ≤ 250 ms | 绿 |
| B2 200 刻度中位数 | 未测（red 在 B1 已失败） | 179.5 ms（复跑 255.1 ms） | ≤ 1500 ms | 绿 |
| B3 20/6 刻度比 | 未测 | 1.08（复跑 1.19） | ≤ 4.0 | 绿 |

隔离命令复跑（同一天、不同负载）：`B1 [299.6, 233.6, 227.2] median=233.6 p95=299.6; B3 ratio=1.19; B2 [221.8, 262.2, 255.1] median=255.1`。B1 余量只有十几毫秒，读数波动来自本机 git 进程启动（110–150 ms/次），不是采样算法本身。

**B4 SHA256（旧/新逐刻度相等，Green 复跑与 Red 阶段完全一致）：**

```
tick 0 @1767268800 ac0801759e342e5fdd0f02acb05d242b3f71a7945e62b8d95be4602ce58fc2bc
tick 1 @1767355200 bfd5faec9a7f6f85f8636930209442ebb0a5257846120da46585f46d134ea75a
tick 2 @1767441600 eaf0ea43f9e2f678fc480f0a34a8d7e16ae9a637ac882593df42ba8d32a7cee0
tick 3 @1767528000 b9771a0e52a2d9f22f98ea79339bee9b04f8c87a7c51b9cbebd0248bae2fb3d3
tick 4 @1767614400 4694cb0cdaf55404a607f2cd1486f54e60f19b751ee2aa2421bf2896f62807ea
tick 5 @1767787200 e51d652fb8e41d4e2a9d324282e468cbbee5f10b7e4f2ff947e84db082f2b08d
```

反向用例：`repeated_modifications_accumulate_and_unrelated_commit_is_neutral`（累计 4 次 touch / +4 / -3 / total 7；只改 `docs/notes.md` 的刻度 `modules` 逐字节不变）与 `prefixes_honor_max_commits_truncation`（`max_commits=2` 逐刻度字节一致，`truncated=true`、`scanned=2`、`available=6`）均通过。

## 「O(n²)」诊断复核（2026-11-08；真实 tsx 历史）

**触发：** 两组既有读数互相矛盾：fixture 6 刻度端到端 4435ms（含窗口启动与 500ms 自动关闭），与 tsx 617 刻度 372.1s（票 12 测试包、改前二进制）。617 刻度只有 6 刻度的 84 倍，而纯二次应给出数千倍。因此**先做规模曲线，未看清曲线前不改实现**。

**固定样本：** `privatenumber/tsx` @ `4473971f4b3261de253209b148d8fdc43269fcc4`；`git rev-list --count --first-parent` = **617**（与票 12 同口径）。克隆到仓库外 `%TEMP%\tsx-hist`，不将完整克隆入库。

**命令：**

```
git clone --no-checkout https://github.com/privatenumber/tsx.git %TEMP%\tsx-hist
cargo run -q -p arch-core --example history_scale_curve -- %TEMP%\tsx-hist 3
cargo run -q -p arch-core --example history_prefix_equiv -- %TEMP%\tsx-hist 617
```

两个 example 是一次性测量工具，已入库以便复现；读数来自上述命令，硬件同前（Ryzen 7 8845H / 32GB / Windows / debug）。

**时序偏差（必须记）：** 批式实现与视图接线在收到本次复核指令**之前**已提交并推送（`3d171dd`…`bebe3ae`）。所以本节「逐刻度」曲线的每刻度固定成本是 **2 次并发 git 调用**，不是 372.1s 当时的 4 次串行。曲线形状仍可判定；绝对值不能直接与 372.1s 对比。

### 规模曲线（同一份 tsx 历史，3 次实测）

| 刻度数 n | 逐刻度 total（ms，3 次） | 中位数 | 单刻度中位数 | 批式 total（ms，3 次） | 批式中位数 | 批式单刻度 |
| --- | --- | --- | --- | --- | --- | --- |
| 6 | 694.7 / 620.9 / 627.0 | 627.0 | 104.5 ms | 293.7 / 305.3 / 315.5 | 305.3 | 50.9 ms |
| 20 | 2606.0 / 2926.2 / 2481.1 | 2606.0 | 130.3 ms | 433.2 / 430.9 / 400.2 | 430.9 | 21.6 ms |
| 100 | 16470.0 / 20919.8 / 13078.2 | 16470.0 | 164.7 ms | 410.2 / 348.0 / 377.8 | 377.8 | 3.8 ms |
| 617 | 143100.6 / 157425.3 / 165635.4 | 157425.3 | 255.15 ms | 1261.9 / 1315.8 / 1222.0 | 1261.9 | 2.05 ms |

单刻度探针（同一命令）：最早提交 `[227.4, 220.1, 201.6]` 中位数 **220.1ms**；最新提交 `[424.8, 393.1, 387.0]` 中位数 **393.1ms**。

### 判定：混合模型，不是纯 O(n²)，也不是线性

- 6→617（102.8× 刻度）：总耗时 627.0 → 157425.3ms，即 **251×**；线性应约 64.5s，实测为线性的 **2.44×**。
- 纯二次预测：627×(617/6)² ≈ **6630s**；实测 157.4s，仅为二次预测的 **2.4%**。二次项真实存在，但不是主导。
- 单刻度成本随 n 从 104.5 → 255.2ms（**2.44×**），说明每刻度确有随历史长度增长的遍历项。
- 单刻度最早 vs 最新相差 173.0ms，对应 617 个提交的遍历增量（≈0.28ms/提交）；固定成本约 220ms/刻度，617 刻度合计约 135.7s，占实测 157.4s 的 **86%**。
- 结论：`total ≈ a·n + b·n²`，a（每刻度固定进程成本）主导，b（git log 前缀遍历）真实但次要。

**保留原说法与推翻理由：**

- 原记录（本文件第 49 行）「O(n²) 次采样」：方向成立（遍历项确实随刻度数增长），但把它当作 617 刻度的主导根因**不成立**；4435ms 端到端读数中窗口启动与 500ms 自动关闭占大头，不能当作采样成本。
- 反方读数（84×）也不完整：它把 fixture 6 刻度与 tsx 617 刻度**跨仓**比较，且忽略同一 tsx 上单刻度成本实测上涨 2.44×。两组读数都没有把「每刻度固定进程成本」与「前缀遍历成本」分开。
- 误导来源：**把不同仓库、含 UI 启动的端到端读数与纯采样读数混为一谈**。

### 对票 12 读数的影响（必须重测）

372.1s 是票 12 测试包在**改前二进制**上的读数。批式实现落地后，该测试包所有 `arch-view` 启动读数——**地图 4.0s / diff 5.1s / 时间线 372.1s**——全部失效，必须在新二进制上重测；重测读数不得与本 ticket 的改前读数放在同一张表。

### 逐字节等价（真实 tsx 抽查）

`history_prefix_equiv` 对 n=6 / 20 / 617 的每个刻度比较旧 `build_history(until=@T+1)` 与批式的序列化 JSON：**全部 `equal=true`**。n=617 最后一刻度两侧 SHA256 均为 `f492c2a60b588b31c5f771a580347878baf036651f24a5a3cf7a190e17107a79`。fixture 的完整 B4 断言见上节。

### 阈值（617 真实历史）

- 617 真实历史**没有预钉阈值**；本节曲线是事后测量。按纪律不能把事后读数声称为「已通过预钉判据」。
- **事后提议（待用户确认）**：617 刻度批式采样中位数 **≤ 5 s**（3 次，`history_scale_curve`）。本次实测中位数 **1.262 s**；旧逐刻度基线 **157.425 s**。确认前该条不计为通过项。
- 失败判据（提议）：后续任一次 617 刻度批式中位数 > 5s，或任一 617 刻度 artifact 与旧实现字节不等。

### 自动化状态

- 逐字节等价与反向用例：已自动化（`crates/arch-core/tests/history_prefixes.rs`，fixture + 合成仓）。
- 617 真实历史规模曲线与等价抽查：**手工一次性测量**（`arch-core/examples/history_scale_curve.rs`、`history_prefix_equiv.rs`），未纳入 CI；依赖真实 tsx 克隆，不能在 Gate 默认跑。
- 未验证：617 刻度批式下 `arch-view timeline` 的端到端墙钟（本节只测采样层，UI 启动成本未测）。

端到端同口径（`target/debug/arch-view.exe timeline tests/fixtures/history --auto-close-ms 500`，3 次）：`3993.9 / 3845.5 / 3031.5 ms`，中位数 3845.5 ms；旧读数 4435 ms。

回归：`cargo test -p arch-core --test history_golden` 6 passed；`cargo fmt --all --check` 通过。

### 已知偏差

- **端到端只降了约 13%，不是量级提升**：4435 ms 里大头是窗口/层产物启动，旧 ticket 把它整体算到「采样」名下是误归因。采样阶段本身的正确读数是 B1（3570.0 → 158.5 ms，约 22 倍）。
- **剩余复杂度**：内存内仍是 O(刻度数 × 窗口提交数) 的重复聚合（500×500 量级可忽略）；真正消除的是「每刻度一次 git 遍历」。
- **B1 阈值与机器强相关**：本机单次 git 进程启动约 110–150 ms，B1 达标依赖把 4 次 git 调用压到 2 次并发；换更慢的机器可能复现红，读数口径与方法已固定。

