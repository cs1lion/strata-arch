# 17: P4 双 commit 结构 diff（静态高亮 + 可选 120ms 淡入）

**What to build:** 读取两个 commit 的 tree，按符号数与依赖边集合计算 added / removed / modified 三类结构变更，显式保留疑似重命名两侧的原始记录；桌面端用三种色块静态呈现，可选 120ms 入场淡入，结束停在最终态且可关闭。

**动机：** 回答「这个 commit 改了什么结构」，而不是逐行看 diff。语料 `tests/corpus` 是没有 `.git` 的归档副本，不能在那里取 commit；本轮使用测试内建的合成 fixture 仓库与本仓库自身历史。

**Blocked by:** 04（符号与调用图）、02（模块依赖边）、15/16（动画开关与帧回调接线，不修改其机制）。

**Status:** in progress（判据先于实现固定；读数在实现后按同口径补充）。

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

（实现前在此登记真实失败输出；未实测不预填。）

## 读数（改前 / 改后）

（实现后按同口径补充。）
