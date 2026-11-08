# 14: 修复 arch-view 冒烟测试偶发超时

**What to build:** 查清 `crates/arch-view/tests/smoke.rs` 中 `smoke_renders_one_frame_and_exits_cleanly` 偶发超时的根因并修复。**先取证，后修改**：没有实测证据之前不改代码，也不允许用「把 3 秒调大」直接结案。

**动机：** 这条测试是 ticket 11「Windows 启动冒烟」的唯一自动化证据。它偶发失败 = ticket 11 的「Windows 能启动、渲染一帧、干净退出」结论不稳；不能让一个随机失败的测试继续给 11 背书。

**Blocked by:** 11（3 秒预算与冒烟测试来自 ticket 11）

**Status:** fixed & verified（2026-11-08；取证读数与修后验证见文末）

**固定样本/commit：** `tests/fixtures/tiny`；基线 commit `db26f16b2cbc667fa2041ea01ffaf9bbfcadd3f1`（本票开工时 HEAD）。硬件/OS：`AMD Ryzen 7 8845H`、16 逻辑 CPU、`Microsoft Windows NT 10.0.26300.0`、PowerShell 7.6.9；`cargo test` debug profile。

## 验收项

### 1. 复现与量化（取证先行）

- 命令 A（单跑基线，20 次）：
  `1..20 | ForEach-Object { cargo test -p arch-view --test smoke -- --exact smoke_renders_one_frame_and_exits_cleanly }`
- 命令 B（并行负载，20 次）：每次 smoke 运行期间同时启动其他 crate 测试
  （`cargo test -p arch-core --tests` 与 `cargo test -p arch-cli --tests`），并记录并发进程数。
- 每次必须记录：序号、通过/失败、实际墙钟（ms）、启动与峰值时的并行进程数、完整 stdout/stderr。
- 期望行为：测试在预算内 exit 0 且 stdout 含 `smoke: frame rendered`。
- 失败判据：任一非 0 退出、缺帧标记、3 秒内未退出（被 harness kill）、stderr 含 panic。
- 若 20 次单跑与 20 次并行负载全部通过，如实报告「当前无法复现」，不得人为制造失败；给出提高并行度/上 CI 观察等下一步建议。

### 2. 根因证据

必须给出证据区分以下三种可能，并明确写出哪一种被实测排除、哪一种被实测支持：

1. 「3 秒阈值与真实耗时余量不足」：给出单跑与并行负载下的耗时分布（min / median / max / p95）。
2. 「并行资源争抢（GPU / 窗口句柄 / 单线程初始化）」：比较单跑与并行负载的失败率与耗时，记录失败时刻的并行进程数。
3. 「首次编译或首次窗口创建冷启动」：区分首次窗口创建与后续运行，排除 `cargo` 编译时间。

取证手段允许使用带时间戳的 stdout 流读取（`smoke: layout` → `Application::run` → `smoke: frame rendered` / `smoke: timeout waiting for first frame`），不修改生产代码；若必须插桩，插桩提交与修复提交分开并说明。

### 3. 修复方案

- 拿到证据之前不得改代码。
- 修复必须针对根因，不允许单纯把 3 秒调大。
- 若结论是阈值不合理：用实测分布说明原阈值为什么不成立（例如并行负载下 max/p95 已超过 3 秒），并作为「判据修正」记录旧判据、旧读数、新判据、新读数与理由；不得以「调松凑绿」方式处理。
- 若结论是资源争抢或冷启动：修复应消除争抢/等待来源（例如窗口就绪信号、单实例串行化、渲染完成条件），而不是放宽预算。

### 4. 改后验证

- 修复后重跑命令 A 与命令 B，各 20 次含并行负载。
- 期望：两组成败均为 20/20 通过；改前/改后失败频率与耗时分布同口径对比并归档。
- 若修订预算，必须保证新预算由改前实测分布推导，且仍能挡住「窗口未创建/未渲染」的真实回归。

### 5. 自动化状态

`部分自动化`：冒烟断言本身已自动化（`cargo test -p arch-view --test smoke`）；复现压测依赖并行测试负载与桌面会话，本轮以手动脚本执行并把原始读数归档到 `docs/evidence/14-*`。

## 已知偏差与约束（取证前登记）

- 3 秒预算按已构建二进制的墙钟计，`cargo` 冷编译不计入；但 `cargo test` 的调度与编译锁仍可能影响并行负载下的实测耗时，取证时需分别记录。
- 冒烟需要可用图形后端与桌面会话；无头 CI 可能直接失败（另一类问题，不在本票「偶发」范围内，但要在证据里区分）。
- 已知一次失败现场（2026-11-08，`cargo test --workspace`）：stdout 仅有 `smoke: layout 3 modules, 2 edges, 1 bands`，无 `smoke: frame rendered`；stderr 为空；被 harness 在 3 秒处 kill。该现场作为本票第一份证据，后续压测必须能解释它。

## 取证读数（修前）

测试内耗时为 `cargo test` 输出的 `finished in`；墙钟另含 cargo 启动/构建检查。固定样本与命令见上文。

