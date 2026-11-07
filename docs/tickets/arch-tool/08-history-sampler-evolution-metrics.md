# 08: git 历史采样与模块演化指标

**What to build:** `arch-cli history` 只读 git 对象库（不 checkout），沿 first-parent 链计算每个模块的 `firstSeen`、`lastModified`、`commitCount`、`churn`、`activeSpan`，输出带窗口参数、采样/截断状态的 `history.json`；指标能用系统 `git` 命令交叉验证。

**Blocked by:** 01（CLI 契约）、02（模块解析，用于模块标识与路径过滤）

**Status:** ready-for-agent

**固定样本：**
- `tests/fixtures/history`：本地构造 git 仓库，含 5 个 first-parent 提交、1 个合并提交、1 次重命名、1 个空提交，期望值可手工算出。
- `excalidraw/excalidraw` @ `701b901a0f1ba3d0b390d9cdf63d112e25abac4b`，默认窗口 500 first-parent 提交。

- [ ] `cargo run -p arch-cli -- history --repo tests/fixtures/history --out out/history.json` 退出码 `0`；产物含 `schemaVersion`、`window`（`maxCommits`/`since`/`until`）、`sampled`、`truncated`、`shallow`、`modules`。
- [ ] oracle 对照（fixture）：某模块 `lastModified` 等于 `git log -1 --first-parent --format=%ct -- <path>`；`commitCount` 等于 `git rev-list --count --first-parent HEAD -- <path>`；`churn` 等于该窗口 `git log --first-parent --numstat -- <path>` 的增删行和。
- [ ] 重命名：fixture 中的重命名提交后，旧路径与新路径归并到同一模块谱系，`commitCount` 覆盖重命名前后提交；golden 断言谱系 ID 稳定。
- [ ] 合并提交：第二父分支独有的改动不计入模块指标；fixture 中该模块计数与 first-parent-only 期望一致。
- [ ] 不修改目标：运行前后 `git status --porcelain` 为空、`git worktree list` 条数不变、`git rev-parse HEAD` 不变。
- [ ] 非 git 仓库、无提交仓库、浅克隆（`--depth=1`）分别退出码 `5`，stderr 说明原因；浅克隆不得输出冒充全量的指标。
- [ ] 窗口截断时 `truncated=true` 且写明实际扫描提交数；采样模式 `sampled=true` 并写明步长；两者在 JSON 中可见。
- [ ] 确定性：同参数两次运行 `history.json` 字节一致（不含墙钟字段）。
- [ ] 性能：excalidraw 默认窗口 3 次运行中位数 ≤10s（推荐值，**待用户确认**；硬件基线待定），读数归档。
- [ ] 失败判据：与 git oracle 读数不符、目标仓库被修改、浅克隆/非 git 未返回 `5`、截断/采样未标记、两次运行不同。
- [ ] 自动化状态：全部自动化（依赖 CI 镜像预装 `git`；性能门禁依赖硬件基线）。
