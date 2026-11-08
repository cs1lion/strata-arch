# N1: 架构漂移检测

**状态：** 已决策并实现（2026-11-08，用户拍板 B+C；实现见文末「决策与实现」）。同日按用户裁决升级到 schema v2（见文末「判据调整」）。原文的「待用户拍板」段与验收判据保留作历史对照，不删除。

## 问题

当前工具只回答「现在是什么样」：03/03b 给出分层快照，08 给出每个模块的历史指标，09 给出逐提交的时间线。缺的是「和某个已知良好状态相比，结构哪里变坏了」：

- 新增了低层到高层的反向依赖（例如 `utils` 直接 import `components`）。
- 模块层归属发生迁移（`commands` → `components`），但调用关系没变。
- 新增循环依赖，或把外部包从 `external` 漏进 `components`。
- `unclassified` 占比突然上升，说明规则覆盖退化。

这些变化在 diff review 里很难靠肉眼发现，且不会被现有测试拦住。

## 方案要点

- 基线：显式指定 `--base <commit|tag>` 的 `index + layers` 产物（或直接复用 09 已采样的前缀 `HistoryArtifact`），不做隐式「和上次运行比」。
- 比较维度：层分布（每层模块数）、被方向规则标记的边、循环依赖集合、`unclassified` 原因码分布、外部包泄漏。
- 输出 `drift.json`：`added` / `removed` / `moved` / `newViolations` / `resolvedViolations`，每条带模块 ID、旧层、新层、证据位置；稳定排序。
- 重命名必须用 `git diff -M` 归并，避免把 rename 误报成 remove+add。
- CLI：`arch-cli drift <repo> --base <commit> --out drift.json`；默认只报告，不阻断（阈值与阻断策略由调用方决定）。
- UI：在 09 时间线上把发生漂移的刻度标红，点击展示该刻度的漂移明细；不做自动修复。
- 判据可解释：漂移定义是「层归属变化 / 反向边新增 / 循环新增 / external 泄漏」，不引入评分或置信度。

## 基线粒度选项（2026-11-08）

前提读数：当前产物实测大小（`arch-cli index` / `layers`，pretty JSON）——tsx `index` 72KB；excalidraw `index` 892KB、`layers` 1.1MB（802 模块 / 4363 边）。以下「产物大小」均以这一量级为参照。

### 选项 A：整仓基线（单一快照整体 diff）

- 做法：`--base <commit|artifact>` 与 HEAD 各取一份完整 `index + layers`，整体比较模块集合、层归属、边集合、unclassified 原因码。
- 抓到什么：任何层归属变化、任何新增/消失的边（含反向边）、新增循环、external 泄漏、unclassified 占比变化；是信息最全的一档，能精确到单条边。
- 误报什么：把合法重构当漂移（目录整体搬迁会产出大量 `moved`，新增功能会产出大量 `added` 边）；rename 若未用 `-M` 归并会同时报 remove+add；活跃仓库几乎每次运行都有输出，信噪比低。
- 产物大小量级：与基线产物同量级，约 0.1–1MB（excalidraw 级仓库接近 `index + layers` 的 1–2MB）。
- 实现复杂度：中高。当前 `index` 只读工作区，不读历史 tree；要索引基线 commit 需新增「从 git 对象读文件」或临时 `git archive` 路径，超出 08 已有的 `--numstat` 能力。

### 选项 B：按层基线（层聚合指纹）

- 做法：基线只存每层模块数、每层反向边数、层间边矩阵、`unclassified` 原因码计数、external 泄漏计数；不存逐模块身份。
- 抓到什么：层人口结构突变（如 `components` 计数跳变）、反向依赖集中出现、external 漏进业务层、`unclassified` 比例异常——适合做门禁断言。
- 误报什么：合法新增/删除模块直接改变计数；无法区分「一个模块改层」与「两个模块新增/删除」；无法定位到具体模块，只能给层级信号。
- 产物大小量级：O(层数)，单层一条聚合记录，约几 KB（固定 8 层 + 边矩阵）。
- 实现复杂度：低。可从现有 `layers` 产物直接聚合，不需要逐模块 diff；基线可以是仓库内提交的显式基线文件，从而完全避开历史 tree 索引。

### 选项 C：按模块基线（逐模块指纹）

