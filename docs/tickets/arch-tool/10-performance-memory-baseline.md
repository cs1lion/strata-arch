# 10: 性能与内存基线归档

**What to build:** 维护者能复现一组固定的性能读数：两个结构样本仓的冷索引墙钟、历史默认窗口墙钟、峰值内存，以及超限仓库的处理行为；读数与硬件信息一起归档，用于判断预算是否仍成立。

**Blocked by:** 02（模块解析）、03（层推断）、04（符号与调用图）、08（历史采样）

**Status:** ready-for-agent

**固定样本：** `privatenumber/tsx` @ `4473971f4b3261de253209b148d8fdc43269fcc4`、`excalidraw/excalidraw` @ `701b901a0f1ba3d0b390d9cdf63d112e25abac4b`；合成 >5k 文件 fixture。

- [ ] `cargo run --release -p arch-cli -- index <tsx样本>` 3 次运行，中位数 ≤2s；excalidraw 中位数 ≤10s（两者均以推荐硬件基线为准，**待用户确认**）；每次读数写入 `out/perf.json`，含硬件、工具版本、样本 commit。
- [ ] `cargo run --release -p arch-cli -- history --repo <excalidraw样本> --max-commits 500` 3 次运行，中位数 ≤10s（**待硬件基线与用户确认**）。
- [ ] 峰值内存：Linux 用 `/usr/bin/time -v` 采集 RSS；Windows 用 PowerShell 轮询 `WorkingSet64`（代理指标，置信度标注为低）；推荐硬上限 512MB（**待用户确认**），超限时进程以非 `0` 退出并报告已处理规模，不得因 OOM 崩溃。
- [ ] 超限仓库（合成 >5k 文件或 >20k 符号）返回明确拒绝或降级：退出码非 `0` 或产物带 `degraded=true`，且说明实际规模；不得静默输出「达标」。
- [ ] 归档文件可比较：同一机器连续运行 3 次的中位数波动 ≤20%（波动更大时标注环境噪声并保留原始读数）。
- [ ] 失败判据：超过墙钟或内存阈值且未标注待确认/环境限制、OOM 崩溃、超限仓库被静默处理、归档缺硬件信息。
- [ ] 自动化状态：墙钟自动化（单平台 CI）；跨平台峰值内存只有代理口径，不设硬门禁（原因见 `docs/acceptance-gaps.md`）。
