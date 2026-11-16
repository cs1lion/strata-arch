# 25: 可用性人测执行前置——answer key ↔ CLI 实测一致性预检

**What to build:** 票 12 的四件套测试包已就绪但从未执行。在人测招募之前，先用 `arch-cli` 对样本仓 `privatenumber/tsx` @ `4473971f...` 实际跑出 T1/T2/T3/T4 依赖的结构读数，与 `docs/research/usability/answer-key.md` 逐条比对，证明答案 key 本身可复核。理由：key 错会让参与者「答对却被判错」或「答错却被判对」，整场 5 人测试的数据作废，而这是唯一能用命令消除的失效模式。

**动机更正（2026-11-16 登记）**：先前一处评审意见称「§4 的任务与标准答案从未冻结」，**不成立**。票 12 交付物记录（2026-11-08）已含 T1–T5 任务、答案 key、主控脚本、观察表与通过阈值（5 人 / 每题 ≥4/5）。真实缺口是执行，与本票一致。acceptance-gaps §8.6 的表述准确，不需修改。

**Blocked by:** 12（测试包与 key 冻结规则：首位参与者开始后不得修改）。

**Status:** proposal-only（2026-11-16 用户批准开票；未执行，无读数）。

## 验收五项

1. **命令**
   - `arch-cli path tests/corpus/tsx --entries`
   - `arch-cli path tests/corpus/tsx --from <T1 入口ID> --to <T1 目标符号ID> --narrative`
   - `arch-cli layers tests/corpus/tsx`；`arch-cli callgraph tests/corpus/tsx`
   - `arch-cli history tests/corpus/tsx --max-commits <N>`（T2/T3 的热度与沉寂答案）
   - 结构 diff：`arch-cli drift` 或票 17 的双 commit diff 路径（T4 的增/删/改答案）
   - 具体 flag 形态执行时按 `arch-cli --schema` 与实际 help 核实，本票不预设参数拼写
2. **固定样本**：`privatenumber/tsx` 完整 git clone @ `4473971f4b3261de253209b148d8fdc43269fcc4`（票 12 已钉；须非浅克隆，否则 history/diff 不可用）；`tests/corpus/tsx` pin 快照（`eval/corpus.toml` SHA256 校验通过）；release 构建。
3. **期望读数或行为**：key 中每一条被 CLI 输出的结构项（模块/符号序列、经过的层、热度 Top-3、沉寂 2 个模块、单 commit 的增删改集合）与 key 逐字段一致；不一致项逐条列出「key 值 / CLI 值 / 判定哪一方错」，不合并、不四舍五入。T5（边语义与步进顺序）属界面文案理解，不由 CLI 预检，明确登记为「预检不覆盖」。
4. **失败判据**
   - 预检发现 key 有错时**在人测前**修正并记录为 key 的第 0 版更正（此时尚无参与者开始，不违反票 12 冻结规则）；若已有参与者开始才发现问题，按票 12 判据「答案 key 在测试后被修改」记失败，不得中途改 key 继续。
   - 拿 fixture（`tests/fixtures/layers` / `path`）的读数替代真实 tsx 样本得出不一致结论；
   - 只跑一条命令就宣称 key 有效；浅克隆上跑 history；
   - 用预检结果顺手改判据、改阈值或缩短任务数。
5. **自动化状态**：部分自动化——比对本身可用脚本 headless 执行（CLI 纯计算，无窗口依赖），建议落 `eval/usability-key-check.ps1`（`eval/` 新增，独立提交；2026-11-16 用户已授权该落盘计划）；但「参与者 5 人执行测试」仍为纯人工项，本票不含招募与执行，执行仍归票 12。

## 参考实测（2026-11-16，`tests/fixtures` 而非 tsx 样本，仅示意输出形态，不作预检结论）

- `layers tests/fixtures/layers`：26 模块 / 19 边 / 0 unresolved；`unclassified` 6 个；含 `topology:components:support=2` 这类拓扑判据实例（`features/tie-components.test.stories.tsx`）。
- `path tests/fixtures/path --entries`：4 个入口候选，来源分别为 `package.json:bin` / `package.json:main` / `public-export` / `package.json:exports`。
- `path ... --from src/cli.ts#module-toplevel --to src/utils/helper.ts#function:helper --narrative`：3 步，`src/cli.ts#module-toplevel → alt.ts#function:alt（src/cli.ts:5:1）→ utils/helper.ts#function:helper（src/alt.ts:4:3）`，层轨迹 unclassified → unclassified → utils。
- 上述数字用于确认 CLI 能给出 key 所需的字段形态；**tsx 样本上的正式预检未执行，无结论**。

## 未验证的

- key 全部条目：未与 tsx 实测比对，有效性未知。
- T5 界面语义：预检方法本身不覆盖。
