# 16: P1 G1 完成版（离散路径步进 + 当前边 + 上下文淡出 + 模块/符号同步）

**What to build:** 在已有路径步进与 P2 相机转场之上，补齐上下文淡出与模块/符号同步。淡出必须是「步骤索引切换时的状态变化」，同一步内透明度恒定；当前步节点及其 incoming/outgoing 边为最高层；非路径元素保持可读下限；保留关闭开关且关闭时逐位回到旧行为；淡化不影响命中框与 z-order；模块级与符号级视图共用同一 `current` 索引。

**动机：** 动画研究 P1 定义（[animation README 2.5 P1](../../research/animation/README.md)）：动的是步骤推进时的当前边切换与非路径元素淡出，没有连续位移。改前模型只有节点透明度（路径 1.0 / 非路径 0.25），符号场景只强调 incoming 边，且模块/符号焦点靠各自索引数组。

**Blocked by:** 06/07（路径与步骤模型）、05（符号级视图与命中框）、15（P2 转场，不修改其机制）。

**Status:** implemented & verified（2026-11-08；模型与绘制层断言全自动；同日追加本地像素证据链，P1 前后帧可区分，人工项不进 CI，见文末）。

**固定样本/commit：** `tests/fixtures/callgraph`。3 步路径 `src/b.ts#function:caller → src/a.ts#function:alpha → src/util.ts#function:helper`（三步均 `unclassified`，模块分别为 `src/b.ts` / `src/a.ts` / `src/util.ts`）；2 步同模块路径 `src/a.ts#function:beta → src/a.ts#function:alpha`；单步 `alpha → alpha`。开工基线 commit `096877e`。硬件/OS：`AMD Ryzen 7 8845H`、Windows、PowerShell 7.6.9；`cargo test` debug profile。

## 验收项（五项）

1. **命令**
   - `cargo test -p arch-view --test context_fade -- --nocapture`
   - 回归：`cargo test -p arch-view --test playback_wiring -- --nocapture`、`cargo test -p arch-view --test path_playback -- --nocapture`
   - 全量：`cargo test --workspace`；`cargo fmt --all --check`
   - 时间源检查：`rg "Instant::now|SystemTime::now|Date::now|rand::" crates/arch-view/src` → 0 命中
2. **固定样本/commit**：上述四条 fixture 路径；基线 `096877e`。
3. **期望读数或行为**
   - 同一步连续三次采样，节点与边的透明度序列逐位相等；取值只能是 `1.0` 或 `0.68`，不允许中间值。
   - 3 步路径：第 1 步 `current_edges = {(caller,alpha)}`、第 2 步 `{(caller,alpha),(alpha,helper)}`、第 3 步 `{(alpha,helper)}`；当前步节点 opacity `1.0`，上下文节点 `0.68`。
   - 模块级：第 1 步 `current_module_edges = {(b,a)}`、第 2 步 `{(b,a),(a,util)}`、第 3 步 `{(a,util)}`；同模块相邻对不产生模块边。
   - 淡化下限：全部调色板标签对比度 ≥ 5.0:1（WCAG AA 为 4.5:1），实测最低 **5.419:1**（`commands` 层）。
   - 绘制顺序 `Context → Path → Current`，同角色保持原序；当前步最后绘制。
   - 关闭开关：路径节点 1.0 / 非路径 0.25、仅 incoming 边 `strong`、边不淡化、绘制顺序不变。
   - 下钻/返回前后 `current()` 不变；模块焦点与符号焦点都来自同一步。
4. **失败判据**：同一步出现两个不同透明度；透明度出现非 `{1.0,0.68}` 的值；任一步 `current_edges` 漏掉 incoming 或 outgoing；淡化后标签对比度低于 4.5:1；关闭态与旧行为有任何差异；淡化节点无法命中/下钻；模块视图与符号视图步骤索引不一致；四种反向用例出现空指针/零除/错误层级。
5. **自动化状态**：`部分自动化`——8 条模型/绘制层结构断言已进 CI；对比度为按 GPUI primitive alpha 合成模型计算，未从真实帧缓冲采样；`gpui 0.2.2` 无公开离屏导出（见 `docs/acceptance-gaps.md` 第 2.2、10 节）。

## 断言先行清单（B 档证据）

测试文件先于实现提交（`2d1ea7c`），实现提交 `b5d2327`（模型）→ `474e7a8`（绘制层）→ `01e2c08`（UI 接线）。Red 为编译级失败（新 API 尚不存在）：**2×E0432 / 62×E0599 / 1×E0609，退出码 101**。实现后首次运行有 2 条测试自身的 fixture 预期写错（3 步路径的 incoming 边不在 `src/a.ts` 布局；上下文模块未过滤可下钻者），在 `925674d` 修正，判据未放宽。

