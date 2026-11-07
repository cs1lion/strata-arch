# 04: 符号提取与调用图

**What to build:** 用户能在结构产物与后续视图中看到符号级节点（函数、组件、类、方法）以及调用边，包括跨模块的 JSX 组件使用边；动态调用与无法归因的调用显式标记。用户能区分「引用了模块」与「调用了其中某个符号」。

**Blocked by:** 02（模块解析与依赖图）

**Status:** done（04 + 04b + 04c）

**固定样本：** 02 的两个样本仓（同 commit）；另有 `tests/fixtures/callgraph`，含：同文件调用、跨模块调用、JSX 组件使用、`import()` 动态调用、无法归因的成员调用、外部导入的成员/裸调用、CommonJS `require()` 绑定，以及不参与扫描的 `.d.ts`。

- [x] `cargo test -p arch-core --test callgraph_golden` 通过：小型 fixture 上指定的跨模块调用链存在，JSX 组件边存在，动态 `import()` 边与 `unresolved` 记录同时存在。
- [x] 符号节点含稳定 ID、种类（`function`/`component`/`class`/`method`/`module-toplevel`）、所属模块与位置；同一输入两次提取 ID 完全一致。
- [x] 调用边两端必须指向已存在符号；`cargo test` 中断言不存在悬空边（源或目标缺失即为失败）。
- [x] 模块级聚合边由符号级边收敛：每个聚合边可解释为「至少一条符号级边的投影」，去重规则固定；聚合前后调用对数差异符合基线。
- [x] 重载、默认导出、匿名函数、同名不同文件的 ID 规则在实现首票冻结并写入测试；规则变更必须更新 golden 并说明原因。
- [x] 无法归因的调用进入 `unresolved` 的 `call-unattributed` 类别，不得丢弃或强行归到模块顶层（04c 起默认只保留聚合计数，`--include-unattributed` 输出明细）。
- [x] 失败判据：悬空边、ID 不稳定、JSX 边缺失、动态调用被静默忽略、golden 漂移未说明。
- [x] 自动化状态：全部自动化（golden + 不变量断言；基线首次人工抽检 ≥10 条调用边）。

## 04b / 04c 增量（2026-11-07）

### 04b 降噪

- `stats` 由单一 `unresolved` 拆为 `unattributedCalls` / `unresolvedImports`，并增加 `externalCalls`，保证统计与条目一一对应。
- `.d.ts` / `.d.mts` / `.d.cts` 不参与调用扫描（文件数、符号、未解析项均不计入）。
- 已知外部导入的**成员调用**记 `external-call`。
- 读数（tsx / excalidraw）：`unattributedCalls` 3744→3648 / 59314→58852；条目总数 3747→3746 / 59315→59308；下降分别来自 95 / 455 条外部成员调用改判 + 1 / 7 条来自 `.d.ts`。

### 04c 降噪与呈现

- 外部导入的**裸调用**（`clsx(...)`、`new CLIEngine(...)`）同样记 `external-call`。
- CommonJS `const ns = require("pkg")` 与 `const { named } = require("pkg")` 的接收者纳入导入绑定，与 ES import 同等对待（本地/外部/无法解析三分类一致）。
- `call-unattributed` 默认只保留聚合计数，不进入 `unresolved` 明细、不进入边列表/图；CLI `callgraph --include-unattributed` 输出完整明细。
- 契约：call graph 产物独立使用 `CALLGRAPH_SCHEMA_VERSION`，04c 起为 **2**（v1 = 04 初版；v2 = 拆分统计 + `external-call` + 默认聚合）。`index` / `layers` 的 `SCHEMA_VERSION` 不变。
- 读数：tsx `unattributedCalls` 3648→2460、`externalCalls` 95→1283；excalidraw 58852→57732、455→1575；`unresolvedImports` 与边总数不变（3 / 624、1 / 7388）。默认 JSON 体积 tsx 845,995→426,264 字节，excalidraw 14,054,570→3,254,564 字节。
- 边界（不做的）：不做类型推断；不猜测局部变量/参数的成员调用归属；对象字面量方法误判为调用不在本次范围（见 backlog）。

### Backlog（不在本票范围）

- 对象字面量方法定义（`{ onClick() {} }`）被当作调用记入 `call-unattributed`。
- CommonJS `require.resolve(...)` 等 `require` 自身的成员调用未纳入外部绑定。
- 接口/类型别名中的方法签名（非 `.d.ts` 文件）仍可能被当作调用。

