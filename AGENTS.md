# AGENTS.md

Strata：面向「第一次读陌生 TS/JS 仓库」的本地优先架构解析与可视化工具。
Rust workspace：`arch-core`（索引 / 解析 / 层推断 / 调用图）、`arch-cli`（headless CLI）、`arch-view`（GPUI 桌面端）。

## 必读

动手前先读 [docs/process/ENGINEERING-DISCIPLINE.md](docs/process/ENGINEERING-DISCIPLINE.md)。
它是工程纪律的唯一入口：TDD 档位、验收标准、评估体系、流程链、报告与提交纪律、反作弊红线、Skill 与流程条文改造（第 10 节）。
规则新增口径：优先落成检查或工具，文档只留指针；确实无法自动化的写条文并登记为人工项（见第 10.6 节）。

## 不可协商的红线

1. 不看读数回改判据；不调阈值、不放宽判据、不硬塞分类、不事后补写红绿证据。
2. 判据不成立时先报告并给可解释替代，保留原判据与推翻理由，不静默删除。
3. 每张票必须有「命令 + 固定样本/commit + 期望读数或行为 + 失败判据 + 自动化状态」。
4. 报告按「做到的 / 没做到的 / 未验证的」三档分开，读数必须来自实测输出。
5. 按层拆分提交；不 amend、不改写已推送提交、不擅自 push 或改远端 issue。
6. `eval/` 期望值改动必须独立提交并说明旧读数、新读数与理由。

## 事实边界

- `gpui 0.2.2` 无公开离屏导出入口；截图验收已移交 ticket 11（见 `docs/acceptance-gaps.md` 第 2.2 节）。
- 技能装在 `.trae/skills/`；`github.com:443` 不可达时只用 `api.github.com` 归档更新。
