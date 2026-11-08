# Ticket 19 · N3 演化回放补齐

- **Status:** done
- **档位:** A（跳首尾 / 采样刻度数 / 空洞提示 / 快照 JSON）+ C（单帧 p95 预算）+ B（UI 呈现）
- **依赖:** 票 18（播放核心，已 done，不改其冻结判据）、票 09（时间线刻度与退化标记）、票 07（帧回调语义）
- **范围:** 只补 N3 提案与票 18 之间的 5 项缺口；`TimelinePlayback` 与 07 `PlaybackState` 的重复实现登记为技术债，不在本票重构。

## 1. 跳首 / 跳尾

- 命令：`cargo test -p arch-view --test timeline_playback jump_to_first_and_last_stop_playback -- --nocapture`
- 固定样本：`TimelinePlayback::new(6)`；另测 `new(1)`、`new(0)`、`set_enabled(false)`。
- 期望行为：
  - 播放中 `jump_to_first()` → `current()==0`、`is_playing()==false`、`is_ended()==false`，返回 `true`。
  - `jump_to_last()` → `current()==5`、停止播放、`is_ended()==false`（`ended` 只表示「播放自然到达末帧」），返回 `true`。
  - 重复跳首仍返回 `true` 且 `current()==0`；单刻度时首尾都落 `0`；0 刻度或禁用状态返回 `false`。
- 失败判据：任一目标刻度不符、跳转后仍在播放、`ended` 被置真、禁用/0 刻度仍返回 `true`。
- 自动化状态：已自动化（纯模型断言）。

## 2. 显示已采样刻度数

- 命令：`cargo test -p arch-view --test timeline_playback sampled_tick_count_is_visible_with_holes_and_truncation -- --nocapture`
- 固定样本：`tests/fixtures/history`（HEAD `1de5415`，6 刻度）；合成空洞 4 刻度缺第 1 号；合成 `truncated` 产物 `commitsScanned=500 / commitsAvailable=617`。
- 期望读数：
  - 连续 6 刻度：`sampled_ticks()==6`，`sampling_label()=="已采样 6/6 刻度"`。
  - 4 刻度缺 1：`sampled_ticks()==3`，`sampling_label()=="已采样 3/4 刻度"`。
  - 截断窗口：`sampling_label()=="已采样 6/6 刻度（窗口 500/617 提交）"`。
  - 视图 footer 始终显示该文案，退化标记（`sampled / truncated / shallow`）仍单独显示。
- 失败判据：文案缺刻度数、分母不是总刻度、空洞未扣减、截断窗口未标出、UI 未接线。
- 自动化状态：模型断言已自动化；footer 渲染为部分自动化（窗口 smoke 有 trace 文案，无像素证据）。

## 3. 模型单帧更新 p95 ≤ 16.7ms

- 命令：`cargo test -p arch-view --test timeline_playback model_frame_update_p95_is_within_budget -- --ignored --nocapture --test-threads=1`
- 预算：单帧「`on_frame` + `scrub_to` + 全部模块热力读取」p95 ≤ 16.7ms；3 次运行 × 20 刻度 × 19 步；微秒级计时；样本为 fixture 6 个刻度时间循环扩展成 20 刻度。
- 期望读数：3 次 p95 均 ≤ 16.7ms，且每轮 `current()` 从 0 前进到 19；原始 p95 逐轮打印。
- 失败判据：任一 p95 > 16.7ms、步进未到末刻度、测量需重跑 git/采样（每次刻度耗时包含准备阶段）。
- 自动化状态：已自动化，`#[ignore]` 隔离计时；需手动执行并在汇报中附 3 次读数。

## 4. 状态可复现的字节级 JSON 对比

- 命令：`cargo test -p arch-view --test timeline_playback snapshot_is_byte_identical_across_play_pause_and_reentry -- --nocapture`
- 固定样本：`tests/fixtures/history`，目标刻度 3。
- 期望行为：
  - `snapshot_json()` 输出可被 `serde_json` 解析，含 `tick==3` 与 `artifact.stats.commitsScanned`。
  - 直接 `scrub_to(3)` 的字节，与「新建时间线 → 从 0 播放到 3 → 暂停 → scrub」的字节完全相等。
  - 同一状态重复取快照字节稳定。
- 失败判据：任一路径字节不等、字段缺失、重复取快照不稳定、对比使用解析后相等替代字节相等。
- 自动化状态：已自动化。

## 5. 采样空洞跳到缺失刻度必须显式提示

- 命令：`cargo test -p arch-view --test timeline_playback missing_tick_jump_shows_explicit_gap_notice -- --nocapture`
- 固定样本：fixture 前 4 个刻度，缺失第 1 号（0-based）。
- 期望行为：
  - `scrub_to(1)` 返回 `false`，`current_index()` 保持原值，不静默落到缺失刻度。
  - `gap_notice()` 返回 `Some`，文案含「第 2/4 刻度」「未采样」「历史采样空洞」。
  - 随后 `scrub_to(3)` 成功并清空提示；UI footer 显示提示。
