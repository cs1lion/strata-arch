# 17: P4 双 commit 结构 diff（静态高亮 + 可选 120ms 淡入）

**What to build:** 读取两个 commit 的 tree，按符号数与依赖边集合计算 added / removed / modified 三类结构变更，显式保留疑似重命名两侧的原始记录；桌面端用三种色块静态呈现，可选 120ms 入场淡入，结束停在最终态且可关闭。

**动机：** 回答「这个 commit 改了什么结构」，而不是逐行看 diff。语料 `tests/corpus` 是没有 `.git` 的归档副本，不能在那里取 commit；本轮使用测试内建的合成 fixture 仓库与本仓库自身历史。

**Blocked by:** 04（符号与调用图）、02（模块依赖边）、15/16（动画开关与帧回调接线，不修改其机制）。

**Status:** done（判据先于实现固定；读数在实现后按同口径补充；2026-11-08 补大仓耗时并收口）。

**固定样本/commit：**
- 合成 fixture：测试内建 `git init` 仓库，两个提交（见 `crates/arch-core/tests/commit_diff.rs`）。
- 本仓库自身：`2b66deb` → `29f8e27`（该区间只新增 `tests/fixtures/layers/**` 的 25 个 TS/TSX 文件，`git diff --stat` 为该固定读数）。
- 基线与硬件：开工 commit `a1ab1ac`；`AMD Ryzen 7 8845H`、Windows、PowerShell 7.6.9；`cargo test` debug profile。

## 验收项（五项）

1. **命令**
   - `cargo test -p arch-core --test commit_diff -- --nocapture`
   - `cargo test -p arch-view --test commit_diff_view -- --nocapture`
   - 回归：`cargo test -p arch-view --test context_fade -- --nocapture`、`cargo test --workspace`
   - 时间源检查：`rg "Instant::now|SystemTime::now|Date::now|rand::" crates/arch-view/src` → 0 命中
2. **固定样本/commit**：上述合成 fixture 与 `2b66deb → 29f8e27`；本仓库自身历史以固定 SHA 钉住，不随 HEAD 漂移。
3. **期望读数或行为**
   - 合成 fixture：`added = 3`、`removed = 2`、`modified = 1`、`renames = 1`。
     - added：`src/blank.ts`（空文件，符号数 1）、`src/legacy.ts`（由 `src/old.ts` 改名）、`src/new.ts`。
     - removed：`src/dead.ts`、`src/old.ts`（改名源，仍然保留在 removed 中）。
     - modified：`src/a.ts`（符号数 2→3，原因 `symbol-count`）。
     - 重命名：`src/old.ts ↔ src/legacy.ts`，两侧记录都带 `renameHint`。
   - 命中/漏报：改名对命中 1 组；`src/dead.ts` 与 `src/new.ts` 不构成疑似重命名（符号集合交集 0）。
   - 同一 commit 对比自身：`added / removed / modified / renames` 全为 0。
   - 本仓库自身 `2b66deb → 29f8e27`：`added = 25`、`removed = 0`、`modified = 0`、`renames = 0`。
   - 只读：diff 前后 `git status --porcelain` 与 `git rev-parse HEAD` 不变；不 checkout、不建工作树。
   - 淡入：`progress ∈ {0.0, 0.25, 0.5, 0.75, 1.0}` 时 alpha 单调到 255；`progress ≥ 1.0` 的多帧采样色调值与 alpha 逐位恒定。
   - 可关闭：关闭淡入时每个采样帧的 tint 与静态常量逐位相等（packed RGBA）。
   - 三色块：added / removed / modified 的底色互不相同，且与现有层/符号调色板不冲突。
4. **失败判据**：把符号数变化混进 added；重命名只留一侧或显示为「无变化」；同一 commit 对比自身出现非零变化；空文件被记成 modified；整目录删除漏报；读取 commit 时改动工作区或 HEAD；淡入结束后色调值漂移；关闭态与静态渲染不一致；`arch-view/src` 出现墙钟/随机源。
5. **自动化状态**：`部分自动化`——core diff 与视图模型断言进 CI；真实帧缓冲未采样（`gpui 0.2.2` 无公开离屏导出，见 `docs/acceptance-gaps.md` 第 2.2 节）。

