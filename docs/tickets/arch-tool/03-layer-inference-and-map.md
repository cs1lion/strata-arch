# 03: 层推断与分层地图静态渲染

**What to build:** 用户运行 `arch-cli layers` 能看到每个模块的层归属与 `unclassified` 列表；桌面端用分层地图静态渲染出模块与层带，节点布局稳定、可截图。用户第一眼能回答「系统由什么组成」。

**Blocked by:** 02（模块解析与依赖图）

**Status:** ready-for-agent

**固定样本：** 02 的两个样本仓（同 commit）；另有 `tests/fixtures/layers` 小型 fixture，含已知层目录（`cli/`、`commands/`、`utils/`）与一个无法归类的 `misc/`。

- [ ] `cargo run -p arch-cli -- layers <repo> --out layers.json` 对两个样本仓退出码 `0`；每个模块都有 `layer` 字段，值来自固定层集合或 `unclassified`。
- [ ] 小型 fixture 上：`cli`、`commands`、`utils` 下模块归属正确；`misc` 下模块全部进入 `unclassified`；`unclassified` 列表在 JSON 与 UI 中同时可见（UI 有独立视觉标记）。
- [ ] 依赖方向校正可追溯：每条被方向规则降低置信度的边带标记，原边不被静默删除。
- [ ] 分层地图渲染命令 `cargo run -p arch-view -- map <repo> --screenshot out/map.png` 退出码 `0`，输出图像宽度 ≥800、高度 ≥600；节点数等于 JSON 中的模块数。
- [ ] `cargo test -p arch-view --test map_layout` 通过：层带按固定顺序自上而下排列；同层节点包围盒重叠面积总和低于基线阈值；`unclassified` 有独立层带。
- [ ] 截图结构断言通过（节点/边数、层带顺序、包围盒）；像素差门禁**默认关闭**（推荐值，**待用户确认**），开启时需另附基线图。
- [ ] 失败判据：任何模块缺少 `layer`、`unclassified` 被静默吞掉、布局重叠超阈值、截图尺寸或节点数与 JSON 不符。
- [ ] 自动化状态：CLI 与布局断言自动化；像素差门禁待用户确认后启用；`render_to_image` 在本机/CI 尚未验证（方法存在）。
