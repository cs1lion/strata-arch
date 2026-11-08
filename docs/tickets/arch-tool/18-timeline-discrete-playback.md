# 18: P3 时间线离散播放（按 commit dwell，不插值）

**What to build:** 演化时间线可按 commit 刻度播放：每个刻度 dwell 一段时间后离散跳变到下一个刻度；可暂停、单步、调速；到最后一个刻度显式停止；采样/截断时画面有可见标记。研究口径见 [animation README](../../research/animation/README.md) 的 P3 与 T5/T3。

**Blocked by:** 09（时间线状态）、18 依赖的采样修复已收口（`build_history_prefixes`）。

**Status:** done（Red 断言级已取得；模型与 UI 接线已跑绿；UI 点击仍只有 smoke。2026-11-08 解冻复核：判据不变，7 个播放测试复跑全绿）

**固定样本：**

- `tests/fixtures/history`（6 个 first-parent 刻度，`build_history_prefixes` 生成）。
- 合成：3 个刻度复用同一 artifact（「所有 commit 指标完全相同」）；1 个刻度（「无可播放」）。

## 判据（先钉死）

1. **不插值**：任何一帧画出的热力状态必须**完整等于某一个刻度的实测值**；同一 dwell 内所有帧的热力向量逐位相同；切刻度那一帧直接跳到下一刻度，不存在两刻度混合的中间态。
2. **可暂停、可单步、可调速**：暂停后任意帧不推进；单步只前进/后退 1 个刻度并自动暂停；速度档位 `0.5x / 1x / 2x`，dwell 基数 `700ms`（`0.5x` 实际 1400ms，`2x` 实际 350ms）。
3. **明确结束态**：推进到最后一个刻度后 `playing=false`、`ended=true`，后续帧不再改变索引；不循环。重播只能由用户再次显式播放触发。
4. **退化为可见标记**：`sampled` 或 `truncated` 时返回固定文案「采样结果（非完整历史）」；仅 `shallow` 时返回「浅克隆（历史不完整）」；正常历史返回 `None`。
5. **时间源**：播放推进只消费 GPUI 帧回调（`cx.on_next_frame` + `with_animation` 进度）；`arch-view/src` 内 `Instant::now` / `SystemTime::now` / `rand::` / `thread_rng` 命中必须为 0（沿用现有扫描测试）。
6. **可关闭**：关闭播放（`set_enabled(false)`）后不再调度帧，索引与其热力与静态时间线完全一致。
7. **不暗示连续变化**：不画刻度内进度条、不做刻度间 cross-fade；只显示离散的「第 k/N 刻度」与当前刻度高亮；文案不得出现「执行/流/连续」。
8. **反向用例**：1 个刻度不可播放；所有刻度指标相同则每刻度热力向量相同（不得伪造波动）；播放中拖动时间轴必须停止播放并跟到目标刻度。

## 命令与期望读数

`cargo test -p arch-view --test timeline_playback -- --test-threads=1 --nocapture`

- 期望：7 个测试全绿；打印每刻度 dwell 帧数、相同指标刻度的热力向量、关闭后与静态索引。
- 失败判据：任一 dwell 内出现两个刻度的混合热力；暂停/关闭后仍推进；结束态回绕；标记缺失；`arch-view/src` 命中墙钟/随机源；单刻度可播放；拖动后仍在播放。

## 实现

- `TimelinePlayback`（`crates/arch-view/src/timeline.rs`）：纯状态机，消费调用方给的毫秒增量；`play / pause / step_forward / step_back / cycle_speed / on_scrub / on_frame / should_drive_frames`。
- dwell 基数 `PLAYBACK_DWELL_MS = 700ms`，速度 `0.5x / 1x / 2x`；每帧最多前进 1 个刻度并清零累计，保证每个 commit 至少渲染一帧。
- `TimelineView`（`crates/arch-view/src/main.rs`）：`with_animation` 循环进度 → `Cell<f32>` → `cx.on_next_frame` 读取差值 → 模型推进；暂停/结束后不再包动画元素，帧自然停止。
- 刻度点击同时 `playback.on_scrub(index)`；播放中拖动即停止并跟手。
- footer 增加：播放开关、播放/暂停、上一步/下一步、速度、离散位置「第 k/N 刻度」、结束态「播放结束」、退化标记文案。
- `--autoplay`：冒烟模式，开始即播放并把帧回调驱动的刻度切换打印出来。

## Red / Green（实测）

**Red（模型为骨架、`play/on_frame/step` 均返回 false，测试可编译）：**

```
running 7 tests
panicked at tests\timeline_playback.rs:151:5: assertion failed: playback.play()
...
test result: FAILED. 2 passed; 5 failed; 0 ignored
```

失败原因是断言不成立（`play()` 返回 false、`on_frame` 不推进、`step` 不生效），不是编译错误。

**Green（同一命令）：**

```
test result: ok. 7 passed; 0 failed
dwell 帧数（16ms/帧, 1x, 每段到切换为止）: [44, 44, 44, 44, 44]; 热力向量维度 3
相同指标刻度热力向量（2 次推进）: [Heat { color: [85, 143, 165], size_scale_percent: 111 }, Heat { color: [85, 143, 165], size_scale_percent: 110 }]
关闭播放后索引=5, 静态索引=5, 热力向量一致
```

**帧回调冒烟（真实窗口）：** 构建类型：**debug**（`target/debug/arch-view.exe`）；`700ms` 是设计常量，不是实测耗时。

`target/debug/arch-view.exe timeline tests/fixtures/history --autoplay --auto-close-ms 5000` → exit 0：

```
timeline: 6 ticks, 3 modules, 1 bands
playback: frame tick 2 speed 1x
playback: frame tick 3 speed 1x
playback: frame tick 4 speed 1x
playback: frame tick 5 speed 1x
playback: frame tick 6 speed 1x
playback: ended at tick 6
```

（起始刻度 1 不打印；6 个刻度在 1x 下约 3.5s 播完并在 5000ms 自动关闭前进入结束态。）

**大历史冒烟（tsx 617 刻度）：**

`target/debug/arch-view.exe timeline %TEMP%\tsx-hist --autoplay --auto-close-ms 5000` → exit 0；`timeline: 617 ticks, 136 modules, 4 bands`，帧推进 `tick 2 → 7`（每刻度约 700ms）。该历史默认 `max_commits=500`，每个刻度 `truncated=true`，footer 走 `degradation_label` 显示「采样结果（非完整历史）」；标记文案与接线有断言，**未做像素证据**。

**时间源扫描：** `view_sources_do_not_query_wall_clock_or_randomness` 递归扫描 `arch-view/src`，禁用项 `Instant::now / SystemTime::now / Date::now / rand:: / thread_rng` 命中 0，通过。

## 已知偏差与未验证

- **未做研究里的「可选 150ms cross-fade」**：本票判据 1 要求刻度之间不存在中间热力态，cross-fade 会违反该判据；保留研究原文，按更严格口径实现为纯离散切换。
- **UI 点击未自动化**：播放/暂停/单步/速度/toggle 只做了窗口冒烟，没有 `gpui::test` 事件断言；没有像素证据（与 ticket 11 的截图缺口一致）。
- **未接入系统 reduced-motion**：动画开关是应用内的「播放:开/关」，未读取系统偏好。
- **未显示 commit 日期/数量**：footer 的离散位置只有「第 k/N 刻度」，commit hash/subject 仍在原有位置；研究 T4 的「显示日期/commit 数」未做。
- **速度切换只做了模型断言**：UI 上的调速点击未做交互自动化。
