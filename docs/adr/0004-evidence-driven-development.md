# 工程纪律定名：Evidence-Driven Development（EDD）

本项目自 Round 2 起实际执行的纪律——判据先于代码、固定样本、可失败的失败判据、读数随 commit/硬件归档、报告分「做到的/没做到的/未验证的」三档——在社区语境中有明确同名范式：Evidence-Driven Development（DEV.to《Evidence-Driven Development: Give Your Coding Agent Something to Prove》2026-10；Jan Bosch《Speed, Data, and Ecosystems》2017 的 Evidence-Driven Development 章为其前史）。2026-11-16 讨论中确认本项目的 ENGINEERING-DISCIPLINE 红线与该范式高度同构，缺的是命名与两处机制（见 Consequences），不是新方法论。

本 ADR 只做两件事：给既有纪律一个可检索的名字；把「靠人守」的红线中可机器化的部分转为工件要求。它**不新增、不放宽、不修改**任何既有判据、阈值或读数。

**Status**: accepted（2026-11-16 用户批准三条 EDD 草案落盘，即批准「定名 + 补 negative control / run 不可变两处机制」这一选项）

## Considered Options

- 不命名：纪律继续只以 ENGINEERING-DISCIPLINE.md 的中文条款存在。成本低，但「判据先行」这一核心机制没有可引用的外部同名范式，新人/新 agent 容易当成项目自创规矩而打折执行。
- 引入完整 EDD 工具链（角色状态机、前置矩阵、`.peaks/` 会话工作空间）。被否：与本项目 `docs/tickets/` + `eval/` + `out/` 的既有工件结构重复，属另一套流程而非同一套。
- **定名 + 补齐两处机制（选中）**：保留全部既有工件结构，只增补 (a) negative control 要求、(b) run 不可变要求。

## Consequences

- `docs/process/ENGINEERING-DISCIPLINE.md` 的等价关系说明已补入：第 12 行 bullet + 第 8 节决策记录表「纪律定名」行（提交 `b5af3ee`，2026-11-16）。该提交只追加指针与记录行，第 1–10 节条文一字未动。另登记一个存量缺口：第 8 节决策表仍缺 ADR 0003（双许可）行，用户指示本轮不补。
- **negative control 成为门禁票的默认要素**：任何新增或修改门禁（drift-gate、显著性判定、像素断言）的票，必须同时给出「该门禁能变红」的构造性证明，否则该门禁视为未验证。落地细节见票 24。
- **run 不可变**：证据产物（`out/**`、`docs/evidence/**`）一次运行一个新目录、永不替换已有读数。**查证更正（2026-11-16）**：`eval-harness.md` 第 74 行早已以 `~/.cache/strata/eval/archive/<git-sha>/<timestamp>/` 规定该约定，缺的不是条文而是实践——`eval/pixel-evidence.ps1` 固定写 `out/pixel-evidence/report.json`，同名重跑覆盖上一批。已由提交 `c3a9347` 在该文件第 76 行补显式条目（含既有缺口登记）、第 264 行澄清歧义的「固定归档路径」。在脚本改造（见票 24 共用口径）落地前，本条状态为「约定已在细则中、执行存在部分缺口」，不作已实现机制宣称。
  - **状态更新（2026-11-17）**：该「脚本改造」已落地——`eval/pixel-evidence.ps1` 默认落点改为 `out/pixel-evidence/<git-sha>/<timestamp>/`，且显式目录若已有读数则拒绝覆盖（退出码 2），并由 `cite-check` 两条守护 + 破坏反证锁定（提交 `be1bd89`、`0412aaa`，细节见票 24 与 `eval-harness.md` §3 的闭合附注）。因此本条 **run 不可变** 在 `pixel-evidence` 这一处从「约定已在细则中、执行存在部分缺口」升为**已实现**；上方 2026-11-16 的更正原文保留，其"不作已实现机制宣称"的限定对该脚本而言已过期。其余 `out/**` 产物是否全部满足该约定，本次未逐一核查，不在升级范围内。
- 「用户能理解」（acceptance-gaps §4）在产品目标维度上仍处于「推断」档：测试包已就绪但未执行，本 ADR 不改变其证据等级，只登记缺口位置（票 25 承接执行前置）。
- 命名本身不产生证据。若后续任何一处 EDD 机制与既有红线冲突，**以既有红线为准**，本 ADR 相应条款作废并记录冲突原因。
