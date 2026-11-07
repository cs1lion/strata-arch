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

（后续每票在此追加）
