# 22: IME 中文输入接入（搜索面板，Windows 先行）

**What to build:** 让票 20 的搜索面板支持中文等 IME 组字输入：`MapView` 实现 gpui 0.2.2 公开的 `EntityInputHandler`，组字（marked text）期间在面板 query 区显示组字串，提交后进入既有子串匹配；取消组字不残留。ASCII 路径与票 20 全部判据不回归。平台范围 **Windows 先行**（2026-11-16 用户裁决）；macOS/Linux 显式登记为未验证，不宣称支持。

**动机:** 票 20「已知限制」登记 IME 未接入：`handle_key` 只采 `keystroke.key_char` 的可打印 ASCII（`crates/arch-view/src/main.rs:2347-2394`），中文组字不可用。对中文用户这是搜索功能的硬伤；模块路径含中文文件名的仓库完全无法定位。

**Blocked by:** 20（搜索状态机与 C1–C10 判据，直接复用不改造）。

**Status:** proposal-only（2026-11-16 用户批准开票；未实现，无验收读数）。

## 可行性事实（2026-11-16 源码核实，已验证）

- `gpui 0.2.2` 存在公开 trait `EntityInputHandler`（`gpui-0.2.2/src/input.rs:10`）：`text_for_range` / `selected_text_range` / `marked_text_range` / `unmark_text` / `replace_text_in_range` / `replace_and_mark_text_in_range`，偏移口径为 **UTF-16**。
- Windows 平台已处理 `WM_IME_STARTCOMPOSITION` / `WM_IME_COMPOSITION`（`gpui-0.2.2/src/platform/windows/events.rs:102-103`），经 `ImmGetContext` + `GCS_COMPSTR` / `GCS_RESULTSTR` 取组字串与提交串（events.rs:631-686）。
- 结论：**不需要破坏 `gpui = "=0.2.2"` 与 `--locked`**。macOS/Linux 的 IME 路径本轮未核实，不在范围。

## 已定决策

1. 平台范围 Windows 先行；macOS/Linux 登记未验证，另票补齐（用户裁决 2026-11-16）。
2. 匹配规则不变：仍是 `NodeLayout.id` 子串匹配（票 20 C2），不新增模糊匹配、不搜符号；CJK 无大小写问题，大小写不敏感规则对 CJK 无副作用。
3. 组字期间 marked text 只显示、不参与匹配；匹配只消费已提交文本。组字取消（Esc / 切换焦点）后 marked 清空，query 回到组字前状态。
4. 既有 ASCII `on_key_down` 路径保留；`EntityInputHandler` 提交路径与 ASCII 路径汇入同一 `SearchState` 变更入口，不产生第二套状态机。
5. 不改变票 20 的状态机语义：输入态 / 聚焦态 / 关闭态的转移与 `Esc` 一步复原不变；组字态是输入态的子状态，不是第四个顶层状态。

## 验收五项

1. **命令**
   - 新增（先 Red）：`cargo test -p arch-view --test ime_input -- --nocapture`
   - 回归：`cargo test -p arch-view --test module_search -- --nocapture`、`cargo test -p arch-view --test context_fade -- --nocapture`、`cargo test -p arch-view --test playback_wiring -- --nocapture`、`cargo test -p arch-view --test camera_transition -- --nocapture`
   - 全量：`cargo test --workspace`；`cargo fmt --all --check`；`cargo clippy --locked --workspace --all-targets -- -D warnings`
   - 窗口级 CJK 注入（本机人工项）：`pwsh -File eval/capture-window.ps1 -Keys <WM_CHAR CJK 序列>`（脚本需按 I4 扩展；现有 `-Keys` 走 `PostMessage(WM_KEYDOWN/WM_KEYUP)`，见 acceptance-gaps 2.2.1）
   - 真机 IME（人工项）：按 I6 协议执行并归档
2. **固定样本/commit**
   - 交互与状态机：`tests/fixtures/layers`（26 模块 / 19 边 / 8 层带，含 6 个 `unclassified`；2026-11-16 实测更正，原沿用票 20 正文的「16 模块」与该 fixture 当前实测不符，fixture 自 `29f8e27`(2026-11-07) 未变更）
   - CJK 匹配：新增 `tests/fixtures/ime`（含中文文件名模块，如 `src/组件/按钮.tsx`、`src/组件/输入框.tsx`、`src/utils/format.ts`；模块集合与期望命中集合在票内钉死，fixture 独立提交）
   - 基线 commit：本票判据提交
3. **期望读数或行为**：见 I1–I6，逐条可观察。
4. **失败判据**：见 I1–I6 每条末尾；任一成立即不通过，不调判据、不放宽匹配规则。
5. **自动化状态**：`部分自动化`——I1/I2/I3/I5 的模型与结构断言进 CI（headless 可跑）；I4 窗口注入与 I6 真机 IME 需可见 Windows 桌面会话与真实输入法，为本机人工项，登记 `docs/acceptance-gaps.md`。

## 判据（6 条）

### I1 组字状态机（模型，A 档）

