# NIGHTLOG — 2026-11-07 夜间自主推进

授权：用户休息，约 10 小时自主推进；每票按 ENGINEERING-DISCIPLINE 档位验收、可直接分层提交并 push。
红线：不放松判据/不调阈值/不事后补红绿；未验证项显式登记；不改写已推送提交；不 force push。

约定：时间使用本地时间（Asia/Shanghai）。「读数」均来自实测命令输出。

---

## 22:00 前（承接白天会话）

### 04c 调用图语义补全

- 完成：`external-call` 覆盖外部窄调用与 CommonJS `require()` 绑定；`call-unattributed` 默认只留 `stats.unattributedCalls` 聚合，`--include-unattributed` 出明细；`CALLGRAPH_SCHEMA_VERSION = 2`。
- 提交并 push：`1e4a19e`（实现）、`e03cbd9`（测试）、`9b5faad`（文档）。
- 读数（复跑实测）：tsx 边 2460 / 未解析 1283 / 3 类原因 / 624；excalidraw 57732 / 1575 / 1 / 7388 / 1576；index 产物 SHA256 与 eval-harness 第 8 节记录一致（语料未漂移）。

### 样本仓 pin 与拉取脚本

- 产物：`eval/corpus.toml`（tsx @ `4473971`、excalidraw @ `701b901`，含文件数与 manifest 哈希）、`eval/fetch-corpus.ps1`（缺失拉取、存在校验、失败 exit 1、`-VerifyOnly`/`-Force`）。
- 读数：两仓 `-VerifyOnly` OK；临时缓存拉取 tsx 成功；篡改文件后 exit 1（反向用例）。
- 提交：`1ab78cf`（未 push，与 08 一并推送）。

---

## 22:49 08 git 历史采样与演化指标

- 开始：读 ticket 08，A 档 TDD，先写测试得到真实 Red（`history` 子命令不存在）。
- 完成：`arch-core/src/history.rs`、CLI `history` 子命令、core golden 6 项、CLI 7 项、fixture 构造脚本。
- 实现口径：单次 `git log --first-parent --numstat -M` + 3 次 rev-parse；合并提交整段跳过；重命名按 `-M`（含 brace 形式）归并谱系；只统计 TS/JS 扩展名并复用 index 的忽略规则。
- fixture 读数：`src/app.ts` count=2、churn=4/1、span=86400；`src/helpers.ts` count=3、churn=5/1、span=345600；`commitsScanned=6`、`modules=2`、`truncated=false`。
- 截断反向用例：`--max-commits 2` → `truncated=true`、`commitsScanned=2`、`commitsAvailable=6`。
- 子目录识别：`tests/fixtures/tiny` 在 Strata 工作树内，`isGit=true`、`path`/`head` 与 `git -C <dir> rev-parse` 一致（修正 ticket 01 只看根 `.git` 的限制，仅落在 history 产物）。
- 非 git / 无提交 / 浅克隆：均退出 5（浅克隆用 `git clone --depth 1 file:///...` 覆盖）。
- 确定性：同参数两次运行字节一致。
- 全量：`cargo test --workspace` 77 passed / 0 failed；`cargo fmt --all --check` OK。
- 性能：本仓 19 提交 0.66s；合成 100 文件 × 500 提交 0.99s；excalidraw 完整克隆**未验证**（L2 缓存无 `.git`），已登记在 ticket。
- 偏差记录：ticket 原文 oracle `rev-list --count --first-parent HEAD -- <path>` 会把合并提交算入（实测 3），改用 `--no-merges` 等价 oracle（实测 2）；原判据保留并写明理由。
- 自主决定：`index` 的 `repo.isGit` 维持 ticket 01 行为（改为 show-toplevel 会让 `tests/corpus/*` 带上 Strata 自身 HEAD，pin 的 SHA256 随提交漂移），只修 `history`，记入 backlog。
- 未验证：excalidraw 默认窗口性能。

---

## 23:2x 中断：图片规范落文档（未提交，等确认）

- 来源：用户上传《Skill 自迭代能力改造指南》长图，要求逐条转写并对照现有纪律文档。
- 处理：`ENGINEERING-DISCIPLINE.md` 新增第 10 节（诊断四问、先提标准再确认、四种改造动作、四类验证状态、核心原则）；`AGENTS.md` 只在必读加一行指针。
- 已覆盖不重复：无证据不宣称通过、自动检查与证据、样例回归、主观判断人工复核。
- 冲突与裁决（2026-11-08，用户均选 A）：
  - 规则形态：优先落成检查/工具，文档只留指针；例外是无法自动化的规则仍写条文并登记人工项（注明原因与复核时机）。
  - 提问上限：每轮 3 问仅约束 Skill 改造访谈；grill / wayfinder / to-spec 的未决问题仍逐个问清。
  - 被否的 B 方案与理由保留在 10.6。
- 状态：文档按 `docs:` 单独提交并推送。

## 06 入口候选与最短路径查询

- 开始：读 ticket 06，A 档 TDD，先写 CLI 与 core 测试。
- Red（实现前实测）：`cargo test -p arch-cli --test path` → 0 passed / 9 failed，全部因退出码 2 与 stderr「未知子命令: path」；`cargo test -p arch-core --test path` 编译期失败（`arch_core::guided_path` 不存在）。
- 完成：`arch-core/src/guided_path.rs`（`list_entries` / `build_path`，`PATH_SCHEMA_VERSION = 1`）、CLI `path` 子命令、core 测试 7 项、CLI 测试 10 项、`tests/fixtures/path`。
- 实现口径：入口来自 `package.json` bin/main/module/exports（保留声明顺序）、根与 workspace 包目录 `index.*`/`src/index.*`、入口模块公开导出；`dist|build|out|lib → src` 启发式映射；BFS 只走调用边，邻接按 `(target, kind, line, column)` 排序；不可达退出 4 且 stdout 为空，零候选退出 5，未知符号退出 2。
- 读数（实测，`target/debug/arch-cli.exe`）：
  - tsx（`4473971`）：entries 8（bin 1 / exports 7），0.19s；golden `src/cli.ts#module-toplevel → src/utils/ipc/get-pipe-path.ts#function:getPipePath` 3 步（`server.ts` 中转），0.16s。
  - excalidraw（`701b901`）：entries 17（index 3 / src-index 6 / public-export 8），2.38s；golden `#component:ExcalidrawBase → i18n.ts#function:setLanguage` 3 步（`InitializeApp` 中转），2.31s。
  - fixture：entries 含 bin/main/public-export 三类；`src/cli.ts` → `helper` 走 `alt.ts` 3 步，`via` = null/call/call。
- 反向用例：不可达 exit 4 + stdout 空；未知符号 exit 2；零候选 exit 5；两次运行字节一致；core 重算调用图逐对断言相邻边存在。
- 全量：`cargo test --workspace` 93 passed / 0 failed；`cargo fmt --all --check` OK。
- 偏差（未静默放宽，已写入 ticket）：依赖图补足未实现（`import` 不等于执行、落点符号无唯一答案，待产品确认）；调用图对 `root.render(<...>)` 参数内 JSX 不产生 module-toplevel 边，导致部分入口无路径，golden 改从公开组件起步；构建映射为启发式，失败即丢弃；`path` 每次重建调用图与层，无缓存。

（后续每票在此追加）