## 判据细节（先于实现固定）

- **三类必须分开**：added = 仅存在于目标 commit 的源文件；removed = 仅存在于源 commit 的源文件；modified = 两侧都存在，且「符号数」或「该文件的出向依赖边集合 `(target, kind)`」发生变化。
- **modified 的判定只按上述两项**，不把纯内容/行数变化算作结构变更。
- **重命名**：`路径相似度 ≥ 0.6` 且 `符号名称集合 Jaccard ≥ 0.8`（两侧非空）判为「疑似」。命中只写 `renameHint` 与 `renames` 列表，不删除、不合并 added/removed 原始记录。宁可多报。
  - 路径相似度 = 同目录 0.4 + 同扩展名 0.2 + 文件名公共前缀比例 0.2 + 公共后缀比例 0.2。
- **数据源**：`git ls-tree` + `git cat-file --batch` 读出两个 commit 的 blob，落到仓库外的临时目录后复用现有 `build_index` / `build_call_graph`；结束即删除临时目录。不 checkout、不碰目标工作区。
- **不改索引产物 schema / 层推断**：新增独立 `commit_diff` 产物与常量，`IndexArtifact`、`LayersArtifact` 的字段与判据均不动。

## Red/Green 证据

### core（A 档）

- Red 提交：`5caf63c test(commit-diff): add assertion-level red for tree diff`。
- Red 命令：`cargo test -p arch-core --test commit_diff -- --nocapture`。
- Red 实测（重放该提交）：`test result: FAILED. 1 passed; 6 failed`；测试二进制编译通过，失败均为断言不成立：
  - `added 计数 left: 0 right: 3`（合成 fixture 期望 3）
  - `removed 计数 left: 0 right: 2`
  - `疑似重命名命中数 left: 0 right: 1`
  - `空文件应出现在 added`
  - `added 计数 left: 0 right: 25`（本仓库 `2b66deb → 29f8e27`）
  - `diff 必须真的读到提交内容`
- Green 命令与实测：同命令 `test result: ok. 7 passed; 0 failed`。

### view（B 档）

- Red 提交：`bde8afb test(diff-view): add assertion-level red for tints and entrance fade`。
- Red 命令：`cargo test -p arch-view --test commit_diff_view -- --nocapture`。
- Red 实测（重放该提交）：`test result: FAILED. 2 passed; 3 failed`：
  - `改名不得合并成一行 left: 0 right: 6`
  - `最终态 alpha 必须到 255 left: 0 right: 255`
  - `关闭淡入后 progress=0 必须与静态渲染逐位一致 left: 795557632 right: 795557887`
- Green 命令与实测：同命令 `test result: ok. 5 passed; 0 failed`。

## 读数（改前 / 改后）

### 结构 diff 计数

| 样本 | added | removed | modified | 疑似重命名 | 命令 |
| --- | --- | --- | --- | --- | --- |
| 合成 fixture（测试内建） | 3 | 2 | 1 | 1 | `cargo test -p arch-core --test commit_diff synthetic_diff_separates_added_removed_modified -- --nocapture` |
| 本仓库 `2b66deb → 29f8e27` | 25 | 0 | 0 | 0 | `cargo test -p arch-core --test commit_diff repository_history_pair_has_expected_counts -- --nocapture` |

合成 fixture 明细（同命令原始输出）：

- added：`src/blank.ts`（空文件，符号数 1）、`src/legacy.ts`（改名目标）、`src/new.ts`。
- removed：`src/dead.ts`、`src/old.ts`（改名源，仍在 removed 中）。
- modified：`src/a.ts`，符号数 2→3，reason `symbol-count`。
- 疑似重命名命中 1 组：`src/old.ts ↔ src/legacy.ts`（路径相似度 0.6、符号 Jaccard 1.0）。
- 漏报对照：`src/dead.ts` 与 `src/new.ts` 路径相似度 0.6，但符号 Jaccard 0，按规则不提示（测试断言两者 `renameHint = None`）。
- 反向用例：同一 commit 对比自身 `0/0/0/0`；整目录删除 `removed = 2`；空文件留在 added（符号数 1），未进 modified。
- 只读：diff 前后 `git status --porcelain` 与 `git rev-parse HEAD` 均不变；同一输入两次运行 JSON 字节一致。

