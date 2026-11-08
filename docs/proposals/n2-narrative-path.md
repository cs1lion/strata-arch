# N2: 叙述式路径解释

**状态：** 已实现（CLI 与路径产物，2026-11-08）；UI 文案复用与备选路径未做。

## 问题

06 的 `path` 已经给出确定性的最短路径（入口 → 若干边 → 目标符号），但输出是一串符号与边类型。第一次读仓库的人仍然要自己回答：

- 这一步为什么存在？是运行时调用、JSX 渲染、动态 import，还是 CommonJS `require`？
- 证据在哪（文件与行号）？这条边是静态可证，还是有条件/动态的不确定性？
- 有没有更短或同层的替代路径？这条路径是否经过测试或构建产物映射？

现在只能靠人工读代码补全，路径结果的「可解释性」没有被产品化。

## 方案要点

- 在 06 的路径产物上增加一层**结构化叙述**，不引入 LLM、不生成自由文本：每个 step 输出固定字段 `{index, from, to, edgeKind, evidence, layerFrom, layerTo, confidence}`。
- 措辞模板固定且可测试：例如 `"components/Button.tsx#fn:render 通过 jsx 渲染到 components/Dialog.tsx#fn:Dialog"`；模板版本号写入产物。
- 不确定性显式：`dynamic-import` / `require` / 测试专用边 / 构建产物映射落入路径时，step 带 `conditional` 或 `mapped` 标记，不做「一定执行」的断言。
- 可选替代：同层路径、次短路径只作为「备选」列出（上限 2 条），不改变主路径。
- CLI：`arch-cli path <repo> --from <入口> --to <符号> --explain`；默认输出 JSON，`--explain` 增加叙述字段或独立 `explanation.json`。
- UI（可选后续）：07 的回放面板按 step 展示同一模板，不做两套文案。

## 实现记录（2026-08 决议对照）

- 用户裁决：叙述内嵌进路径产物，同时提供 `--narrative` 输出独立文本；不引 LLM，用固定模板。
- 落地字段：每步 `evidence`（`{file,line,column}`）、`layerFrom` / `layerTo`、`conditional`、`testOnly`、`narrative`；`PATH_SCHEMA_VERSION` 1 → 2。入口候选产物改用独立 `ENTRY_SCHEMA_VERSION = 1`。
- CLI：`arch-cli path <repo> --from <入口> --to <符号> --narrative`，stdout 或 `--out` 输出文本；`--entries` 与 `--narrative` 冲突按输入错误退出 `2`。
- 与提案差异：
  - 提案的 `--explain` 名称改为用户裁决的 `--narrative`；没有独立 `explanation.json`。
  - 提案的 `confidence` 字段未做；用布尔 `conditional` / `testOnly` 表达不确定性，避免无证据的数值置信度。
  - 提案的备选路径（同层/次短，上限 2 条）未做。
  - 模板版本未单列字段，由产物顶层 `schemaVersion` 承担。
- 验收：`cargo test -p arch-core --test path_narrative` 4 passed；`cargo test -p arch-cli --test path` 13 passed；叙述文本两次运行字节一致；禁用词「一定 / 必然 / 证明」断言不含。

## 可执行验收判据

1. 对 06 的 golden 路径，叙述 step 数 == 路径 step 数；每步都带 `file:line` 证据，字段完整。
2. 两次运行输出字节一致（确定性）；叙述模板版本固定。
3. 无证据的边必须标 `unresolved`，不得生成具体文件名或行号（反向用例：构造一条无位置信息的边）。
4. `dynamic-import` 边必须带 `conditional=true`；测试专用边必须标 `test-only=true`。
5. 不可达路径仍返回退出码 4，且不生成叙述半成品。
6. 禁用词检查：叙述中不得出现「一定」「必然」「证明」等无证据措辞（可做字符串断言）。

## 成本区间

低到中：主要是路径产物的字段扩充与模板实现，不新增解析逻辑；需要先统一各 edge 的证据字段。粗估 S–M。

## 与现有 ticket 的关系

- 直接扩展 06（引导路径）；复用 04 的调用图边证据。
- 与 07 回放共享 step 展示；与 09/N1 无耦合。

## 为什么现在做 / 以后做

现在不做：06 的 `kind`/证据字段还没有完全统一，`require`/动态 import 的语义边界刚定；先保证路径本身正确、确定（06 已完成），再谈叙述。以后做：作为 06 的增量票，先 CLI `--explain`，再考虑 UI 文案复用。
