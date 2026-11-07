# 03: 层推断与分层地图静态渲染

**What to build:** 用户运行 `arch-cli layers` 能看到每个模块的层归属与 `unclassified` 列表；桌面端用分层地图静态渲染出模块与层带，节点布局稳定、可由布局模型做结构断言。用户第一眼能回答「系统由什么组成」。

> **验收调整（2026-11-07，用户确认走 D）**：截图验收已从本票移除，移交 ticket 11 重新定义；当前未验证，不得记为通过。本票只保留布局/结构断言。原 `render_to_image`「方法存在」的断言已被源码检索推翻（gpui 0.2.2 无公开离屏导出入口），详见 `docs/acceptance-gaps.md` 第 2.2 节。

> **03b 增补（2026-11-07，用户决定先提覆盖率再一起验收）**：层集合增加独立 `external` 层，并扩展目录/文件名语义与原因码，详见 `03b-layer-coverage.md`。本票的 `unclassified` 读数与层集合以 03b 完成后的产物为准；两票一起验收，当前均未提交。

**Blocked by:** 02（模块解析与依赖图）

**Status:** ready-for-agent

**固定样本：** 02 的两个样本仓（同 commit）；另有 `tests/fixtures/layers` 小型 fixture，含已知层目录（`cli/`、`commands/`、`utils/`）与一个无法归类的 `misc/`。

- [ ] `cargo run -p arch-cli -- layers <repo> --out layers.json` 对两个样本仓退出码 `0`；每个模块都有 `layer` 字段，值来自固定层集合（含 `external`）或 `unclassified`。
- [ ] 小型 fixture 上：`cli`、`commands`、`utils` 下模块归属正确；`misc` 下模块全部进入 `unclassified`；`unclassified` 列表在 JSON 与 UI 中同时可见（UI 有独立视觉标记）。
- [ ] 依赖方向校正可追溯：每条被方向规则降低置信度的边带标记，原边不被静默删除。
- [ ] 分层地图渲染入口 `cargo run -p arch-view -- map <repo>`（无截图参数）退出码 `0`；布局模型中的节点数等于 JSON 中的模块数，层带与节点矩形由 `map_layout` 测试断言。
- [ ] `cargo test -p arch-view --test map_layout` 通过：层带按固定顺序自上而下排列；同层节点包围盒重叠面积总和低于基线阈值；`unclassified` 有独立层带。
- [ ] 截图结构断言：**已移交 ticket 11，当前未验证**（不适用本票验收，不得记为通过）；像素差门禁随截图能力一并待定。
- [ ] 失败判据：任何模块缺少 `layer`、`unclassified` 被静默吞掉、布局重叠超阈值、布局模型节点数与 JSON 不符。
- [ ] 自动化状态：CLI 与布局断言自动化；截图与像素差门禁不在本票范围（移交 ticket 11，当前未验证）。