| 断言 | 可见行为 |
| --- | --- |
| `fade_is_a_discrete_step_state_not_a_timeline` | 同一步多帧透明度恒定，且不出现连续插值 |
| `roles_are_numeric_per_step` | 每步当前节点/边、上下文节点/边、模块角色与数量 |
| `context_floor_keeps_labels_legible` | 0.68 下限下全部调色板标签对比度实测 |
| `paint_order_and_hit_testing_are_preserved` | Context→Path→Current 绘制顺序；淡化节点仍可命中/下钻 |
| `module_and_symbol_views_share_the_step_index` | 下钻/返回不改变步骤索引与当前模块/符号 |
| `reverse_cases_do_not_produce_invalid_layers` | 单步 / 全 `unclassified` / 同模块 / 全覆盖 |
| `fade_off_keeps_the_legacy_path` | 关闭开关逐位回到旧路径（回归判据） |
| `painted_symbol_edges_use_the_step_roles` | 第 1 步 outgoing 边被提升为 strong（旧口径为 0 条） |

## 读数（改前，同一 fixture / 同一统计口径）

实现前模型：路径节点 1.0、非路径节点 0.25；符号边只强调 incoming；模块/符号焦点来自各自的模块序列与符号索引。

```text
legacy: node opacities [0.25, 1.0], strong edges [("src/a.ts#function:beta", "src/a.ts#function:alpha")]
contrast at 0.25 (legacy) minimum: 2.103:1
```

- 每步被淡化元素：符号节点 **29/32**、模块节点 **13/16**；边**不淡化**（旧绘制全部 1.0）。
- 标签可读性：0.25 时最低 **2.103:1**（`function`），低于 WCAG AA 4.5:1，标签已不可辨认。
- 第 1 步没有任何 `strong` 边（旧口径只认 incoming）。

Red 证据（实现前，`2d1ea7c`）：`cargo test -p arch-view --test context_fade` → 2×E0432 / 62×E0599 / 1×E0609，退出码 101。

## 读数（改后，同口径）

命令：`cargo test -p arch-view --test context_fade -- --nocapture`

```text
step 1/3: context nodes 29/32 (legacy 0.25 fades 29/32), context edges 4/6, context modules 13/16
step 2/3: context nodes 29/32 (legacy 0.25 fades 29/32), context edges 4/6, context modules 13/16
step 3/3: context nodes 29/32 (legacy 0.25 fades 29/32), context edges 4/6, context modules 13/16
discrete step 2/3: node opacities [1.0, 0.68, 0.68, 0.68, 0.68, 0.68, 0.68, 1.0, 0.68, 0.68, 0.68, 0.68, 0.68, 0.68, 0.68, 0.68, 0.68, 0.68, 0.68, 0.68, 0.68, 0.68, 0.68, 0.68, 0.68, 0.68, 0.68, 0.68, 0.68, 0.68, 1.0, 0.68]
discrete step 2/3: edge opacities [1.0, 0.68, 0.68, 1.0, 0.68, 0.68]
context floor: minimum 5.419:1 at layer[commands] (WCAG AA needs 4.5:1)
paint order for src/a.ts: [2, 3, 4, 1, 0], roles [Context, Context, Context, Path, Current]
sync: current 2/3, module Some("src/a.ts"), symbol Some("src/a.ts#function:alpha")
reverse: single-step empty edges ok; all-unclassified ok; same-module no module edges ok; full coverage context 0/0
painted edges: 2 total, 1 strong, context opacity 0.68
```

- 每步被淡化元素：符号节点 **29/32**、模块节点 **13/16**、符号边 **4/6**（新增边淡化）；当前步 2 个节点与 3 条边保持 1.0。
- 透明度序列：同一步三次采样逐位相等，仅出现 `1.0` / `0.68`，无中间值。
- 可读性下限：0.68 时最低 **5.419:1**（`commands` 层），高于 AA 4.5:1 约 20%；各调色板序号见测试输出（`function` 5.703、`method` 5.515、`class` 5.587、`unclassified` 6.472）。
- 两视图步骤索引：`current=2/3`，模块焦点 `src/a.ts`、符号焦点 `src/a.ts#function:alpha`；`drill_down → go_back` 后两者不变。
- 绘制顺序：`src/a.ts` 5 个符号按 `[2,3,4,1,0]` 绘制，角色 `[Context, Context, Context, Path, Current]`。
- 关闭开关：`cargo test -p arch-view --test playback_wiring` 3 passed；旧 `current_edge` 仍为 incoming-only，旧绘制边 opacity 全 1.0。
- 全量：`cargo test --workspace` 全绿；`cargo fmt --all --check` 干净；`crates/arch-view/src` 无墙钟/随机源。

## 淡化下限的依据

- `CONTEXT_OPACITY = 0.68`（alpha 字节 `173`）。
- GPUI 的 `div.opacity()` 按 primitive 乘 alpha：节点底色先压到画布/层带，标签再以同一 alpha 压到已淡化的底色。
- 按此模型逐调色板计算 WCAG 对比度 `(L1+0.05)/(L2+0.05)`；0.68 时最低 5.419:1，0.25 时最低 2.103:1。
- 4.5:1 是 AA 正文阈值；0.54 附近会跌破 4.5，0.68 留出约 15% 余量，避免不同显示器/伽马导致标签不可读。
- 标签不单独设更低的透明度：节点与其标签共用同一角色透明度，保证「节点可辨认」与「标签可读」一致。

