# 03: 层推断与分层地图静态渲染

**What to build:** 用户运行 `arch-cli layers` 能看到每个模块的层归属与 `unclassified` 列表；桌面端用分层地图静态渲染出模块与层带，节点布局稳定、可由布局模型做结构断言。用户第一眼能回答「系统由什么组成」。

> **验收调整（2026-11-07，用户确认走 D）**：截图验收已从本票移除，移交 ticket 11 重新定义；当前未验证，不得记为通过。本票只保留布局/结构断言。原 `render_to_image`「方法存在」的断言已被源码检索推翻（gpui 0.2.2 无公开离屏导出入口），详见 `docs/acceptance-gaps.md` 第 2.2 节。

> **03b 增补（2026-11-07，用户决定先提覆盖率再一起验收）**：层集合增加独立 `external` 层，并扩展目录/文件名语义与原因码，详见 `03b-layer-coverage.md`。本票的 `unclassified` 读数与层集合以 03b 完成后的产物为准。两票实现已提交：`26235e5`（core 层推断）、`fb071a0`（布局与 GPUI 渲染）、`29f8e27`（测试与反向用例）；验收记录见文末。

**Blocked by:** 02（模块解析与依赖图）

**Status:** done（布局/CLI 断言全绿；`external` 视觉与截图未验证）

**固定样本：** 02 的两个样本仓（同 commit）；另有 `tests/fixtures/layers` 小型 fixture，含已知层目录（`cli/`、`commands/`、`utils/`）与一个无法归类的 `misc/`。

- [x] `cargo run -p arch-cli -- layers <repo> --out layers.json` 对两个样本仓退出码 `0`；每个模块都有 `layer` 字段，值来自固定层集合（含 `external`）或 `unclassified`。
- [x] 小型 fixture 上：`cli`、`commands`、`utils` 下模块归属正确；`misc` 下模块全部进入 `unclassified`；`unclassified` 列表在 JSON 与 UI 中同时可见（UI 有独立视觉标记）。
- [x] 依赖方向校正可追溯：每条被方向规则降低置信度的边带标记，原边不被静默删除。
- [x] 分层地图渲染入口 `cargo run -p arch-view -- map <repo>`（无截图参数）退出码 `0`；布局模型中的节点数等于 JSON 中的模块数，层带与节点矩形由 `map_layout` 测试断言。
- [x] `cargo test -p arch-view --test map_layout` 通过：层带按固定顺序自上而下排列；同层节点包围盒重叠面积总和低于基线阈值；`unclassified` 有独立层带。
- [x] 截图结构断言：**已移交 ticket 11，当前未验证**（不适用本票验收，不得记为通过）；像素差门禁随截图能力一并待定。
- [x] 失败判据：任何模块缺少 `layer`、`unclassified` 被静默吞掉、布局重叠超阈值、布局模型节点数与 JSON 不符。
- [x] 自动化状态：CLI 与布局断言自动化；截图与像素差门禁不在本票范围（移交 ticket 11，当前未验证）。

## 验收记录（2026-11-08）

### 读数（实测，`target/debug/arch-cli.exe` / `arch-view.exe`）

| 样本 | 模块数 | external | unclassified | 层分布 | 边数 | directionAdjusted |
| --- | --- | --- | --- | --- | --- | --- |
| `tests/fixtures/layers` | 26 | 1 | 6（ambiguous 2 / no-rule-match 4） | 8 带：cli 2、commands 1、components 7、server 1、utils 4、tests 4、external 1、unclassified 6 | 19 | 0 |
| tsx @ `4473971` | 136 | 41 | 44 | external 41、tests 19、utils 32、unclassified 44 | 379 | 0 |
| excalidraw @ `701b901` | 802 | 84 | 198 | components 285、tests 162、external 84、commands 44、utils 29、unclassified 198 | 4363 | 0 |

- 两个样本仓 `layers` 退出码均 `0`，每个模块都有 `layer`，取值来自固定层集合或 `unclassified`。
- fixture `misc/notes.ts`、`misc/plain.ts`、`src/features/random.ts`、`packages/web/src/index.ts` 为 `no-rule-match`；`src/utils/both.test.stories.tsx`、`features/tie-none.test.stories.tsx` 为 `ambiguous`。
- `cargo test -p arch-view --test map_layout`：9 passed / 0 failed。
- `cargo test --workspace`：108 passed / 0 failed；`cargo fmt --all --check` 通过。
- 地图入口 smoke：`arch-view map tests/fixtures/layers --auto-close-ms 300` → exit 0，输出 `26 modules, 19 edges, 8 bands`，墙钟 2126ms（含 300ms 自动关闭等待）。

### 偏差与未验证

- **方向校正端到端 0 触发**：三个固定样本的 `directionAdjusted` 均为 0（没有低层模块反向依赖高层模块的边）。标记逻辑由 core 单元测试 `direction_violations_are_marked_without_ranking_unknown_layers` 覆盖，但没有样本端到端触发；不把「0 条」写成「已验证可追溯」。
- **UI 视觉未验证**：`external` 与 `unclassified` 在渲染代码中有不同层带底色、边框与节点角标，但无截图证据（移交 ticket 11）。
- **截图与像素差**：本票不适用，未验证。