- 失败判据：返回 `true`、索引被改、提示缺失或文案不含刻度号/原因、成功跳转后提示残留。
- 自动化状态：模型断言已自动化；footer 呈现为部分自动化（同 2）。

## 6. 文档口径同步

- 命令：`git diff -- docs/proposals/n3-evolution-playback.md docs/process/NIGHTLOG.md`
- 期望行为：两处原「只设计不实现 / 未拆票」的原文保留，追加 2026-08 日期说明：票 18 已落地播放核心，票 19 补齐 5 项缺口；历史记录不删除。
- 失败判据：改写或删除原文、未写时间点、把 18 与 19 的工作混成同一结论。
- 自动化状态：人工检查（纯文档）。

## 技术债登记

- `TimelinePlayback` 与 07 `PlaybackState` 是两套状态机，语义有重叠（播放/暂停/单步/结束态）。
- 代价：两处都要维护边界用例；07 的 `PlaybackState` 不感知刻度与 dwell，18 的 `TimelinePlayback` 不感知地图视图转场。
- 处理：本票不重构 18 已冻结判据；若后续需要统一，另开票并先补两套状态机的等价性断言。

## 验收五要素汇总

| 项 | 命令 | 固定样本 | 期望 | 失败判据 | 自动化 |
| --- | --- | --- | --- | --- | --- |
| 跳首尾 | `cargo test ... jump_to_first_and_last_stop_playback` | `new(6/1/0)` + 禁用 | 见 §1 | 见 §1 | 已自动化 |
| 采样刻度数 | `cargo test ... sampled_tick_count_is_visible...` | fixture 6 刻度 + 空洞 + 截断 | 见 §2 | 见 §2 | 模型已自动化 / UI 部分 |
| p95 | `cargo test ... model_frame_update_p95... --ignored` | 20 刻度 × 3 轮 | ≤16.7ms | 见 §3 | 已自动化（忽略隔离） |
| 字节级 JSON | `cargo test ... snapshot_is_byte_identical...` | fixture → 刻度 3 | 字节相等 | 见 §4 | 已自动化 |
| 空洞提示 | `cargo test ... missing_tick_jump...` | 4 刻度缺 1 | 见 §5 | 见 §5 | 模型已自动化 / UI 部分 |
| 文档口径 | `git diff -- docs/...` | 两处文档 | 追加说明 | 见 §6 | 人工 |

## 读数记录

- **Red（实现前实测，断言级）**：`cargo test -p arch-view --test timeline_playback -- --test-threads=1 --nocapture` → `7 passed; 4 failed; 1 ignored`（2.70s）。失败均为断言而非编译：
  - `jump_to_first_and_last_stop_playback` 在 `jump_to_first` 返回 `false` 处失败。
  - `sampled_tick_count_is_visible_with_holes_and_truncation`：`sampling_label()` 期望 `"已采样 6/6 刻度"`，实际 `""`。
  - `snapshot_is_byte_identical_across_play_pause_and_reentry`：`parsed["tick"]` 期望 `3`，实际 `Null`。
  - `missing_tick_jump_shows_explicit_gap_notice` 在缺失刻度未被拒绝处失败。
- **Green**：同一命令 → `11 passed; 0 failed; 1 ignored`（3.12s）。
- **Green 前测试修正**：截断样本原先误用 4 刻度 entries，实际读数 `"已采样 4/4 刻度（窗口 500/617 提交）"`；改为 6 刻度后通过。仅修正 fixture 构造，未改判据。
- **p95（C 档，3 轮 × 19 步）**：`cargo test -p arch-view --test timeline_playback model_frame_update_p95_is_within_budget -- --ignored --nocapture --test-threads=1` → run1 p95=0.0125ms、run2 p95=0.0043ms、run3 p95=0.0055ms，worst=0.0125ms ≤16.7ms；每轮 `current()==19`。
- **窗口 smoke**：`cargo run -p arch-view -- timeline tests/fixtures/history --autoplay --auto-close-ms 5000` → exit 0，stdout `playback: 已采样 6/6 刻度`、`timeline: 6 ticks, 3 modules, 1 bands`、`playback: frame tick 2..6`、`playback: ended at tick 6`。
- **时间源扫描**：`rg -n "Instant::now|SystemTime::now|Date::now|rand::|thread_rng" crates/arch-view/src` → 0 命中。
- **回归**：`cargo test --workspace` → 全部 `test result: ok`，0 failed；`cargo fmt --all --check` → exit 0。
- **未验证 / 部分自动化**：跳首/跳尾与采样空洞提示的按钮/footer 只有模型断言和源码接线，未做像素级证据；与票 18 的 UI 缺口一致。