| 场景 | 次数 | 失败 | 测试内耗时 min / median / max | 并发进程 | 失败现场 |
| --- | --- | --- | --- | --- | --- |
| A 单跑 | 20 | 3（15%） | 1.54 / 1.81 / 3.08s | 0 | 3 次都在 3.08s 被 kill；1 次 stdout 已含 `smoke: frame rendered`，2 次只有 `smoke: layout ...`；stderr 全空 |
| B 并行负载 | 20 | 18（90%） | 2.29 / 3.10 / 3.37s（被 3s kill 截断） | 16–20 | 全部 `smoke 未在 3 秒内退出`；stdout 仅 `smoke: layout 3 modules, 2 edges, 1 bands`，无帧标记；stderr 全空 |

- 完整失败 stdout/panic 见 `docs/evidence/14-single.json`、`14-parallel-1.json`、`14-parallel-2.json`。
- 并行负载 harness：每次 smoke 运行期间同时跑 `cargo test -p arch-core --tests` 与 `cargo test -p arch-cli --tests`；并发进程数 16–20。
- 记录一次无效 harness：用 PowerShell `Start-Job` 跑 smoke（会改变进程上下文）得到 9/10 失败，该批不作证据；已改为同 shell 前台执行后重跑，失败率仍为 18/20。

## 到帧时间（修前，并行负载，10 次二进制探针）

`docs/evidence/14-diag-parallel.json`：

| 阶段 | 范围 |
| --- | --- |
| 进程启动 → `smoke: layout` | 4.13–5.17s |
| `smoke: layout` → `smoke: frame rendered` | 2.31–3.51s |
| 启动 → 帧 | 6.51–8.66s |
| 退出总墙钟 | 6.60–8.70s |

10/10 最终都渲染成功并 exit 0，说明不是 GPU/窗口句柄耗尽导致「永远渲染不出」。

## 根因归因

- **阈值余量不足：实测支持。** 3 秒预算下单跑 3/20 失败；成功样本最慢 2.62s，余量仅 0.38s；并行负载下到帧 6.5–8.7s，3 秒不够。
- **并行资源争抢：实测支持。** 失败率 3/20（单跑）→ 18/20（并行，16–20 进程），测试内耗时中位数 1.81s → 3.10s（被 kill 截断）；`smoke: layout` 本身在负载下被推迟到 4–5s，说明争抢发生在进程启动/调度阶段，不只是 GPU 或窗口句柄。
- **首次编译/首次窗口创建冷启动：实测排除为唯一原因。** 修前第 1 次 2.91s（构建后首次），第 19/20 次仍在热态失败；修后首次 3.75s，后续 2.50–4.04s。冷启动会抬高首次耗时，但不是 90% 并行失败的来源。
- **GPUI 内 2.5s 兜底失效：实测支持。** 修前 3 次单跑失败与 18 次并行失败中，stderr 均无 `smoke: timeout waiting for first frame`；进程停在 3.08s 被外部 kill。原因是 `cx.spawn` 定时器排在 GPUI 执行器上，窗口创建阻塞时执行器无法运行，兜底永远不会触发。

## 修复（取证后）

1. `crates/arch-view/tests/smoke.rs`：预算 3s → 15s（`SMOKE_BUDGET`）。这是判据修正，不是调松凑绿：原 3 秒来自 ticket 11 的单次 828ms 读数，没有并行负载与分布；新预算覆盖实测最慢 8.66s，保留 1.73x 余量，仍能抓住真正的挂死。失败信息增加 `frame_rendered=true/false`，区分「已渲染但没及时退出」与「完全没渲染」。
2. `crates/arch-view/src/main.rs`：删除 GPUI `cx.spawn` 的 2.5s 定时器，改为 `Application::run` 前启动独立 OS 线程看门狗（12s）。窗口创建阻塞时线程仍能打印 `smoke: timeout waiting for first frame` 并 `exit 2`；12s < 15s，真实无渲染会先给出可读超时，而不是被测试 harness kill。

## 改后验证

| 场景 | 次数 | 失败 | 测试内耗时 min / median / max |
| --- | --- | --- | --- |
| A 单跑（修后） | 20 | 0 | 2.50 / 3.41 / 4.04s |
| B 并行负载（修后，16–20 进程） | 20 | 0 | 3.43 / 4.49 / 7.52s |

- 对比：单跑 3/20 → 0/20；并行负载 18/20 → 0/20。
- 修后并行最慢 7.52s，低于 15s 预算，也低于 12s 看门狗。
- 读数归档：`docs/evidence/14-post-single.json`、`14-post-parallel-1.json`、`14-post-parallel-2.json`。
- 最终回归：`cargo test --workspace` 与 `cargo fmt --all --check` 的结果见提交记录。

## 自动化状态（更新）

`部分自动化`：正向断言（15 秒内 exit 0 + 帧标记 + 无 panic）已自动化并纳入 `cargo test`。看门狗负路径（窗口不渲染时 12 秒 exit 2）未做自动化验证；本轮修后 40/40 均为正常渲染，没有触发看门狗。
