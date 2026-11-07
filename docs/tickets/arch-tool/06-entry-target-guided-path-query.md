# 06: 入口候选、目标选择与最短路径查询

**What to build:** `arch-cli path` 能列出目标仓库的候选入口，接收入口与目标符号，输出一条从入口到目标的有序「模块 → 符号」路径；不可达时给出专用退出码与原因；同一输入输出确定。

**Blocked by:** 02（模块解析）、03（层推断）、04（符号与调用图）

**Status:** ready-for-agent

**固定样本：** `privatenumber/tsx` @ `4473971f4b3261de253209b148d8fdc43269fcc4`（`package.json` 有 `bin` 与复杂 `exports`）与 `excalidraw/excalidraw` @ `701b901a0f1ba3d0b390d9cdf63d112e25abac4b`；路径起点/终点 ID 与期望长度在 golden 中冻结（首次人工确认）。

- [ ] `cargo run -p arch-cli -- path --repo <repo> --entries` 退出码 `0`，输出候选入口列表；tsx 至少包含来自 `package.json` `bin`/`exports` 的候选，且每个候选都映射到已存在模块；零候选时退出码 `5` 并在 stderr 说明原因。
- [ ] `cargo run -p arch-cli -- path --repo <repo> --from <entry-id> --to <symbol-id>` 退出码 `0`，输出有序步骤；每步含模块 ID、符号 ID、层与进入该步的边种类。
- [ ] 路径合法性断言（core 测试）：任意相邻两步之间在图中存在对应边；起点等于给定入口、终点等于给定目标；不含重复步骤。
- [ ] 不可达：给定一个孤立 fixture 中不连通的符号，退出码 `4`，stdout 路径为空，stderr 给出原因；不得回退到「随便给一条路径」。
- [ ] 确定性：同一 `--from/--to` 连续运行两次输出字节一致；多条等长路径时选择规则固定（golden 断言路径节点序列）。
- [ ] 两个样本仓各至少一条 golden 路径通过；路径长度与步骤序列冻结，变更需说明原因。
- [ ] 失败判据：相邻步骤无边、端点不符、不可达未返回 `4`、两次运行不同、golden 漂移。
- [ ] 自动化状态：全部自动化（CLI + core 断言）。
