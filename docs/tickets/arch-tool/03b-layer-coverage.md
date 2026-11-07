# 03b: 层推断覆盖率（external 独立层 + 语义扩展）

**What to build:** 在不硬塞的前提下提高层推断覆盖率：外部包独立成 `external` 层；目录与文件名语义覆盖常见仓库形态；workspace 子包按包内目录判断；拓扑位置只用于命名冲突断平；每个未归类模块带可追溯原因码。

**Blocked by:** 03（03 未提交，本票完成后与 03 一起验收）

**Status:** ready-for-agent（A 档：先失败测试，再实现）

**不提交、不 push、不改远端 issue。**

## 1. 层集合与外部包

固定层集合扩展为：

```
cli → commands → components → server → utils → tests → external → unclassified
```

- `external` 是独立层，不再落进 `unclassified`；JSON 的 `layer`、`layerOrder` 与 UI 层带都独立。
- `external` 的视觉角色为 `external`，与 `unclassified` 的 `unclassified` 明确区分（不同层带/节点配色）。
- `external` 节点携带原因码 `external`，但**不**出现在 `unclassified` 列表里；`unclassified` 列表只包含 `no-rule-match` 与 `ambiguous`。
- 原因码枚举保留三项 `no-rule-match` / `ambiguous` / `external`，其中 `external` 专用于 `external` 层，保证「所有未归类语义都有原因码」这一约束在 JSON 中可逐条核对。

## 2. 规则与优先级（冲突时谁优先）

自上而下，先命中先返回：

1. **kind 规则**：`kind == external-package` → `external`。
2. **文件名后缀规则**（强于目录，因为后缀是跨目录的显式角色声明）：
   - 文件名包含 `.test.` 或 `.spec.` → `tests`
   - 文件名包含 `.stories.` → `components`
   - 同时命中 test 与 stories → 命名冲突，进入第 4 步拓扑断平；断平失败记 `ambiguous`。
3. **目录规则**：对路径中的所有目录段做大小写不敏感匹配，**最深匹配优先**。
   - `cli`
   - `command` / `commands` / `action` / `actions` → `commands`（命令式用例/动作层）
   - `component` / `components` / `hooks` / `renderer` / `ui` → `components`（React 与渲染层约定）
   - `server` → `server`
   - `util` / `utils` / `lib` / `common` / `shared` → `utils`
   - `test` / `tests` / `__tests__` / `spec` / `specs` → `tests`
   - `app` / `apps` / 以 `-app` 结尾的目录段 → `components`（应用外壳/页面组合）
   - 文件名本身不参与目录匹配；`src/<第一段>` 指 `src` 后第一个**目录**段，例如 `src/components/...`。
   - 目录同义的依据：`actions` 是 action/command 模式下的用例动作；`hooks`/`renderer`/`ui` 是 React 与渲染约定下的表现层；`lib`/`common`/`shared` 是共享工具代码；`app`/`-app` 是应用外壳。它们都按目录名成立，不依赖具体仓库的业务命名。
4. **拓扑断平（仅用于第 2 步的命名冲突）**：统计该模块的直接邻居（入边 + 出边）中已由命名规则归层的模块层，按层计数；仅当某个候选层是唯一最大值且支持数 ≥ 2 时采用该层，规则记为 `topology:<layer>:support=<n>`。否则记 `ambiguous`。
5. **无匹配**：`no-rule-match`。

**冲突优先级结论：** kind > 文件名后缀 > 目录 > 拓扑。拓扑**不参与**从零归层，只在文件名规则自相冲突时断平，避免把无法判断的模块硬塞进某一层。

## 3. workspace 子包

- 目录扫描覆盖完整路径，因此 `packages/<pkg>/src/...` 与 `src/<第一段>/...` 使用同一套目录规则。
- 子包内部目录更深时按最深匹配优先，例如 `packages/web/src/components/Badge.tsx` → `components`；`packages/cli/src/main.ts` 可由包名段 `cli` 命中 → `cli`。
- 只有 `packages/web/src/index.ts` 这类包内无任何语义目录的模块才保持 `unclassified`，不按包名整体兜底。

