# 04: 符号提取与调用图

**What to build:** 用户能在结构产物与后续视图中看到符号级节点（函数、组件、类、方法）以及调用边，包括跨模块的 JSX 组件使用边；动态调用与无法归因的调用显式标记。用户能区分「引用了模块」与「调用了其中某个符号」。

**Blocked by:** 02（模块解析与依赖图）

**Status:** ready-for-agent

**固定样本：** 02 的两个样本仓（同 commit）；另有 `tests/fixtures/callgraph`，含：同文件调用、跨模块调用、JSX 组件使用、`import()` 动态调用、无法归因的成员调用。

- [ ] `cargo test -p arch-core --test callgraph_golden` 通过：小型 fixture 上指定的跨模块调用链存在，JSX 组件边存在，动态 `import()` 边与 `unresolved` 记录同时存在。
- [ ] 符号节点含稳定 ID、种类（`function`/`component`/`class`/`method`/`module-toplevel`）、所属模块与位置；同一输入两次提取 ID 完全一致。
- [ ] 调用边两端必须指向已存在符号；`cargo test` 中断言不存在悬空边（源或目标缺失即为失败）。
- [ ] 模块级聚合边由符号级边收敛：每个聚合边可解释为「至少一条符号级边的投影」，去重规则固定；聚合前后调用对数差异符合基线。
- [ ] 重载、默认导出、匿名函数、同名不同文件的 ID 规则在实现首票冻结并写入测试；规则变更必须更新 golden 并说明原因。
- [ ] 无法归因的调用进入 `unresolved` 的 `call-unattributed` 类别，不得丢弃或强行归到模块顶层。
- [ ] 失败判据：悬空边、ID 不稳定、JSX 边缺失、动态调用被静默忽略、golden 漂移未说明。
- [ ] 自动化状态：全部自动化（golden + 不变量断言；基线首次人工抽检 ≥10 条调用边）。