- 做法：基线存 `模块 ID → 层 + 入/出边种类计数 + 违规标记`；比较输出 `added` / `removed` / `moved` / `newViolations` / `resolvedViolations`。
- 抓到什么：单模块改层、单模块新增反向边、包含某模块的新循环、external 泄漏到具体模块；可直接生成 review 清单。
- 误报什么：新增模块永远报 `added`（无法区分计划内/外）；内部重构改变边计数会被当漂移；rename 需 `git diff -M` 归并，否则 remove+add 噪音；同层内小改动也可能触发边计数变化。
- 产物大小量级：O(模块数)，excalidraw 802 模块约 0.1–0.3MB（按单模块 200–400 字节估算），5k 文件仓约 0.5–1MB。
- 实现复杂度：中。需要模块级 set diff、rename 归并、边种类计数；若基线为仓库内显式产物，则同样无需历史 tree 索引。

### 推荐：B + C 组合（层聚合做门禁，模块明细做解释）

- 门禁层（B）：只比较**违规与一致性指标**（新增反向边数、新增循环数、external 泄漏数、`unclassified` 原因码变化），不比较原始模块计数；默认全为 0 才通过，避免「新增模块=漂移」的系统性误报。
- 明细层（C）：输出模块级 `added` / `removed` / `moved` / `newViolations` 供人看，不参与门禁判定，可按模块 ID 稳定排序。
- 推荐理由：
  1. 只有 B 能当门禁——它天然不受「合法新增」影响；只有 C 能解释「哪里变了」——它给出模块 ID 与证据。两者合起来正好覆盖「阻断 + 定位」。
  2. 基线做成仓库内显式提交的产物（`drift-baseline.json`），避免为基线 commit 重新索引历史 tree；这是 A 无法回避的实现成本。
  3. 与提案原判据一致：不引入评分/置信度，漂移只由四类可解释事件定义。
- 与 A 的取舍：A 信息最全但信噪比与实现成本都最高，适合作为后续「按需全量 diff」的可选模式，而不是默认门禁。

**待用户拍板：** 是否采用 B+C 组合、门禁默认只挡「四类违规新增」、基线以仓库内显式文件提交。未拍板前不实现。

（2026-11-08 已拍板：采用 B+C，门禁只挡四类违规新增，基线以仓库内显式 `drift-baseline.json` 提交。本段保留作历史记录。）

## 可执行验收判据

1. fixture 仓库分别构造三类漂移（新增反向边、模块改层、新增循环），`drift.json` 逐条精确列出，顺序稳定（两次运行字节一致）。
2. 无漂移时 `newViolations` 为空数组，退出码 0。
3. 只重命名（内容不变）时 `added`/`removed` 为空，不产生噪音。
4. `--base` 缺失或 commit 不存在：退出码非 0，stderr 说明原因，不输出半成品 JSON。
5. 基线提交与当前提交的模块 ID 集合差异必须与 `git diff -M` 的 rename 结果一致（抽样断言）。
6. 反向用例：基线本身已有违规时，`resolvedViolations` 与 `newViolations` 必须分开计数，不能合并。

## 成本区间

中等：需要复用 03/03b 的层推断、08 的提交树 diff、09 的前缀产物；新增比较层与 JSON schema。粗估 M（数天量级），不涉及 UI 大改。

## 与现有 ticket 的关系

- 依赖 03/03b（层集合与原因码）、08（历史采样）、09（时间线状态）。
- 与 06 无直接依赖；与 11 的截图/门禁无关。
- 可独立成票，排在 03/08/09 稳定之后。

## 为什么现在做 / 以后做

现在不做：03/03b 的层集合与原因码刚稳定，08/09 的产物契约还在收尾；漂移检测必须先有可复现的基线产物格式，否则会重复造一套比较逻辑。以后做：等 09 产物格式与 10 的性能口径固定后，用 `--base` 显式基线起步，先 CLI 后 UI。

## 决策与实现（2026-11-08）

> **历史记录（v1）**：以下内容记录首次实现时的口径。用户随后裁定 schema v2 调整（见文末「判据调整」）：`populationTolerance` 与按层计数已移除。本段保留作对照，不静默改写。

### 落地形态