## 4. 原因码与可追溯字段

每个 `LayeredModule` 增加：

- `rule`: 命中规则标识，如 `kind:external-package`、`file:test-suffix`、`dir:components`、`topology:tests:support=2`；未归类/未断平为 `null`。
- `reason`: 仅 `external` 与 `unclassified` 携带，值为 `external` / `no-rule-match` / `ambiguous`。

`unclassified` 从字符串数组改为对象数组：

```json
{ "id": "misc/notes.ts", "path": "misc/notes.ts", "reason": "no-rule-match" }
```

未分类列表因此可逐条解释。

## 5. 反向用例（防止过度泛化）

fixture 中必须存在且保持 `unclassified` 的样本：

| 路径 | 期望原因码 | 目的 |
| --- | --- | --- |
| `misc/notes.ts` | `no-rule-match` | 无任何层语义，仅有 1 条依赖边，拓扑不得归层 |
| `misc/plain.ts` | `no-rule-match` | 无任何层语义且无依赖边 |
| `src/features/random.ts` | `no-rule-match` | `src/<第一段>` 的第一段不是已知层名，不得因在 `src/` 下就兜底 |
| `packages/web/src/index.ts` | `no-rule-match` | workspace 子包内无语义目录，不得按包整体兜底 |
| `src/utils/both.test.stories.tsx` | `ambiguous` | 文件名同时命中 tests 与 stories，且无拓扑证据 |
| `features/tie-none.test.stories.tsx` | `ambiguous` | 命名冲突且拓扑无支持 |

拓扑断平的正向样本：

| 路径 | 期望层 | 依据 |
| --- | --- | --- |
| `features/tie-tests.test.stories.tsx` | `tests` | 两个 tests 模块导入它，支持数 2 |
| `features/tie-components.test.stories.tsx` | `components` | 两个 components 模块导入它，支持数 2 |

## 6. 验收读数（改前/改后同口径）

口径：`modules` 总数；`external` 计数（`kind == external-package`）；`unclassified` 只统计自有源码（`kind != external-package`）；自有源码占比 = 自有源码 unclassified / 自有源码总数。

改前（03 实现，external 混在 unclassified）：

| 样本 | 模块总数 | external | unclassified（含 external） | 自有源码 unclassified | 自有源码占比 |
| --- | --- | --- | --- | --- | --- |
| tsx | 136 | 41 | 85 | 44 / 95 | 46.3% |
| excalidraw | 802 | 84 | 414 | 330 / 718 | 46.0% |

改后（03b 实现，external 已独立）：

| 样本 | 模块总数 | external 层 | unclassified（自有源码） | 自有源码 unclassified | 自有源码占比 |
| --- | --- | --- | --- | --- | --- |
| tsx | 136 | 41 | 44 | 44 / 95 | 46.32% |
| excalidraw | 802 | 84 | 198 | 198 / 718 | 27.58% |

原因码分布（改后）：

| 样本 | `external` | `no-rule-match` | `ambiguous` |
| --- | --- | --- | --- |
| tsx | 41 | 44 | 0 |
| excalidraw | 84 | 198 | 0 |

规则命中分布（改后）：

| 样本 | `kind:external-package` | `file:test-suffix` | `dir:*` | `topology:*` |
| --- | --- | --- | --- | --- |
| tsx | 41 | 0 | 51 | 0 |
| excalidraw | 84 | 150 | 356 | 0 |

`cargo test --workspace`：54 passed / 0 failed（arch-cli 21（contract 7 / layers 10 / resolution 4）、arch-core 24（unit 22 / golden 2）、arch-view 9、doc-tests 0）；`cargo fmt --check` exit 0。

改后剩余未分类构成（用于解释差距）：

