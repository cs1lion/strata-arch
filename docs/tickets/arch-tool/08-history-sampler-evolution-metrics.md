# 08: git 历史采样与模块演化指标

**What to build:** `arch-cli history` 只读 git 对象库（不 checkout），沿 first-parent 链计算每个模块的 `firstSeen`、`lastModified`、`commitCount`、`churn`、`activeSpan`，输出带窗口参数、采样/截断状态的 `history.json`；指标能用系统 `git` 命令交叉验证。

**Blocked by:** 01（CLI 契约）、02（模块解析，用于模块标识与路径过滤）

**Status:** done（08；性能项未验证，见文末）

**固定样本：**
- `tests/fixtures/history`：由 `tests/fixtures/history-seed/build.ps1` 生成的本地 git 仓库（该目录被 `.gitignore` 排除，不入库），含 5 个 first-parent 提交、1 个合并提交、1 次重命名、1 个空提交，期望值可手工算出。
- `excalidraw/excalidraw` @ `701b901a0f1ba3d0b390d9cdf63d112e25abac4b`，默认窗口 500 first-parent 提交。

- [x] `cargo run -p arch-cli -- history --repo tests/fixtures/history --out out/history.json` 退出码 `0`；产物含 `schemaVersion`、`window`（`maxCommits`/`since`/`until`）、`sampled`、`truncated`、`shallow`、`modules`。
- [x] oracle 对照（fixture）：某模块 `lastModified` 等于 `git log -1 --first-parent --format=%ct -- <path>`；`commitCount` 等于 `git rev-list --count --first-parent HEAD -- <path>`；`churn` 等于该窗口 `git log --first-parent --numstat -- <path>` 的增删行和。
- [x] 重命名：fixture 中的重命名提交后，旧路径与新路径归并到同一模块谱系，`commitCount` 覆盖重命名前后提交；golden 断言谱系 ID 稳定。
- [x] 合并提交：第二父分支独有的改动不计入模块指标；fixture 中该模块计数与 first-parent-only 期望一致。
- [x] 不修改目标：运行前后 `git status --porcelain` 为空、`git worktree list` 条数不变、`git rev-parse HEAD` 不变。
- [x] 非 git 仓库、无提交仓库、浅克隆（`--depth=1`）分别退出码 `5`，stderr 说明原因；浅克隆不得输出冒充全量的指标。
- [x] 目标为 git 仓库子目录时，必须能识别仓库根与 HEAD：`git -C <subdir> rev-parse --show-toplevel` 与 `git -C <subdir> rev-parse HEAD` 和产物中的仓库信息一致；误报为非 git、或 `head` 为空判红。此判据用于修正 ticket 01 中 `repo.isGit` 只看目标根目录 `.git` 的已知限制。
- [x] 窗口截断时 `truncated=true` 且写明实际扫描提交数；采样模式 `sampled=true` 并写明步长；两者在 JSON 中可见。
- [x] 确定性：同参数两次运行 `history.json` 字节一致（不含墙钟字段）。
- [ ] 性能：excalidraw 默认窗口 3 次运行中位数 ≤10s（推荐值，**待用户确认**；硬件基线待定），读数归档。
- [x] 失败判据：与 git oracle 读数不符、目标仓库被修改、浅克隆/非 git 未返回 `5`、截断/采样未标记、两次运行不同。
- [x] 自动化状态：全部自动化（依赖 CI 镜像预装 `git`；性能门禁依赖硬件基线）。

## 实现与验收记录（2026-11-07）

### 实现要点

