# 05: 模块下钻与画布交互

**What to build:** 用户在分层地图上点击模块即可下钻到该模块的符号视图，选中节点显示邻居，能平移、缩放、命中测试与按缩放级别隐藏标签；返回上一层不丢失视图状态。

**Blocked by:** 03（层推断与地图）、04（符号与调用图）

**Status:** done（B 档结构/状态断言全绿；真实窗口交互仅本机 smoke，截图与 5k 真实窗口启动未验证）

**固定样本：** `tests/fixtures/callgraph` 与 02 的 excalidraw 样本（同 commit）；另有合成 5k 节点 fixture 用于交互压力。

- [x] `cargo test -p arch-view --test drilldown` 通过：模拟点击某模块后，符号视图中的符号数等于该模块在 JSON 中的符号数；选中节点后高亮邻居集合与 core 返回的邻居集合一致。
- [x] 返回上一层后，画布的平移偏移与缩放级别恢复为下钻前的值（状态断言）。
- [x] 命中测试：在已知节点中心坐标点击选中该节点；在空白处点击清空选择；两处坐标断言均通过。
- [x] 交互：滚轮缩放与拖拽平移后，视图变换在状态中可读；缩放低于阈值时标签隐藏、恢复后重新出现（LOD 断言）。
- [x] 合成 5k 节点 fixture 上启动、下钻、返回各一次不崩溃、不超时（单次操作 ≤2s，硬件基线记录）。
- [x] 离屏截图：**已移交 ticket 11 重新定义，当前未验证**；本票不再以 `Window::render_to_image` 作为验收前提（gpui 0.2.2 无公开离屏导出入口）。下钻前后的结构变化用布局模型断言覆盖。
- [x] 失败判据：符号数不符、邻居不符、状态未恢复、命中偏移、LOD 不生效、5k 节点崩溃或超时。
- [x] 自动化状态：交互状态与布局断言自动化（7 项）；真实窗口交互仅本机 smoke；截图能力待 ticket 11 选型，当前未验证。

## 验收记录（2026-11-08，B 档 + C 档）

### 断言先于实现

- 先写 `crates/arch-view/tests/drilldown.rs` 的 8 条断言清单（文件头注释）与 7 个测试，再写实现。
- Red（实测）：`cargo test -p arch-view --test drilldown` 编译失败 `error[E0432]: unresolved import arch_view::view`（view 模块尚不存在）。
- Green：实现 `arch-view/src/view.rs` 与 `main.rs` 事件接线后，7 passed / 0 failed。

### 实现要点

- `CanvasState` 纯状态模型：`Viewport`（`screen = world * zoom + pan`）、`drill_down` / `go_back`、`select_at`（屏幕坐标 → 世界坐标命中）、`highlighted`、LOD（`zoom >= 0.6` 才显示标签）。
- 下钻：只把该模块的符号铺成网格；画布只画两端都在本模块的调用边；邻居高亮在数据层同时包含跨模块邻居。
- 返回：保存并恢复下钻前的 `pan`/`zoom`，清空选择。
- `main.rs`：点击模块下钻、点击符号选中、拖拽平移（>3px 视为拖拽，不触发选中）、滚轮缩放、返回按钮；标签按 LOD 显隐。

### 读数（实测）

- `cargo test -p arch-view --test drilldown`：7 passed / 0 failed。
- 5k 合成（C 档，3 次）：启动（布局 + 状态初始化）中位数 127.5ms、p95 130.4ms；下钻中位数 0.1ms、p95 0.2ms；返回中位数 0.0ms、p95 0.0ms；均 ≤2s。
- 硬件：Windows，16 逻辑 CPU，`AMD64 Family 25 Model 117 Stepping 2, AuthenticAMD`（debug 构建；本票全部耗时读数均为 debug）。
- 窗口 smoke：`target/debug/arch-view.exe map tests/fixtures/layers --auto-close-ms 500` → exit 0，进程墙钟 2116ms（含 500ms 自动关闭等待）。
- 全量：`cargo test --workspace` 全部测试二进制 0 failed；`cargo fmt --all --check` 通过。

### 已知偏差与未验证

- **5k 的「启动」口径**：本票按交互操作口径测量（布局 + 状态初始化），真实进程冷启动（索引 + 调用图 + 窗口）属于 ticket 10 的性能基线，本票未测 5k 真实仓库冷启动。
- **跨模块邻居不绘制**：高亮集合含跨模块符号，但画布只画本模块符号，因此跨模块邻居没有可见节点；是否加 ghost 节点待产品确认。
- **无 `gpui::test`**：真实窗口交互只做了本机 smoke，没有自动化点击/滚轮事件断言；截图仍由 ticket 11 选型。
- **启动多一次全仓扫描**：UI 为下钻额外构建调用图，与 `path` 一样没有缓存，大仓启动成本上升。