- `SearchState` 新增组字子状态：`start_composition` / `update_composition(text)` / `commit_composition(text)` / `cancel_composition`。
- 组字期间：marked text 可读出且等于最近一次 update 的串；匹配结果只由已提交 query 决定（组字串不进入匹配）。
- 提交后：marked 清空，query = 组字前 query + 提交串（插入位置正确），匹配立即按新 query 更新。
- 取消后：marked 清空，query 与组字前逐字符相等，匹配结果不变。
- 失败判据：组字串参与匹配；提交丢字/重复；取消后 query 残留组字串；组字态下 `Enter` 直接聚焦（应先完成组字提交语义，不得把组字确认键当聚焦键）。

### I2 UTF-16 偏移换算（模型，A 档）

- `text_for_range` / `selected_text_range` / `marked_text_range` / `replace_text_in_range` 的 range 按 UTF-16 code unit 解释；含 CJK（BMP）与代理对（如 emoji）的 query 换算往返一致。
- 固定用例：query = `"组件a😀"`（UTF-16 长度 5），逐 range 断言 `text_for_range` 返回与字符边界对齐；`replace_text_in_range` 在代理对中间不得产生半个字符。
- 失败判据：任一 range 换算错位；代理对被截断；`marked_text_range` 与 marked 实际 UTF-16 长度不一致。

### I3 CJK 匹配（模型 + fixture，A 档）

- `tests/fixtures/ime` 上 query=`组件` 命中集合恰为 `{src/组件/按钮.tsx, src/组件/输入框.tsx}`（分母 2，排序按票 20 C3 四级规则）；query=`按钮` 命中恰 1 条。
- 空串仍 0 命中；ASCII query 在该 fixture 上行为与票 20 C2/C3 一致。
- 失败判据：命中集合多一条/少一条；排序违反 C3；空串返回全量。

### I4 窗口级接线（B 档，断言先行；本机人工项）

- 断言清单先于实现落盘（测试文件或提交顺序为证）：`MapView` 实现 `EntityInputHandler` 后，① 组字期间面板 query 区显示 marked 串；② 提交后面板结果列表按新 query 更新；③ 取消后 marked 显示消失。
- 窗口级验证：`PostMessage(WM_CHAR)` 注入 CJK 字符（如 `组`、`件`），stdout 出现对应 `phase:` 标记且 query 更新；注入路径与票 20 的 `PostMessage` 口径一致（`SendKeys` 已证不进 GPUI 键盘链，禁用）。
- 失败判据：注入 CJK 后 query 不变或乱码；面板不重绘；组字显示与提交结果不一致；断言清单晚于实现。

### I5 不回归（自动）

- 票 20 C1–C7、C9、C10 全部判据不变且全绿；`module_search` / `context_fade` / `playback_wiring` / `camera_transition` / `map_layout` 测试全绿；`cargo test --workspace` 全绿；`cargo fmt --all --check` 干净；clippy `-D warnings` 全绿。
- `eval/pixel-evidence.ps1` 24/24 保持（人工项）；搜索三态与聚焦态像素读数不因 IME 接线漂移。
- 失败判据：票 20 任一条变红；像素证据任一状态失败；为过测试修改票 20 判据或匹配规则。

### I6 真机 IME 协议（人工项，登记 acceptance-gaps）

- 环境：Windows + 微软拼音；固定样本 `tests/fixtures/ime`；release 构建。
- 步骤与期望：① `/` 打开面板；② 拼音输入 `zujian` 组字期间面板显示组字串；③ 选词提交 `组件` 后结果列表恰 2 条；④ 再次组字后按 Esc 取消，marked 消失、query 不变；⑤ `Enter` 聚焦、再 `Esc` 清除，行为与票 20 C1 一致。
- 证据：逐步截图 + stdout `phase:` 标记归档 `docs/evidence/`；记录输入法版本与日期。
- 失败判据：任一步骤行为不符；组字串不显示；提交后命中数 ≠2；取消残留。
- 边界：真机 IME 不可自动化（需要真实输入法组字），登记为人工项；未执行前本票不得标 done。

## 实现顺序（按层提交）

1. **模型 + Red**：`crates/arch-view/src/search.rs` 组字子状态与 UTF-16 换算 + `crates/arch-view/tests/ime_input.rs`（先真实 Red，A 档）。
2. **fixture**：`tests/fixtures/ime`（独立提交，不与实现混）。
3. **接线**：`main.rs` `MapView` 实现 `EntityInputHandler`、marked 显示、与 `handle_key` 汇流（B 档断言清单先行）。
4. **证据脚本**：`eval/capture-window.ps1` 扩展 WM_CHAR CJK 注入（`eval/` 改动独立提交；若触及期望值文件，按 `eval:` 前缀写明旧/新读数）。
5. **文档**：README 已知限制更新（IME 支持范围 Windows）、`docs/acceptance-gaps.md` 登记 I4/I6 人工项、票务 README 状态更新。

## 未验证的

- macOS / Linux IME：平台路径未核实，不在本票范围，不宣称支持。
- 真机 IME 组字（I6）：未执行前保持未验证。
- lavapipe 侧输入：无注入路径，沿用票 20 的未测登记。
- 组字期间帧时间：本票不设帧预算判据（属票 23 范围）；若组字触发每键整帧重绘导致卡顿，登记读数移交票 23，不在本票调阈值。