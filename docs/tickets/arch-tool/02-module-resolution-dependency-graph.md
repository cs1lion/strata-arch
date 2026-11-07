# 02: 模块解析与依赖图

**What to build:** `arch-cli index` 对真实 TS/JS 仓库产出模块级依赖图：相对导入、`tsconfig` 路径别名、workspace 包、外部 `node_modules` 包级节点都被正确归类；无法解析的导入显式进入 `unresolved` 并带原因类别。用户在小型 CLI 与中型 React 仓库上都能得到可信的边集合。

**Blocked by:** 01（CLI 与产物契约）

**Status:** ready-for-agent

**固定样本：**
- `privatenumber/tsx` @ `4473971f4b3261de253209b148d8fdc43269fcc4`（小型 CLI，相对导入为主）
- `excalidraw/excalidraw` @ `701b901a0f1ba3d0b390d9cdf63d112e25abac4b`（workspace + `@excalidraw/*` 路径别名 + JSX）
- 以上组合依据 R2-8 讨论稿倾向，**未逐项确认**。

- [ ] `cargo test -p arch-core --test resolution_golden` 在 tsx 样本上通过：所有内部相对导入（`.ts/.tsx/.js/.jsx/.mjs/.cjs`）解析成功，`unresolved` 中不存在原因类别 `relative-missing`。
- [ ] excalidraw 样本上，前缀 `@excalidraw/` 的导入 100% 解析到 workspace 包模块，`unresolved` 中不存在原因类别 `alias-missing`；`tsconfig` `paths` 未命中项必须落入 `unresolved` 而非丢弃。
- [ ] 外部依赖不展开：`node_modules` 内文件不出现在模块集合中，外部包以包级节点出现；包级节点数等于 golden 基线（首次运行生成，人工抽检 ≥10 条边后签字冻结）。
- [ ] 每个 `unresolved` 项含 `file`、`specifier`、`reason`（枚举之一：`external-package`、`dynamic-import`、`unsupported-extension`、`alias-missing`、`relative-missing`、`syntax-error`），且 UI/JSON 可见。
- [ ] 动态 `import()` 与 `require()` 能解析时产出边；不能解析时入 `unresolved` 且不猜测目标。
- [ ] 确定性：样本仓连续两次索引，`edges` 与 `unresolved` 排序后完全一致。
- [ ] 失败判据：任一内部相对导入落入 `unresolved`、别名未解析、外部包被展开成文件级节点、或 golden 漂移未说明原因。
- [ ] 自动化状态：全部自动化（golden + 结构断言；基线需一次人工抽检）。
- [ ] 评估接入（最小可用版本）：`cargo test -p arch-core --test resolution_golden` 覆盖产物不变量（ID 唯一、模块/边/未解析有序、边两端存在、外部包不展开）与确定性（连续两次序列化字节一致），并锁定第 8 节样本读数；完整标注集、合成极端用例、独立 eval 脚本与门禁分层见 [eval-harness.md](../../process/eval-harness.md)。

**备注：** 首次生成 golden 后需记录两仓的 `edges`、`modules`、`unresolved` 计数作为期望读数；在此之前 CI 只跑不变量断言（相对导入零未解析、别名零未命中）。