- **门禁（B）**：只比较四类「新增违规」。容差 `0` 显式记录在基线 `populationTolerance` 里，而不是隐含在代码里。
- **解释（C）**：报告输出 `added` / `removed` / `moved` / `newViolations` / `resolvedViolations`，每条违规带模块 ID 或层名。
- **基线**：`tests/fixtures/drift/drift-baseline.json`，仓库内显式提交；不读历史 tree，不提供 `--base <commit>`。
- **schema**：`DRIFT_SCHEMA_VERSION = 1`，独立于 index / layers / callgraph / path / history，不复用任何现有版本号。
- **CLI**：`arch-cli drift <repo> --record --out <基线>`；`arch-cli drift <repo> --base <基线> [--out <报告>]`。

### 四类判据边界

| 判据 | 会触发 | 不会触发 | 去重 / 解释 |
| --- | --- | --- | --- |
| 层人口突变 | 任意一层模块计数 `current - baseline` 的绝对值 > `populationTolerance`（0） | 只新增边、不新增模块 | 违规带层名、旧计数、新计数、delta；不做 resolved（基线记录的是目标计数而非违规集合） |
| 新增反向边 | 当前 `directionAdjusted == true` 的 `(source,target,kind)` 不在基线 | 合法正向边（高层 → 低层） | 违规带两端所在层；基线里已有的反向边不重复计数，消失的进 `resolvedViolations` |
| 新增循环 | 模块级 SCC（成员 ≥2 或自环）的成员集合不在基线 | 已存在的循环；同层单向边 | 一个 SCC 只报一条，不按环上的边重复计数 |
| external 泄漏 | `kind == external-package` 的模块落在 `external` 之外的层 | 外部包正常落在 `external`；业务层新增对 external 的边 | 违规带模块与「被污染层」；基线记录集合，新增与已解决分开 |

**external 泄漏的事实边界（实测）**：`arch-cli layers tests/fixtures/layers` 中所有 `external-package` 模块都在 `external` 层（1/1）。分类规则对 `external-package` 无条件返回 `external`，因此真实仓库当前不可能触发这条判据。保留它作为「规则退化导致外部包漏进业务层」的回归不变量；正向基线恒为空，触发路径由合成 `LayersArtifact` 覆盖（测试交换 react / Button 的层归属并保持层计数不变）。

### 原验收判据逐条对照（原文保留，不删除）

| 原判据 | 现状 | 说明 |
| --- | --- | --- |
| 1 三类漂移逐条列出、顺序稳定 | 支持 | 现为四类；`newViolations` 按判据顺序 + 字典序稳定输出，两次运行字节一致 |
| 2 无漂移时 `newViolations` 为空、退出码 0 | 支持 | fixture 基线自检 exit 0，四类 new/resolved 全 0 |
| 3 只重命名时 `added`/`removed` 为空 | **推翻** | 用户最新要求：rename 必须显式报 added/removed，不得静默归并为「无变化」。原因：显式基线无历史 tree，`git diff -M` 不可用；且静默归并会隐藏身份变化 |
| 4 `--base` 缺失或 commit 不存在非 0、stderr 说明、无半成品 JSON | 支持（改为文件基线） | 基线文件缺失 / 非法 / schema 不匹配 → exit 2，stdout 为空 |
| 5 模块 ID 差异与 `git diff -M` rename 一致 | **推翻** | 同上：不做历史 tree 索引，也不做 rename 归并；rename 以 removed + added 呈现 |
| 6 `resolvedViolations` 与 `newViolations` 分开计数 | 支持 | 反向边 / 循环 / external 泄漏三类支持；人口突变无 resolved 概念，已在文档写明 |

原判据 3 与 5 的推翻理由同时登记在 NIGHTLOG 与 `crates/arch-core/src/drift.rs` 模块文档；未静默删除原判据。

### 实测读数（2026-11-08）

- 正向基线：fixture 8 模块（cli 1 / commands 1 / components 1 / server 1 / utils 2 / external 1 / unclassified 1），reverseEdges 0、cycles 0、externalLeaks 0；自检 exit 0，`added`/`removed`/`moved` 全 0。
- 反向用例：合法正向边 `cli → utils` 不报；`utils → commands` 反向边命中 1 条；`utils` 层内 `format ↔ text` 只报 1 个 SCC；同层 rename 门禁通过但 added/removed 显式（v2 起改为门禁失败，见下节）；违规消失时 resolved 1 / new 0。
- 自动化：`cargo test -p arch-core --test drift` 10 passed / 0 failed；`cargo test -p arch-cli --test drift` 5 passed / 0 failed；`cargo test --workspace` 全绿；`cargo fmt --all --check` 通过。

