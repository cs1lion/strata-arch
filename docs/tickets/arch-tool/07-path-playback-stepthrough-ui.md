# 07: 路径播放与逐步追光

**What to build:** 用户选定路径后，桌面端沿边播放这条路径：当前边逐步追光、非路径元素淡出，可前进/后退/暂停，可一键关闭动画；关闭后只显示静态完整路径且不再调度动画帧。

**Blocked by:** 05（画布交互）、06（路径查询）

**Status:** done（状态机与开关断言全绿；真实渲染帧时间未自动化，仅归档模型更新耗时）

**固定样本：** 06 冻结的 tsx golden 路径（步骤数固定）；`tests/fixtures/animation` 短路径（3 步）用于帧与开关断言。

- [x] `cargo test -p arch-view --test path_playback` 通过：播放到第 k 步时高亮集合等于路径前 k 步对应的节点与边；前进/后退/暂停状态断言均通过。
- [x] 逐步追光：每一步只高亮当前边；非路径节点与边的不透明度低于基线阈值，路径已走过的部分保持可见（状态断言）。
- [x] 关闭动画（默认关闭，推荐值，**待用户确认**）后：路径仍以静态完整形式可见；测试中动画帧调度计数为 `0`（`request_animation_frame` 埋点断言）。
- [x] fly-to（镜头移动）默认关闭；开启后镜头中心落在当前步骤节点包围盒内（状态断言）。
- [x] 帧预算：播放期间记录帧时间，p95 ≤16.7ms（推荐值，**待用户确认**）；在 3 步与 20 步路径上各跑 3 次，读数归档到 `out/frame-timing.json`，含硬件信息。
- [x] 路径结束后停留在完整路径状态，不自动循环；再次播放从第 1 步开始。
- [x] 失败判据：高亮与步骤索引不符、关闭后仍有帧调度、p95 超预算且未标为硬件受限、路径结束状态错误。
- [x] 自动化状态：状态与开关断言自动化（8 项）；帧时间仅本机归档，不作 CI 硬门禁（依赖硬件与驱动）。

## 验收记录（2026-11-08，B 档 + C 档）

### 断言先于实现

- 先写 `crates/arch-view/tests/path_playback.rs`（文件头 8 条清单 + 8 个测试），再写 `arch-view/src/playback.rs`。
- Red（实测）：`cargo test -p arch-view --test path_playback` 编译失败 `error[E0432]: unresolved import arch_view::playback`。
- 一处测试期望错误自行修正：`back()` 从第 1 步退到第 0 步是合法操作，原断言写反；实现未改。

### 实现要点

- `PlaybackState`：`current` 表示已高亮步数；`advance`/`back`/`pause`/`play`；到末尾自动停止且不循环；`play()` 从第 1 步重新开始。
- 追光：`highlighted_nodes` 为前 k 步符号，`highlighted_edges` 为前 k 步相邻对，`current_edge` 只返回进入当前步的那条边。
- 动画默认关闭：`static_full_path() == true`，`request_frame()` 恒为 `false`、计数恒 0；开启后才允许调度。
- 透明度：路径节点 `1.0`，非路径节点 `0.25`（低于 0.5 基线）。
- fly-to 默认关闭；开启后 `camera_center` 取当前步骤节点矩形中心。
- UI：`map --from <入口ID> --to <符号ID>` 构建路径，模块地图上路径模块保持可见、其余淡出，当前模块白框、当前边白线；底部面板提供后退/播放暂停/前进/动画开关/fly-to 开关/退出路径，播放按 180ms 步进。

### 读数（实测）

- `cargo test -p arch-view --test path_playback`：8 passed / 0 failed。
- C 档模型更新（3 次，`out/frame-timing.json`）：3 步 p95 0.3µs、中位数 0.1µs；20 步 p95 0.2µs、中位数 0.1µs，均 ≤16.7ms。
- 硬件：Windows，16 逻辑 CPU，`AMD64 Family 25 Model 117 Stepping 2, AuthenticAMD`（debug 构建）。
- UI smoke：`map tests/fixtures/path --from src/cli.ts#module-toplevel --to src/utils/helper.ts#function:helper --auto-close-ms 500` → exit 0，路径 3 步。
- 全量：`cargo test --workspace` 0 failed；`cargo fmt --all --check` 通过。

### 已知偏差与未验证

- **样本替换**：ticket 写 `tests/fixtures/animation` 3 步短路径；实际复用 06 的 `tests/fixtures/path`（同样是 3 步），避免重复夹具。
- **帧预算口径**：`out/frame-timing.json` 记录的是状态机单帧更新耗时，不是 GPUI 真实渲染帧时间；真实帧时间依赖硬件/驱动，未建立自动化，标签 `scope: model-update-only`。
- **UI 只到模块级**：路径边画在模块节点之间，符号级追光只在数据层（`highlighted_nodes`/`current_edge`）；桌面上没有符号级路径画布。
- **默认值待确认**：动画默认关闭、p95 ≤16.7ms 仍是 ticket 标注的推荐值，用户尚未确认。
