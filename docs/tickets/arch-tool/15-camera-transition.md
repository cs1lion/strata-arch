# 15: P2 短镜头转场（150–250ms 视口插值）

**What to build:** 把 `apply_fly_to` 的一次性瞬移替换为 150–250ms 的视口插值转场。进度必须由 GPUI 的动画帧/帧回调驱动，禁止墙钟；保留关闭转场回到瞬移的开关；手动平移/缩放可中断且相机停在当前插值位置；转场途中点击按当前插值相机换算坐标。

**动机：** 动画研究 P2 定义（[animation README 4.2](../../research/animation/README.md)）：视口中心插值解决下钻/跳转后的方向感与 object constancy。研究同时点名「转场中点击错位」这一风险，必须有用例覆盖。改前实测：一次跳变 **590.8164px**，无中间帧。

**Blocked by:** 05（`CanvasState` / viewport / 命中框）、07（`camera_center` 模型）。

**Status:** implemented & verified（2026-11-08；UI 实际帧数未验证，见文末）。

**固定样本/commit：** `tests/fixtures/callgraph`；开工基线 commit `a030101`（本票 fork 点 HEAD）。硬件/OS：`AMD Ryzen 7 8845H`、16 逻辑 CPU、Windows、PowerShell 7.6.9；`cargo test` debug profile。

## 验收项（五项）

1. **命令**
   - `cargo test -p arch-view --test camera_transition -- --nocapture`
   - 回归：`cargo test -p arch-view --test playback_wiring -- --nocapture`
   - 全量：`cargo test --workspace`；`cargo fmt --all --check`
   - 时间源检查：`rg "Instant::now|SystemTime::now|Date::now|rand::" crates/arch-view/src`
2. **固定样本/commit**：`tests/fixtures/callgraph`（`src/a.ts` 的 `beta → alpha` 路径与符号布局）；基线 `a030101`。
3. **期望读数或行为**
   - 转场中间帧相机中心严格落在起终点之间且不等于两端；`p=0.5` 为几何中点。
   - 转场关闭时视口与旧 `apply_fly_to` 完全一致。
   - 起终点相同 / 目标缺失 / fly-to 关闭：不启动转场、视口不变。
   - 手动平移/缩放中断：相机停在当前插值位置，不弹回起点、不跳到终点。
   - 转场途中点击命中按当前插值相机换算。
   - 单步路径走完精确落在终点，无上冲/NaN。
4. **失败判据**：只到终态但没有中间态（瞬移）；关闭开关后视口与旧行为有任何差异；中断后位置等于起点或终点；转场中点击命中错误节点或落空；出现 NaN/Inf；生产代码出现墙钟或随机源。
5. **自动化状态**：`部分自动化`——模型层 7 条结构断言已自动化；GPUI 实际帧数/总时长无公开离屏入口，无法自动实测（登记见 `docs/acceptance-gaps.md` 第 3、8 节）。

## 断言先行清单（B 档证据）

测试文件先于实现提交（`0dd2232`），实现提交 `894b79a`。断言与可见行为映射：

| 断言 | 可见行为 |
| --- | --- |
| `transition_passes_through_intermediate_camera_centers` | 相机沿起终点之间逐帧移动，而非一次跳变 |
| `transition_off_is_identical_to_instant_fly_to` | 关闭转场的回归判据：与旧瞬移逐位一致 |
| `same_endpoints_and_missing_target_start_no_transition` | 已在目标 / 无目标时不产生空转 |
| `manual_pan_interrupts_and_keeps_the_interpolated_camera` | 拖动中断，相机停在插值位置 |
| `manual_zoom_interrupts_and_keeps_the_interpolated_camera` | 缩放中断，锚点处的插值中心不跳 |
| `clicks_during_transition_use_the_interpolated_camera` | 转场中点击不按起点/终点换算 |
| `single_step_path_transition_lands_without_overshoot` | 单步路径无异常中间态 |

## 读数（改前，实测瞬移）

命令（实现前执行）：`cargo test -p arch-view --test playback_wiring fly_to -- --nocapture`