### 基线重录纪律

`drift-baseline.json` 的任何更新必须独立提交，提交信息写明旧读数、新读数与变更原因；不得与功能或测试提交混在一起。层人口容差为 0 时，任何合法新增模块也会要求重录基线——这是有意的流程约束，而不是误报。

## 判据调整（2026-11-08，schema v2）

### 用户裁决

1. **去掉 `populationTolerance` 数字阈值**，改为「新增/删除模块必须人工确认层归属」。
2. **external 泄漏判据维持现状**（回归不变量 + 合成产物验证）；**不**重新定义为「外部依赖面扩张」。

### 变更内容

- `DRIFT_SCHEMA_VERSION` 1 → 2；基线删除 `populationTolerance` 与按层计数 `layers`（计数可由 `modules` 推导，且不再参与门禁）。
- 违规类型 `layer-population` → `module-change`（JSON `kind: "moduleChange"`）：
  - 按层分组，字段 `layer` / `added` / `removed`；
  - 新增模块用当前层，删除模块用基线层；层与模块 ID 都稳定排序；
  - 没有 `resolved`：新增和删除都必须人工确认，任何 ID 集合差异都要求重录基线。
- 同 ID 改层不进入 `moduleChange`，只在 `modules.moved` 解释。原提案提到的「模块改层」属于解释层，不参与门禁（v1 与 v2 都是这样）。
- rename 现在会同时出现在 `added` 与 `removed`，并且**门禁变红**——这是「必须人工确认」的直接结果。

### 为什么移除数字阈值（保留原判据与推翻理由）

- 计数无法区分「一个模块改层」与「两个模块增删」，只能给层信号。
- 阈值可被事后调整来凑绿，违反「不看读数回改判据」红线。
- 模块身份是更可解释的判据：报告能直接说「哪一层的哪个模块是新增/删除」，确认动作就是重录基线。
- 原 v1 判据「任意一层计数变化 > 0 即漂移」保留在本文件上文，未被删除；它的缺陷正是触发条件只与计数有关。

### external 泄漏维持现状

- 不重新定义为「外部依赖面扩张」：该口径没有精确定义（新增合法依赖与泄漏无法区分），硬造等于凑数。
- 仍为回归不变量：`external-package` 必须落在 `external`；正向恒空，触发路径由合成 `LayersArtifact` 验证。

### schema v2 的 Red / Green（实测）

- Red：core `E0609 no field module_changes` / `E0599 no variant ModuleChange`；CLI 2 passed / 4 failed（schemaVersion 1≠2、`layerPopulation`、缺 `moduleChanges`、旧 v1 基线未被拒绝）。
- Green：core 13 passed / 0 failed；CLI 6 passed / 0 failed。
- 反向用例变化：同层 rename 由「门禁通过」变为「门禁失败 + 按层列出 added/removed」；其余 (a)(b)(c) 不变。

### 真实基线实证（2026-11-08）

- tsx（`tests/corpus/tsx`，pin `4473971`）：136 模块；`--record` 107ms → `eval/baseline/drift-tsx.json`
  11,178 字节；基线自检 exit 0，四类 new/resolved 全 0，added/removed/moved 全 0。
- excalidraw（`tests/corpus/excalidraw`，pin `701b901`）：802 模块；`--record` 905ms →
  `eval/baseline/drift-excalidraw.json` 108,905 字节；自检 exit 0，四类全 0。
- 比较耗时（`target/debug`，含 index + layers + 比较）：tsx 103ms；excalidraw 879ms。
- fixture 基线 v1 → v2 重录：1,197 → 702 字节（`populationTolerance` 与 `layers` 计数移除）。
- 新增文件门禁（tsx 真实基线 + 临时副本新增 `src/utils/drift-probe.ts`）：exit 1，实输出
  `{"kind":"moduleChange","layer":"utils","added":["src/utils/drift-probe.ts"],"removed":[]}`，
  同时 `modules.added` 列出同一 ID——报告能定位到层与模块，而不是只给计数。临时副本已删除。
- 真实基线与 fixture 基线均单独提交；提交信息写明旧读数、新读数与原因。
