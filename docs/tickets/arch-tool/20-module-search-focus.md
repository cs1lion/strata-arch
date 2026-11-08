# 20: 模块搜索与聚焦（Enter 聚焦 + 一跳淡出 + 聚焦标签）

**What to build:** 在模块地图上增加最小可用的模块搜索与聚焦：`/`（无修饰）或 `Ctrl/Cmd+F` 打开面板，按模块路径子串匹配并排序；`Enter` 聚焦选中模块、**关闭面板**并保留角落的聚焦标签；聚焦态按一跳集合（上下游直接邻居）淡出其余模块（沿用 P1 的 `0.68` 下限）；`Esc` 一步清除淡出、标签并复原相机。

**动机：** 802 模块语料下，用户无法靠肉眼在地图上找到某个文件；票 13 已把「搜索并定位」列为 TBD。本轮只做模块级定位，先解决「找得到、看得清、退得回」，符号级搜索另议。

**Blocked by:** 05（命中框与画布交互）、15（P2 相机转场，直接复用不改造）、16（P1 上下文淡出与 `0.68` 口径）、19（P1 步进带来的播放态判定）。

**Status:** criteria-first（判据已定，2026-11-09；实现按「模型 → 相机/淡出接线 → 标签 → 像素证据 → 文档」分层推进）。

**固定样本/commit：** 交互与排序：`tests/fixtures/layers`（16 模块，含同 basename 与多级路径）；性能：`tests/corpus/excalidraw`（802 模块，`eval/corpus.toml` 钉住的快照）；像素证据：`tests/corpus/excalidraw`（与既有像素基线同场景）。硬件/OS：`AMD Ryzen 7 8845H`、Windows、PowerShell 7.6.9；性能读数用 `--release`。

## 已定决策（四项）

1. `Enter` 聚焦后关闭面板；`Esc` 一步恢复（不回到输入态）。
2. `map` 带 `--from/--to` 时禁用搜索，并写进 `arch-view map --help` 与 README。
3. 淡出下限沿用 P1 模块口径 `CONTEXT_OPACITY = 0.68`，不新增常量、不改 P1 行为。
4. 本轮只搜模块（`NodeLayout.id`），不搜符号；符号级定位另议。

## 验收项

1. **命令**
   - `cargo test -p arch-view --test module_search -- --nocapture`
   - 回归：`cargo test -p arch-view --test context_fade -- --nocapture`、`cargo test -p arch-view --test playback_wiring -- --nocapture`、`cargo test -p arch-view --test camera_transition -- --nocapture`、`cargo test -p arch-view --test map_layout -- --nocapture`
   - 全量：`cargo test --workspace`；`cargo fmt --all --check`
   - 性能：`cargo run --release -p arch-view -- map tests/corpus/excalidraw` + 匹配耗时探针（脚本/测试输出）
   - 像素证据：`pwsh -File eval/pixel-evidence.ps1`
2. **固定样本/commit**：`tests/fixtures/layers`、`tests/corpus/excalidraw`；基线 commit 为票 20 判据提交。
3. **期望读数或行为**：见下方 C1–C10，逐条给出可观察行为。
4. **失败判据**：见下方 C1–C10 每条末尾；任一成立即不通过，不调阈值、不降判据。
5. **自动化状态**：`部分自动化`——C1–C7、C9 的结构部分、C10 的模型部分进 CI；C8 的 p95 与 C9/C10 的像素帧为人工实测项（`gpui 0.2.2` 无公开离屏导出，见 `docs/acceptance-gaps.md` 第 2.2 节）。

## 判据（9 条 + 追加 1 条）

### C1 触发与状态机

- `/`（无修饰）或 `Ctrl+F`/`Cmd+F` 打开搜索面板，进入输入态；输入态显示 query 与结果列表。
- 输入态 `Esc`：关闭面板、清空 query，不改变相机与淡出。
- 输入态 `Enter`（有选中项）：进入聚焦态并**关闭面板**。
- 聚焦态 `Esc`：一步清除——淡出复原、聚焦标签消失、相机复原到进入前快照，不回到输入态。
- 空命中的输入态 `Enter`：不进入聚焦态、面板保持打开。
- 失败判据：`Esc` 需要两步才回到无焦点态；`Enter` 后面板仍可见；关闭后 query 残留；空命中 `Enter` 进入聚焦。

### C2 匹配规则

- 仅匹配 `NodeLayout.id`（模块路径），子串匹配、大小写不敏感；不匹配符号 id、不匹配标签文本。
- 空串返回 0 命中（不返回全量）。
- 无命中时面板显示 `无匹配 · 0/0`。
- `hit_count` 记录全部命中数；列表最多列前 6 条并显示 `前 6/共 N`；`0/0` 与 `1/1` 不省略。
- 失败判据：空串返回全量；命中数 > 0 却显示无匹配；列表截断后丢失总数；匹配到符号或非路径字段。

### C3 排序

- 四级稳定排序：`basename` 完全相等 > `basename` 前缀 > `basename` 包含 > 仅路径包含；同级按 `id` 升序（字节序）。
- 失败判据：同 query 两次结果不同；同级非 `id` 升序；basename 精确命中被仅路径命中排在前面。

### C4 选中移动

- `↑/↓` 在全部命中中移动选中，**不循环**：首项按 `↑` 停在首项，末项按 `↓` 停在末项。
- 列表可见窗口为最多 6 条；选中项移出窗口时窗口跟随（`list_start = clamp(selected + 1 - 6, 0, hit_count - 6)`），选中项始终可见。
- 失败判据：首项 `↑` 跳到末项（循环）；选中项不可见；窗口起点为负或超出 `hit_count - 6`。

### C5 一跳集合与淡出

