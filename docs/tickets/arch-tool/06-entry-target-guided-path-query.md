# 06: 入口候选、目标选择与最短路径查询

**What to build:** `arch-cli path` 能列出目标仓库的候选入口，接收入口与目标符号，输出一条从入口到目标的有序「模块 → 符号」路径；不可达时给出专用退出码与原因；同一输入输出确定。

**Blocked by:** 02（模块解析）、03（层推断）、04（符号与调用图）

**Status:** done（06；依赖图补足与调用图 JSX 归属缺口见文末）

**固定样本：** `privatenumber/tsx` @ `4473971f4b3261de253209b148d8fdc43269fcc4`（`package.json` 有 `bin` 与复杂 `exports`）与 `excalidraw/excalidraw` @ `701b901a0f1ba3d0b390d9cdf63d112e25abac4b`。冻结的 golden 起终点：
- tsx：`src/cli.ts#module-toplevel` → `src/utils/ipc/get-pipe-path.ts#function:getPipePath`。
- excalidraw：`packages/excalidraw/index.tsx#component:ExcalidrawBase` → `packages/excalidraw/i18n.ts#function:setLanguage`。

- [x] `cargo run -p arch-cli -- path --repo <repo> --entries` 退出码 `0`，输出候选入口列表；tsx 至少包含来自 `package.json` `bin`/`exports` 的候选，且每个候选都映射到已存在模块；零候选时退出码 `5` 并在 stderr 说明原因。
- [x] `cargo run -p arch-cli -- path --repo <repo> --from <entry-id> --to <symbol-id>` 退出码 `0`，输出有序步骤；每步含模块 ID、符号 ID、层与进入该步的边种类。
- [x] 路径合法性断言（core 测试）：任意相邻两步之间在图中存在对应边；起点等于给定入口、终点等于给定目标；不含重复步骤。
- [x] 不可达：给定一个孤立 fixture 中不连通的符号，退出码 `4`，stdout 路径为空，stderr 给出原因；不得回退到「随便给一条路径」。
- [x] 确定性：同一 `--from/--to` 连续运行两次输出字节一致；多条等长路径时选择规则固定（golden 断言路径节点序列）。
- [x] 两个样本仓各至少一条 golden 路径通过；路径长度与步骤序列冻结，变更需说明原因。
- [x] 失败判据：相邻步骤无边、端点不符、不可达未返回 `4`、两次运行不同、golden 漂移。
- [x] 自动化状态：全部自动化（CLI 10 项 + core 7 项）。

## 实现与验收记录（2026-11-08）

### Red（实现前）

- CLI：`cargo test -p arch-cli --test path` → `0 passed; 9 failed`，失败原因统一为退出码 `2` 与 stderr「未知子命令: path」。
- Core：`cargo test -p arch-core --test path` 在编译期失败（`arch_core::guided_path` 模块尚未创建），随后补实现转绿。

### 实现要点

- 新增 `arch-core/src/guided_path.rs`：`list_entries` 与 `build_path`；`PATH_SCHEMA_VERSION = 1`。
- 入口候选：`package.json` 的 `bin`（字符串或对象）/`main`/`module`/`exports`，递归展开 `exports` 并跳过 `types`/`typings` 与 `.css`/`.json`/`.map`/`.d.*`；仓库根与 workspace 包目录的 `index.*`、`src/index.*`；入口模块的公开导出符号（token 级 `export` 扫描，仅取 function/component/class）。
- 构建产物映射：`dist|build|out|lib` → `src`、去掉输出目录段（含 `prod|dev|esm|cjs|lib`）、去掉后补 `src/`；`js|jsx|mjs|cjs` 再尝试 `.ts`/`.tsx`；映射不到已索引模块的候选丢弃。
- 路径：只走调用边；BFS 最短路径；相邻目标按 `(target, kind, line, column)` 字典序排序，先到先得；`--from` 支持模块 ID 自动补 `#module-toplevel`。
- CLI：`path` 子命令，`--entries` 与 `--from/--to` 二选一；`--repo`/位置参数、`--out` 均支持；退出码 `4` 不可达（stdout 为空）、`5` 零候选、`2` 未知符号/参数。
- 每次调用各构建一次调用图与层产物（两次索引扫描）；样本仓耗时在 0.2–2.4s。