- tsx 44：`src/esm` 12、`src/cjs` 12、`src` 根目录入口/工具文件 8、`src/@types` 2、`src/watch` 2、`src/platform` 1、`docs/` 5、仓库根配置 1、`scripts/` 1。这些路径没有层名目录，也没有 `*.test.*` / `*.stories.*` 后缀；按第 2 节优先级不应归层。
- excalidraw 198：`packages/*` 167（`excalidraw` 87、`element` 56、`math` 17、`laser-pointer` 5、`fractional-indexing` 2）、`scripts/` 15、`dev-docs/` 7、`examples/` 6、仓库根配置 3。`packages/*` 中没有语义目录的模块（如 `packages/element/src/Scene.ts`、`packages/math/src/point.ts`）保持未分类，未按包名整体兜底。

**原目标达成情况（已判定不成立，保留为历史记录）：未达到 < 10%。** tsx 停在 46.32%（规则对 tsx 的 `src/esm`/`src/cjs` 双构建目录、`src` 根入口文件、docs/scripts 无覆盖）；excalidraw 从 46.0% 降到 27.58%，剩余主要是 workspace 包内无层语义目录的领域代码。差在「没有可共识的层名依据」，不是阈值口径问题；未为凑比例放宽包名或拓扑兜底。

### 判据调整（2026-11-07，用户确认）

**为什么原 10% 目标不成立：**

- tsx 是平铺结构：自有源码 95 条中 44 条没有层名依据，集中在 `src/esm` 12、`src/cjs` 12、`src` 根入口/工具 8、`src/@types` 2、`src/watch` 2、`src/platform` 1、`docs/` 5、根配置与 `scripts/` 各 1。只有把双构建目录、入口文件、文档和脚本按猜测兜底，才能压到 10% 以下。
- excalidraw 剩余 198 条中 167 条是 workspace 包内的领域代码（`packages/excalidraw` 87、`packages/element` 56、`packages/math` 17 等），包内没有 `actions`/`hooks`/`renderer` 之外可共识的层名目录；按包名整体归层属于硬塞。
- 再压只能依赖「包名兜底」或「拓扑兜底」，与本票「判据是规则确实覆盖，不是比例好看」直接冲突。

**新验收判据（替代 10% 比例）：**

1. 每个 `unclassified` 条目必须带 `no-rule-match` 或 `ambiguous` 原因码；`external` 独立成层并带 `external` 原因码，不出现在未分类列表。
2. 反向用例不得退化：`misc/notes.ts`、`misc/plain.ts`、`src/features/random.ts`、`packages/web/src/index.ts` 保持 `no-rule-match`；`src/utils/both.test.stories.tsx`、`features/tie-none.test.stories.tsx` 保持 `ambiguous`。
3. 每条已归层模块的 `rule` 字段可追溯到具体规则；拓扑只用于命名冲突断平。
4. 覆盖率读数继续如实记录，用于观察趋势，但不再作为通过/失败阈值。

## 7. 验收项

- [x] `external` 独立层出现在 JSON `layerOrder`、模块 `layer` 与 UI 层带/节点视觉角色中，且与 `unclassified` 配色/角色不同。
- [x] 目录语义覆盖 `src/<第一段>`、`packages/*/src`、`app/`、`lib/`、`server/`；workspace 子包内目录可命中。
- [x] `*.test.*` / `*.spec.*` → `tests`，`*.stories.*` → `components`；与目录冲突时按第 2 节优先级裁决并在 `rule` 中可追溯。
- [x] 拓扑断平只在命名冲突时生效，`rule` 记录 `topology:<layer>:support=<n>`；命名规则始终优先。
- [x] 每个 `unclassified` 条目带 `no-rule-match` / `ambiguous`；`external` 条目带 `external` 且不在未分类列表。
- [x] 反向用例全部保持 `unclassified`，原因码符合第 5 节。
- [x] 判据已由「自有源码 unclassified < 10%」调整为可解释性判据：原因码齐全、external 分离、反向用例不退化；原目标及不成立原因保留在 6 节。
- [x] `cargo test --workspace` 全绿，`cargo fmt --check` 通过。

## 8. 失败判据

- 任何 external 包落进 `unclassified`。
- 无命名依据的模块被拓扑或包名兜底归层。
- `unclassified` 条目缺少原因码，或原因码不能解释。
- UI 中 `external` 与 `unclassified` 视觉角色相同。