## 反向用例（四种）

1. **单步路径**（`alpha → alpha`）：`current_edges` / `path_edges` / `module_path_edges` / `current_module_edges` 全为空；当前节点仍为 `Current`。
2. **路径覆盖全部节点**：模块级合成路径覆盖全部 16 个模块 → 可淡出模块 **0**；符号级覆盖 `src/a.ts` 全部符号 → 可淡出符号 **0**；每一步恰好 1 个 `Current`。
3. **起终点同模块**（`beta → alpha`）：`module_path_edges` 与 `current_module_edges` 为空（同模块相邻对跳过），当前模块唯一，其他模块为 `Context 0.68`。
4. **`unclassified` 节点在路径上**：3 步路径三步 layer 均为 `unclassified`，当前步仍为 `Current`、其余路径节点为 `Path`，不会因层缺失降级为 `Context`，也不出现多个 `Current`。

## 实现要点

- 模型（`crates/arch-view/src/playback.rs`）：`VisualRole { Context, Path, Current }`、`EdgeRole`；`current_symbol` / `current_module` 作为两视图唯一焦点来源；`current_edges` = incoming ∪ outgoing；`module_path_edges` 跳过同模块相邻对；`paint_order` / `module_paint_order` 稳定排序。
- 绘制（`crates/arch-view/src/view.rs`）：`PaintedSymbolEdge` 增加 `opacity`；`paint_symbol_edges_with_roles` 按步骤角色给 `strong` 与透明度；旧 `paint_symbol_edges` 签名与语义不变（关闭态回归）。
- UI（`crates/arch-view/src/main.rs`）：`fade_enabled = animation_enabled()`；开启时节点/模块/边的 opacity 与绘制顺序走新路径，关闭时逐位走旧路径；`edge_canvas` 用 `rgba` 输出 0.68 的 alpha；模块路径边在淡出态跳过同模块对；两场景焦点都取 `current_module()` / `current_symbol()`。
- 调色板移到 `view.rs`（`symbol_kind_palette` / `layer_palette` / `band_background_palette`），值与原 `main.rs` 逐一相同，供对比度实测直接引用。

## 与研究文档的差异（保留原判据与理由）

animation README 的 P1 落地要点 2 写「关闭动画时显示静态完整路径（所有路径边可见、当前边不高亮），并停止 ticker」。本轮按用户明确要求「关闭时必须与现在完全一致（回归判据，逐位对比）」执行：关闭态保持旧行为（路径 1.0 / 非路径 0.25、仅 incoming 强调、ticker 行为不变）。两处冲突在此登记；未静默删除 README 原判据，也未借机改关闭态语义。若后续要落地「静态完整路径」，应作为关闭态语义变更单独开票并提供改前/改后读数。

## 像素证据（2026-11-08 追加，人工项，不进 CI）

- 证据钩子：`STRATA_EVIDENCE_ANIMATION=1` 让 `map --from/--to` 启动即处于「动画: 开」态（等价用户在 UI 里点击一次开关），用于抓取关闭态/开启态对照帧；正常运行不受影响。
- 命令：`pwsh -File eval/pixel-evidence.ps1`（固定样本 `tests/fixtures/callgraph`，`src/b.ts#function:caller → src/util.ts#function:helper`）。
- 判定方式：关闭态最后一帧 vs 开启态最后一帧，整帧帧差（通道差 >6 记为变化像素，32×32 热点单元）。
- 阈值与依据：changedRatio ≥2%、热点单元（≥30% 变化）≥10。依据：16 节点中 13 个非路径节点透明度 0.25→0.68，节点瓦片+标签约占画面 10%，取 1/5 余量；13 个节点各至少覆盖 1 格。
- 实测：`changed=7.886%`、`meanAbs=1.256`、`maxCell=100%`、`hotCells=454`、变化 bbox `(37,142)-(2560,1816)`；同次复跑无空白帧。
- 边界：需要可见 Windows 桌面会话与 DPI-aware 抓屏（脚本已设 per-monitor v2，物理尺寸 2728×1879）；不算 CI 自动断言，也不替代逐位像素比对。阈值与完整读数见 `docs/acceptance-gaps.md` 2.2.1。

## 未验证的

- **真实帧缓冲未采样**：淡化透明度与标签对比度按 GPUI primitive alpha 合成模型计算；`gpui 0.2.2` 无公开离屏导出，无法对窗口像素逐位比对。（2026-11-08 追加：已用 Windows `PrintWindow` 采到关闭态/开启态两帧并做帧差，见上；仍非 CI 自动断言，也未逐位比对。）
- **跨节点遮挡的视觉 z-order 未验证**：fixture 布局中节点矩形不重叠，绘制顺序断言只能证明「当前步最后绘制」，不能证明重叠时的实际遮挡；命中测试仍按 layout `z_index`，未改。
- **DPI/刷新率/字体差异**未测；对比度对显示伽马的敏感性未做实测。
- 关闭态的「逐位一致」是结构级判据（旧常量、旧 `current_edge`、旧绘制入口、原迭代顺序），不是像素 diff。