### 读数（实测，直接运行 `target/debug/arch-cli.exe`）

- **构建类型：debug**（本票全部耗时读数；release 对照未做）。

| 样本 | 命令 | 读数 | 用时 |
| --- | --- | --- | --- |
| fixture | `path tests/fixtures/path --entries` | 候选含 `src/cli.ts#module-toplevel`（`package.json:bin`）、`src/index.ts#module-toplevel`（`package.json:main`）、`src/utils/helper.ts#function:helper`（`public-export`） | <0.1s |
| fixture | `path ... --from src/cli.ts#module-toplevel --to src/utils/helper.ts#function:helper` | 3 步：`src/cli.ts#module-toplevel` → `src/alt.ts#function:alt` → `src/utils/helper.ts#function:helper`；`via` = null/call/call；层 = unclassified/unclassified/utils | <0.1s |
| tsx | `path tests/corpus/tsx --entries` | 8 个：`package.json:bin` 1 个（`src/cli.ts`），`package.json:exports` 7 个（`src/loader.ts`、`src/cjs/*`、`src/esm/*` 等） | 0.19s |
| tsx | `path ... --from src/cli.ts#module-toplevel --to src/utils/ipc/get-pipe-path.ts#function:getPipePath` | 3 步：`src/cli.ts#module-toplevel` → `src/utils/ipc/server.ts#function:createIpcServer` → `src/utils/ipc/get-pipe-path.ts#function:getPipePath`；`via` = null/call/call | 0.16s |
| excalidraw | `path tests/corpus/excalidraw --entries` | 17 个：`index` 3、`src-index` 6、`public-export` 8 | 2.38s |
| excalidraw | `path ... --from packages/excalidraw/index.tsx#component:ExcalidrawBase --to packages/excalidraw/i18n.ts#function:setLanguage` | 3 步：`ExcalidrawBase` → `InitializeApp` → `setLanguage`；`via` = null/jsx/call | 2.31s |

反向用例（均自动化）：

- 不可达：`src/isolated.ts#function:isolated` → 退出 `4`，stdout 为空，stderr 含「路径不可达」。
- 未知目标：`src/isolated.ts#function:missing` → 退出 `2`，stderr 含「未知符号」。
- 零候选：临时目录只有 `src/only.ts`、无 `package.json`/`index.*` → 退出 `5`，stderr 含「入口候选」。
- 确定性：同参数两次运行 stdout 字节一致。
- 相邻边：core 测试重新构建调用图，逐对断言边存在且 `via` 等于边种类。

全量：`cargo test --workspace` 93 passed / 0 failed；`cargo fmt --all --check` 通过。

### 已知偏差与 backlog

- **依赖图补足未实现**：spec 原写「调用图为主、依赖图补足」，实现只走调用边。理由：纯 `import` 边不等于执行，且落到目标模块的哪个符号没有唯一答案；需要产品确认后再做，未静默放宽。
- **调用图缺口影响入口**：`excalidraw-app/index.tsx` 与 `examples/*/index.tsx` 的顶层 JSX 位于 `root.render(<...>)` 参数内，调用图未产生 `module-toplevel` 的 jsx 边，因此这些入口无法给出路径；excalidraw golden 改从公开组件 `ExcalidrawBase` 起步。该缺口属于调用图归属规则，记入 backlog。
- **构建产物映射是启发式**：`dist → src` 等规则覆盖当前两个样本，但其他构建布局（自定义 `outDir`、多级产物目录）可能映射失败；失败时丢弃候选而非猜测。
- **无缓存**：每次 `path` 重新构建调用图与层产物（两次索引扫描）；样本仓可接受，大仓需后续加缓存。