- 上游 `N` = `EdgeLayout.target == F` 的来源模块集合，下游 `M` = `EdgeLayout.source == F` 的目标模块集合；同层邻居、二跳节点不进入集合。
- 自环 `(F, F)` 在集合中只出现一次（F 本身），在标签计数中只计一次（归入下游，见 C10）。
- 焦点集合 `S = {F} ∪ 上游 ∪ 下游`：`S` 内模块 opacity `1.0`，`S` 外 `0.68`（复用 `CONTEXT_OPACITY`，不新增取值）。
- 边：任一端点在 `S` 内的边 opacity `1.0`，其余 `0.68`。
- 绘制顺序与 `z_index` 不变，只改透明度；命中框与下钻不受影响。
- 失败判据：出现 `{1.0, 0.68}` 之外的值；自环被计为 2；同层或二跳模块进入 `S`；边出现中间透明度；淡出改变绘制顺序或命中。

### C6 相机

- 聚焦时调用 `begin_fly_to(Some(node.rect.center()), viewport_size)`，复用 P2 的 200ms 转场，不新增参数、不改时长。
- 进入聚焦前快照 `Viewport { pan_x, pan_y, zoom }`；聚焦态 `Esc` 清除时逐字段复原（终值必须相等）。
- 转场关闭（或目标等于当前）时走既有即时定位分支。
- 失败判据：聚焦后相机不动（且节点原本不在视口中心）；`Esc` 后 `pan_x/pan_y/zoom` 任一不等；新增转场参数或改动 200ms。

### C7 互斥与禁用

- P1 播放态（启动带 `--from/--to`，或正在播放/已进入步进态）搜索禁用：按键不打开面板、面板不可见；`arch-view map --help` 与 README 写明「有 `--from/--to` 时禁用搜索」。
- P5 揭示进行中，任意搜索相关按键先 `skip()` 揭示再处理，不叠加两层动画。
- 搜索面板打开或聚焦态时，P1 播放入口不可用（互斥双向）。
- 失败判据：播放态能打开面板；揭示未结束时打开面板导致两层动画叠加；帮助/README 未写禁用条件；聚焦态能开始播放。

### C8 性能

- 802 模块匹配 + 排序中位数 ≤3ms（release；依据：33ms 帧预算的 10%，留出绘制与合成余量）。
- 输入期间（面板打开、连续改 query）帧 p95 ≤33ms：本机 GPU 一次、lavapipe 一次；lavapipe 跑不通则标「未测」并说明原因。
- 失败判据：中位数 >3ms；p95 >33ms 且未给出可复现读数与降级说明。

### C9 不回归与证据

- 无搜索交互时，`eval/map-pixel-compare.ps1`（`tests/corpus/excalidraw`）保持既有判据通过；`cargo test --workspace` 全绿；`cargo fmt --all --check` 干净。
- 像素证据新增三态：**搜索有结果**、**无匹配**、**聚焦态（含标签）**；读数来自 `eval/pixel-evidence.ps1`，人工项不进 CI。
- 失败判据：未触发搜索时与基线出现判据外像素差；证据缺任一状态。

### C10 聚焦标签（用户追加）

- `Enter` 聚焦后面板关闭，画面角落保留**聚焦标签**；内容为聚焦模块 id + `上游 N` / `下游 M` 边计数。
- `Esc` 清除时标签与淡出**同时**消失（同一状态切换，同一帧）。
- 断言：标签显示的 `N`/`M` 与一跳集合实际条目数一致——上游按 `target == F` 计数、下游按 `source == F` 计数；自环只计一次（计入下游，上游不计），焦点集合内 `F` 只出现一次。
- 清除后标签不可见：模型 `focus_label()` 返回 `None`；像素证据的清除后帧不出现标签。
- 失败判据：标签与淡出不同帧消失；标签计数与集合条目不一致；自环被计两次或漏计；清除后仍能读到标签。

## 实现顺序（按层提交）

1. **模型**：`crates/arch-view/src/search.rs`（`SearchState`、排序、选中与可见窗口、`FocusSet`、上下游计数、`FocusLabel`、`Viewport` 快照）+ `crates/arch-view/tests/module_search.rs`（先 Red）。
2. **相机/淡出接线**：`main.rs` 的 `FocusHandle`/`track_focus`/`Window::focus`、`on_key_down`（`/`、`Ctrl/Cmd+F`、`↑/↓`、`Enter`、`Esc`、`Backspace`、可打印 ASCII）、`begin_fly_to` + `VisualRole` 式透明度、`Esc` 复原 viewport、P1 播放态禁用、按键先 `skip()` 揭示。
3. **标签**：角落 `FocusLabel` 渲染（id + 上游 N / 下游 M），与淡出同一状态切换。
4. **像素证据**：`eval/pixel-evidence.ps1` 加三态与聚焦态帧，跑出读数。
5. **文档**：README、`docs/acceptance-gaps.md`、CLI 帮助；按层提交、push、报 CI。

## 已知限制（开工前登记，不假装支持）

- **IME 未接入**：本轮用 `on_key_down` 的 `key_char` 采集 ASCII，不实现 `EntityInputHandler`。中文/日文等输入法组字不保证可用；不宣称支持，后续单开票。
- **符号搜索**：不在本轮范围（只搜模块 id）。
- **像素证据**：依赖可见 Windows 桌面会话，不进 CI；与既有 P1/P5 证据同口径。

## 未验证的

- 802 模块匹配耗时与输入期间帧 p95：待实现后实测（本轮报告给出）。
- 三态像素读数：待实现后实测。
- `Enter` 聚焦标签在不同 DPI/字体下的换行与截断：本轮不做自适应，只保证 100% DPI 下完整可读；未测其他 DPI。
