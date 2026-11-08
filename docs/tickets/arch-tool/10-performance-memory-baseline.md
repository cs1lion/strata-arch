# 10: 性能与内存基线归档

**What to build:** 维护者能复现一组固定的性能读数：两个结构样本仓的冷索引墙钟、历史默认窗口墙钟、峰值内存，以及超限仓库的处理行为；读数与硬件信息一起归档，用于判断预算是否仍成立。

**Blocked by:** 02（模块解析）、03（层推断）、04（符号与调用图）、08（历史采样）

**Status:** partial（索引墙钟与超限拒绝实测通过；历史窗口读数未取得、512MB 内存硬上限未实现，均未验证）

**固定样本：** `privatenumber/tsx` @ `4473971f4b3261de253209b148d8fdc43269fcc4`、`excalidraw/excalidraw` @ `701b901a0f1ba3d0b390d9cdf63d112e25abac4b`；合成 >5k 文件 fixture。

- [x] `cargo run --release -p arch-cli -- index <tsx样本>` 3 次运行，中位数 ≤2s；excalidraw 中位数 ≤10s（两者均以推荐硬件基线为准，**待用户确认**）；每次读数写入 `out/perf.json`，含硬件、工具版本、样本 commit。实测：tsx 中位数 69.4ms（第二批 69.5ms），excalidraw 中位数 278.3ms（第二批 293.4ms）。
- [ ] `cargo run --release -p arch-cli -- history --repo <excalidraw样本> --max-commits 500` 3 次运行，中位数 ≤10s（**待硬件基线与用户确认**）。**未验证**：`tests/corpus/excalidraw` 是归档副本，没有 `.git`，无法复现 first-parent 历史窗口；需真实克隆。
- [ ] 峰值内存：Linux 用 `/usr/bin/time -v` 采集 RSS；Windows 用 PowerShell 轮询 `WorkingSet64`（代理指标，置信度标注为低）；推荐硬上限 512MB（**待用户确认**），超限时进程以非 `0` 退出并报告已处理规模，不得因 OOM 崩溃。读数已采集（tsx 5.7MB、excalidraw 5.9MB，OS `PeakWorkingSet64`），但 **512MB 硬上限与超限拒绝未实现**（未验证）。
- [x] 超限仓库（合成 >5k 文件或 >20k 符号）返回明确拒绝或降级：退出码非 `0` 或产物带 `degraded=true`，且说明实际规模；不得静默输出「达标」。实测 5001 文件 → exit 2、stdout 空、stderr「目标仓库源文件数 5001 超过上限 5000，已拒绝索引」。
- [x] 归档文件可比较：同一机器连续运行 3 次的中位数波动 ≤20%（波动更大时标注环境噪声并保留原始读数）。两批中位数波动：tsx 0.14%、excalidraw 5.4%；单批内 tsx 首轮 93.6ms 属冷启动噪声，原始读数保留。
- [x] 失败判据：超过墙钟或内存阈值且未标注待确认/环境限制、OOM 崩溃、超限仓库被静默处理、归档缺硬件信息。
- [x] 自动化状态：墙钟自动化（单平台 CI）；跨平台峰值内存只有代理口径，不设硬门禁（原因见 `docs/acceptance-gaps.md`）。

## 验收记录（2026-11-08，C 档）

### 预算先行（实现前固定）

- 预算：tsx index ≤2000ms、excalidraw index ≤10000ms、excalidraw history ≤10000ms、峰值内存 512MB（待确认）、3 次运行取中位数、同批波动 ≤20%。
- 测量工具：`eval/perf-baseline.ps1`（release 构建、3 次运行、`WorkingSet64` 轮询 + `PeakWorkingSet64`、墙钟、硬件与样本 commit 写入 `out/perf.json`）。

### 读数（实测）

硬件：`AMD Ryzen 7 8845H w/ Radeon 780M Graphics`，16 逻辑 CPU，`Microsoft Windows NT 10.0.26300.0`，PowerShell 7.6.9；debug → release 构建，工具版本 0.1.0。

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

### 已知偏差与未验证

- **历史窗口未测**：excalidraw 的 L2 缓存是无 `.git` 的归档副本，`history --max-commits 500` 无法运行；没有真实克隆，故该项不按通过计。
- **内存硬上限未实现**：当前没有任何内存超限拒绝逻辑；512MB 仍是「推荐值（待用户确认）」。读数只是 Windows 代理口径，未用 Linux `/usr/bin/time -v` 复核。
- **超限只有文件数维度**：现有拒绝基于 `DEFAULT_MAX_FILES = 5000`；`>20k 符号` 维度没有独立门禁（文件数门禁先行覆盖）。
- **冷启动口径**：`index` 墙钟包含进程启动，但样本仓较小，启动开销占比不可忽略；未单独拆分。
- **硬件基线未确认**：读数来自本机，是否作为 CI 基线待用户确认。