### 淡入最终态与关闭态

- 启用淡入：`progress = 0 → 0.25 → 0.5 → 0.75 → 1.0` alpha 单调；`alpha_at(1.0)` 的 alpha 字节 = `255`。
- 最终态恒定：`progress ∈ {1.0, 1.0, 2.0}` 的三帧，三类 tint 的 packed RGBA 均等于静态 `packed_static`。
- 关闭淡入：`progress ∈ {0.0, 0.1, 0.5, 0.9, 1.0, 2.0}`，三类 tint 的 packed RGBA 与静态渲染逐位相等。
- 三色块：`added = 0x2f6b3f`、`removed = 0x7a3138`、`modified = 0x7a5b24`，互不相同。
- 时间源：`rg "Instant::now|SystemTime::now|Date::now|rand::|thread_rng" crates/arch-view/src` → 0 命中，退出码 1；淡入进度由 `with_animation` 的 GPUI 帧回调写入。

### 回归与门禁

- `cargo test --workspace`：全部套件 `0 failed`。
- `cargo fmt --all --check`：通过。
- drift CI：
  - `cargo run -p arch-cli -- drift tests/corpus/tsx --base eval/baseline/drift-tsx.json` → exit 0，四类 `newCount = resolvedCount = 0`。
  - `cargo run -p arch-cli -- drift tests/corpus/excalidraw --base eval/baseline/drift-excalidraw.json` → exit 0，四类全 0。
- 未触碰 ticket 14/15/16 的既有判据与截图基线。
- `arch-view diff . --from 2b66deb --to 29f8e27 --auto-close-ms 1200`：窗口启动、渲染入口淡入、自动关闭，exit 0；stdout
  `added=25 removed=0 modified=0 renames=0`。

## 做到 / 没做到 / 未验证

- **做到的**：上述计数、疑似重命名命中/漏报、同 commit 零变化、整目录删除、空文件、只读与确定性、三色块、淡入最终态恒定、关闭态逐位一致、时间源 0 命中、全量回归、drift CI、`arch-view diff` 窗口启动与干净退出均实测通过。
- **没做到的**：未从真实帧缓冲采样最终色值（`gpui 0.2.2` 无公开离屏导出）；未提供 headless JSON 子命令（读数由测试与视图窗口 stdout 给出）。
- **未验证的**：大仓库双 commit 快照的耗时与内存未测（本轮样本最大为 802 模块的 excalidraw 基线，但 P4 未对其跑 diff）；三色块的实际屏幕对比度未测；改名规则在真实仓库中的误报率只有合成样本的漏报对照；`with_animation` 的实际帧数与时长未打点。

## 大仓耗时（2026-11-08 收口）

- 命令：`cargo test -p arch-core --test commit_diff large_corpus_commit_diff_wall_clock -- --ignored --nocapture --test-threads=1`
- 固定样本：`tests/corpus/excalidraw`（1328 个文件、54.3MB、802 模块基线）复制到临时目录后 `git init`；base commit 为完整语料，head commit 新增 `perf-probe.ts`（`added=1`）。不使用语料原目录，因为它没有 `.git`。
- 实测读数：`large corpus commit diff: 10589 ms; added=1 removed=0 modified=0 renames=0`；测试总时长 25.97s（含复制语料与两次 commit，计时区间只包 `diff_commits`）。
- 口径：耗时包含两次 tree 物化、索引/调用图/依赖边构建与三类比对；不含测试的语料复制与 git commit。票 17 为 A+B 档，未钉性能阈值，本读数只作收口记录，不回调任何判据。
- 仍未验证：真实帧缓冲色值采样（`gpui 0.2.2` 无公开离屏导出）、屏幕对比度、真实仓库改名误报率、`with_animation` 实际帧数。