```text
fly-to: delta 590.8164px, before Point { x: 600.0, y: 400.0 }, after Point { x: 130.0, y: 42.0 }, target Point { x: 130.0, y: 42.0 }
```

- 相机中心序列：`(600,400)` → `(130,42)`，**1 步**，无中间帧。
- 位移：**590.8164px**。

Red 证据（实现前）：`cargo test -p arch-view --test camera_transition` → **22 个 E0599**（`begin_fly_to` / `set_transition_progress` / `transition` 尚不存在），退出码 101。

## 读数（改后，同口径）

命令：`cargo test -p arch-view --test camera_transition -- --nocapture`

```text
transition centers: p=0.00 (600.00,400.00) p=0.25 (482.50,310.50) p=0.50 (365.00,221.00) p=0.75 (247.50,131.50) p=1.00 (130.00,42.00)
transition terminal error: 0.000000px, displacement: 590.8164px
transition-off viewport: leg=Viewport { pan_x: 470.0, pan_y: 358.0, zoom: 1.0 } off=Viewport { pan_x: 470.0, pan_y: 358.0, zoom: 1.0 }
pan interrupt: from Point { x: 600.0, y: 400.0 } -> mid Point { x: 365.0, y: 221.0 } -> after Point { x: 350.0, y: 246.0 }
click consistency: click (601.00,221.00) world (366.00,42.00) from-world (601.00,221.00) to-world (131.00,-137.00) hit Some("src/a.ts#function:beta")
```

- 中间帧中心序列（模型层 5 个进度采样）：`(600,400) → (482.5,310.5) → (365,221) → (247.5,131.5) → (130,42)`。
- 终态误差：**0.000000px**；总位移仍为 **590.8164px**（与改前同口径）。
- 关闭转场：`pan=(470,358)` 两条路径逐位一致。
- 时间源：`Animation::new(Duration::from_millis(200))`（GPUI 内部时钟推进），`crates/arch-view/src` 中 `Instant::now|SystemTime::now|Date::now|rand::` **0 命中**。
- 测试结果：camera_transition **7 passed**；`cargo test --workspace` 全绿；`cargo fmt --all --check` 干净。

## 反向用例（四种）

1. **单步路径**：`alpha → alpha`，4 段进度走完无上冲、无 NaN，终态精确落在目标。
2. **起终点相同**：以当前相机中心为目标 → `begin_fly_to` 返回 `false`，视口不变。
3. **fly-to 关闭**：`camera_center` 返回 `None` → 不产生目标；UI 侧关闭 fly-to 会取消进行中的转场并清空锚点。
4. **转场中被用户打断**：`pan_by` / `zoom_at` 先取消转场再叠加用户操作，相机停在当前插值位置（`(365,221)` → `(350,246)`），既不回 `(600,400)` 也不跳 `(130,42)`。

## 实现要点

- 模型（`crates/arch-view/src/view.rs`）：`CameraTransition { from, to, progress }`；`begin_fly_to` 启动、`set_transition_progress` 写入、`cancel_transition` 中断；`transition_enabled=false` 时 `begin_fly_to` 直接走旧 `apply_fly_to`。命中测试继续用 `self.viewport`，因此始终是当前插值相机。
- UI（`crates/arch-view/src/main.rs`）：步骤变化时启动转场；`with_animation(("camera-transition", generation), Animation::new(200ms))` 的 animator 把每帧 delta 写入共享 `Cell`；`cx.on_next_frame` 每帧读出并写入相机，转场未结束时续订下一帧。播放面板新增「转场: 开/关」。

## 未验证的

- **GPUI 实际帧数与总时长未实测**：`with_animation` 用 GPUI 内部 `Instant` 推进，`gpui 0.2.2` 无公开离屏导出，测试时钟也不接管 `Animation` 的时钟；可实测的只有模型进度序列与 200ms 的固定 `Animation` duration。已登记到 `docs/acceptance-gaps.md`。
- 真实窗口下的转场观感、DPI/刷新率差异未做人工录屏。
- 视觉迭代：未调整任何像素/布局参数；转场只改视口插值，不动相机模型与布局。