- 单次 `git log --first-parent --numstat -M` 取提交链（`%H` / `%P` / `%ct`）与逐提交变更行数，避免逐提交起进程；3 次进程（`show-toplevel`、`is-shallow-repository`、`rev-parse HEAD`）+ 1 次 log 即可完成。
- 合并提交整段跳过 diff（第二父分支改动不计入）；空提交自然无变更；根提交由 `git log` 直接给出全量新增。
- 重命名按 `-M` 检测（含 `dir/{a => b}.ts` 形式），旧路径 → 新路径累积为谱系；谱系 ID 取窗口内最后一次出现的新路径。
- 路径过滤复用 `index` 的 `is_source_file` 与 `is_ignored_dir`：只统计 `ts/tsx/js/jsx/mjs/cjs`，忽略 `node_modules`、`.git`、`dist`、`build`、`out`、`coverage`、`.next` 等目录。
- 退出码：非目录 `2`；非 git / 无提交 / 浅克隆 `5`；序列化或 IO `3`。
- 子目录定位：`git rev-parse --show-toplevel` 取仓库根，`head` 始终非空；该修复只落在 `history` 产物，`index` 的 `repo.isGit` 判断保持 ticket 01 行为（理由见「已知偏差」）。

### fixture 读数（实测）

构造命令：`pwsh tests/fixtures/history-seed/build.ps1`（生成 `tests/fixtures/history`，HEAD `1de5415b9061ef077f05d1ad9f20d8e9e92c737f`）。

| 模块 | firstSeen | lastModified | commitCount | churn（+/-） | activeSpan |
| --- | --- | --- | --- | --- | --- |
| `src/app.ts` | 1767268800 | 1767355200 | 2 | 4 / 1 | 86400 |
| `src/helpers.ts` | 1767268800 | 1767614400 | 3 | 5 / 1 | 345600 |

- `stats.commitsScanned = 6`、`commitsAvailable = 6`、`modules = 2`、`truncated = false`、`sampled = false`、`sampleStep = null`。
- `src/merge-only.ts` 不出现；`src/app.ts` 的 `lastModified` 停在 `c2`，说明合并提交未计入。
- `--max-commits 2` 时 `truncated = true`、`commitsScanned = 2`、`commitsAvailable = 6`，只余 `src/helpers.ts`（`commitCount = 1`）。

### oracle 口径偏差（保留原判据与理由）

- 本票原文的 oracle 使用 `git rev-list --count --first-parent HEAD -- <path>`。fixture 实测该命令对 `src/app.ts` 返回 **3**（把合并提交算进去），与「合并提交第二父分支改动不计入」冲突。
- 采用等价替代：oracle 增加 `--no-merges` 后返回 **2**，与本票指标一致；测试即按 `--no-merges` 断言。原文判据保留在此，偏差与理由一并记录，未静默改写。

### 性能读数

| 场景 | 提交数 | 用时 |
| --- | --- | --- |
| 本仓 Strata（真实仓库，仅 TS/JS 文件计入） | 19 | 0.66s |
| 合成仓库（100 个 TS 文件、500 次提交） | 500 | 0.99s |
| `excalidraw` @ `701b901` 默认窗口 | 500 | **未验证** |

- **构建类型：debug**（`cargo run` 默认 profile；原记录未显式标注，按命令推断）。release 对照未做。

- `excalidraw` 未验证原因：仓内 L2 缓存是归档解压副本、不含 `.git`，当前环境也没有该仓的完整 git 克隆；未尝试全量 clone，以免把未归档的大体量对象塞进工作区。
- 合成 500 提交读数说明进程数与解析开销在预算内；真实大仓的树规模与重命名检测成本待有 `.git` 的样本时复跑确认。

### 已知偏差与 backlog

- `index` 的 `repo.isGit` 仍只看目标根目录 `.git`：若改为 `git rev-parse --show-toplevel`，位于本工作树内的 `tests/corpus/*` 会带上 Strata 自身 HEAD，使第 8 节 pin 的产物 SHA256 随提交漂移；因此本次只修 `history` 产物，`index` 维持 ticket 01 行为。
- `lastModified` 未附带 commit sha（spec 提及「时间与 commit」；本票只输出时间）。
- 未启用采样；超过窗口一律截断并标记。
- 不解析 blob 内容，churn 直接采用 `--numstat` 行数；二进制与超长文件不计数（`--numstat` 的 `-` 视为 0）。
