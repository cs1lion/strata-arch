# 24: 门禁自证（negative controls：证明现有判据能变红）

**What to build:** 为项目已有的三类门禁各补一个「已知坏」构造实验，证明它们会在人为破坏时**必然报红/必然判显著**，并在恢复后回到绿。今天「不看读数回改判据、不事后补红绿证据」这条红线靠人守；本票把它变成机器可检查的事实。本票不改任何判据、阈值或基线期望值。

**动机:** drift-gate、`eval-harness.md` §6.1 显著性判定、`eval/pixel-evidence.ps1` 像素断言都只有「通过」的历史记录，没有一次「故意做坏 → 门禁报警」的记录。一个从未变红过的门禁，与一个坏掉的门禁，在证据上长得一样。（EDD 范式中的 Break-the-application-on-purpose 一步；判据要求由 ADR 0004 定名。）

**Blocked by:** 无。可与票 22/23 并行（触碰面：`eval/` 与 `docs/`，不改 `crates/` 行为）。

**Status:** proposal-only（2026-11-16 用户批准开票；未执行，无本轮读数）。

## 阶段 1：drift-gate 必须能报红（headless，可进 CI）

1. **命令**
   - 绿基线：`pwsh eval/drift-gate.ps1 -PlanOnly`
   - 注入坏：对 `tests/fixtures/drift` 的一个副本加入一处真实结构变化（新增一个跨层 import），在副本上 `arch-cli drift <副本> --base eval/baseline/drift-tsx.json`
   - 报警断言：进程非零退出 + 报告列出该新增边所在的结构项
   - 复原：删除副本，重跑绿基线，必须回到退出码 0
2. **固定样本**：`tests/fixtures/drift`（副本，不改动原 fixture）；`eval/baseline/drift-tsx.json`、`drift-excalidraw.json` 只读，**不写入、不改期望值**。
3. **期望读数或行为**：注入后必红、复原后必绿，二者各实测 1 次并归档命令与退出码。
4. **失败判据**：注入后仍绿（= 门禁对这一类变化盲视，须登记为 drift-gate 覆盖缺口并如实报告，**不得**为了让实验好看而改基线或加过滤）；用原 fixture 而非副本做破坏；复原后不绿（说明有写副作用，属真 bug，单独报）。
5. **自动化状态**：可自动化——纯 CLI 计算，无窗口依赖；可作为 `eval/drift-gate.ps1` 的 self-test 分支，或独立 `cargo test -p arch-cli --test drift_gate_selftest`。是否进 CI 由用户裁决。

## 阶段 2：显著性判定必须既能判「显著」也能判「不可判定」（本机人工项）

> **执行前必读（2026-11-16）**：本阶段已按另一种形态部分执行，且原设计被实测暴露为不成立——见文末「阶段 2 的部分完成与设计修订」。按本节原文直接执行会把「降格收益与代价并存」误判成「检验力不足」。

1. **命令**：`pwsh eval/frame-timing.ps1 -Mode ab`，用现成开关构造**正对照**与**负对照**各一次：
   - 正对照（已知应有收益）：ON 档用 `STRATA_RENDER_SKIP` 强制跳 labels+edges（票 21 路线 C 机制），OFF 档不跳 —— 判定必须为「显著」。
   - 负对照（已知应无收益）：两侧同一构建同一档 —— 判定必须为「不可判定/无显著差」。
2. **固定样本**：`tests/corpus/excalidraw`（802 模块 / 4363 边 / 6 层带）；release；窗口 1200×800；≥12 段 × 60 帧，分段数与 `-Mode stability` 一致。
3. **期望读数或行为**：正对照 gap 满足 `gap > k × (2 × 段间 std / √N)`（k=2）且符号检验方向一致；负对照不满足。两侧段间 std/CV/极差一并归档。
4. **失败判据**：负对照被判「显著」（假阳性 → §6.1 口径有缺陷，须报告并复核公式实现）；正对照被判「不可判定」（检验力不足，须报告并考虑提高分段数，**不得**改 k 或阈值来凑）；用两批顺序比较替代同进程交替；只跑一次。
5. **自动化状态**：不可进 CI（需可见 Windows 桌面会话 + release 构建 + 语料 pin）。原始 JSON 归档，登记 `docs/acceptance-gaps.md`。

## 阶段 3：像素证据链必须能抓到真实画面差异（本机人工项）

1. **命令**：`pwsh eval/pixel-evidence.ps1` 先跑 24/24 绿基线；再以「已知差异」对照跑一次——用 `STRATA_EVIDENCE_ANIMATION` / `--no-reveal` 这类**已声明的产品状态差**构造，断言 P1/P3 类帧差判据按预期由「无变化」翻到「超阈值」，反之亦然。
2. **固定样本**：`tests/corpus/excalidraw` + 现有八状态；DPI-aware 抓屏（`SetProcessDpiAwarenessContext(-4)`）为前置条件，不作可选项。
3. **期望读数或行为**：已知差异下 `changedRatio`/`hotCells` 必须越过登记阈值；无差异静态基线必须低于噪声上限（现状：静态基线 changed=0.000000%）。
4. **失败判据**：已知差异未被抓到（像素断言失效）；把新增帧当回归失败却未登记为产品行为；改动阈值文件。
5. **自动化状态**：不可进 CI，人工项（同 acceptance-gaps 2.2.1 的 12 条边界全部适用）。

## 共用口径

- 三阶段各自独立提交；`eval/` 若有脚本改动独立提交（`eval:` 前缀 + 旧/新读数 + 理由）。
- 任何阶段失败都要如实报剩余，不调阈值、不改判据、不静默删除本票。
- 全部门禁 self-test 的产物遵守 ADR 0004 的 run 不可变约定：每次新目录。
- 向本票、票 23 或 `acceptance-gaps.md` 登记任何数字之前，按 `ENGINEERING-DISCIPLINE.md` 第 5 节第 7 条**先脚本回读源数据**再落笔（回读命令与输出留在同一轮汇报）。本票三阶段尤其要核「这个数字属于哪一批 run 目录」：写之前用 `Get-ChildItem` 确认该档名在列——实测 `out/paired-c-20261116-173507`（c1 17:35）含 labels 与 edges-labels，`out/paired-c2-20261116-190406`（c2 19:04）含 edges-labels 但不含 labels；本票已发生一次此类误记（负对照归档初稿误写 c2，改正为 c1，记录在 `afb09fe` 提交信息，正文未记）。工具化检查 `eval/cite-check.ps1` 尚未落地，本条为人工判据，不得因无检查项而视为已通过。

## 未验证的

- 三阶段均未执行，无任何本轮读数。
- 阶段 2 若 `frame-timing.ps1 -Mode ab` 现实现不支持「同一构建同一档」的负对照形态，需先扩展脚本（属 `eval/` 改动，独立提交），扩展前不判本票进展。

## 阶段 2 的部分完成与设计修订（2026-11-16 实测登记；上方原判据与失败判据一字不改）

票面原文假设「用 `-Mode ab` + `STRATA_RENDER_SKIP` 构造已知有收益的正对照」，实测与源码核查显示该假设不成立。本节登记已完成的部分、不成立的原因，以及已用什么替代。

- **负对照：已完成且通过。** 票 23 筛查中 `-Skip "edges-labels"` 因连字符写法（`STRATA_RENDER_SKIP` 实为逗号分隔，`main.rs:74-79`）静默退化为 base↔base；该组配对重算得 `mean(gap)=+0.003ms`、同号 3/6、门槛 1.494ms ⇒ 判**不显著**。即显著性口径在真实零效应下**未产生假阳性**，正是本阶段要证的负对照性质。归档 `out/paired-c-20261116-173507/`（连字符档所在批，即 c1 17:35）、`out/paired-regression-20261116-195426/paired-c-20261116-173507-edges-labels.json`。
- **正对照：原设计做不到，原因两条实测/源码依据。** ① `-Mode ab` 不轮换 `RenderSkip`：每帧 `from_env()`（`main.rs:3172`），档名仅 `base/A/B/BA/AB` 且只映射 `RenderTuning`（`main.rs:121-158`、`3096-3102`），段边界还是绝对帧序号（`main.rs:3177`、`3184-3189`）与按键轮次不 1:1；② 降格的真实效应**不是干净的「已知有收益」**：降 `edges` 在 p95 侧 +1.593~+3.465ms 改善，median 侧却是 −0.622~−1.760ms 代价（见票 23 阶段 C′）。按原判据「ON 档必须判显著」执行，会在 median 侧得到反向结论，进而被误读为「检验力不足」而非「效应本身有代价与收益两个方向」。
- **已实现的替代正对照（判据式行为级）。** `eval/paired-ab-selftest.ps1` 的性质断言：PC1 同数据在 `SigMinRatio=1.001` 下必须不显著、PC2 在 `K=1000` 下门槛必须抬高且结论必须翻转、PC4 `rel%` 必须用同侧 base 均值作分母；并用三处判据式注入复测证明这些断言真会抓（同号阈值写死 → PC1 报 2 条；k 写死 → PC2 报 2 条；分母取错 → 夹具 4 条 + PC4 1 条），提交 `97e8817`。这替代的是「判据能否正确区分显著/不显著」这一半目标，不替代真实降格档的正对照。
- **本阶段剩余缺口（仍按原判据待做）。** 帧时间层面的「同进程交替 + 降格档」正对照仍未实现，前置条件是扩展 `-Mode ab` 使 `RenderSkip` 参与 segment 轮换（产品代码改动 + A 档先 Red + 段边界改按轮次计数）。在该能力落地前，本票阶段 2 记为**部分完成（负对照已过、正对照未成立）**，不得记为已完成，也不得据此宣布降格无收益（与票 23 阶段 C 第 5 条、`eval-harness.md` §6.1 第 7 条一致）。

## 阶段 1 的更正与已完成范围（2026-11-17 实测登记；上方原判据一字不改）

票面阶段 1 的若干前提经本轮实测不成立或弱于实现，且 arch-cli 这一层的构造实验其实已在测试里完成却未在本票记账。本节只追加说明，**上文阶段 1 的第 1 至第 5 条原文保留不改**。

- **Status 更正**：票头 `proposal-only（未执行，无本轮读数）` 对 arch-cli 这一层已不成立——见下「已完成范围」。本节所有数字均为本轮（2026-11-17）现场回读，非引用旧值。

- **「绿基线 = `pwsh eval/drift-gate.ps1 -PlanOnly`」不成立**：`-PlanOnly` 只输出计划、**不执行任何检测**。实测本轮：`-PlanOnly` 退出码 0，且其输出经匹配判定**不含** `self: passed` 或 `self: FAILED` 任一执行结果字样（含计划/NOT REQUIRED 字样）。拿它当绿基线，证的是「改动是否触及输入面」，不是「drift 门禁通过」。真正执行的是 `Invoke-DriftGate`，其结果字样为 `self: passed（…）` / `self: FAILED（N 条新增违规）`。

- **阶段 1 第 1 步指定的基线用错**：`eval/baseline/drift-tsx.json` 的模块集属真实语料（`src/esm/…`、`docs/.vitepress/…`），与 fixture 的 `cli/main.ts` 等 8 个模块不是同一套。本轮实测：**未注入**的原 fixture 配该基线即 `退出码 1`，注入后同样 `退出码 1`（`wrongbase_untouched=1 / wrongbase_injected=1`）——前后皆红 ⇒ 该组合对本实验**无判别力**。fixture 对应的基线是 `tests/fixtures/drift/drift-baseline.json`（即脚本里的 `$selfBaseline`）。

- **「报警断言：进程非零退出」是弱判据**：真报红 = **1**（`CliError::DriftDetected`），输入类错误 = **2**（本轮实测 `--base` 指向不存在路径 → `missingbase=2`）。二者皆非零。断言必须写「恰为 1」并叠加报告内容，否则「构造无效」会被误当成「门禁报警成功」。

- **一处注入会产生两类违规**：本轮向副本 `utils/format.ts` 追加一条指向 `server/api.ts` 的反向 import，实测 `gate.passed=False`、`newViolations=2`，kind 分别为 `reverseEdge`（`utils→server`）与 `cycle`（`[server/api.ts, utils/format.ts]`）——因原图已有 `server→utils`，反向即成双向；`gate.checks.reverseEdges.newCount=1` 与 `cycles.newCount=1` 同步非零。故任何「violations 恰为 1 条」或「只认 reverseEdge」的断言与实现不符。

- **已完成范围（arch-cli 层，headless 测试，已在 `cargo test --workspace` 内）**：`crates/arch-cli/tests/drift.rs` 的 6 个测试（本轮 `cargo test -p arch-cli --test drift -- --list` 列出，全部在 246 之内）已覆盖阶段 1 的核心结构——`drift_with_matching_baseline_exits_zero_and_reports_empty`（绿基线 + `gate.passed=true` + `reverseEdges.newCount=0`）、`drift_gate_failure_exits_one_with_a_full_report`（**副本 + 追加反向 import → 断言退出码 1、`passed=false`、`violations[0]` 的 kind/source/target**）、`a_new_file_fails_the_gate_with_its_layer_and_module`（`moduleChange` 类注入）、`missing_or_invalid_baseline_exits_two_without_partial_json`（输入错误 = 退出码 2 且不吐半截 JSON）、`drift_record_writes_a_portable_deterministic_baseline`、`drift_argument_contract_rejects_missing_or_conflicting_flags`。这些已满足阶段 1 第 2 条（用副本、不动原 fixture、基线只读）与第 4 条的多数禁止项。

- **残余（阶段 1 尚未闭合之处，均指 `eval/drift-gate.ps1` 这一 CI 门禁层，而非 arch-cli 层）**：
  1. `Invoke-DriftGate` 的状态映射把 `failed` **只交给退出码 1**，不看 `newViolations` 是否非空；drift-gate 作为 CI 门禁「故意注入 → 整体变红」**从未跑过一次**。若把该映射改成 `1 { "passed" }`，现有任何检查都不会报警——正是本票「一个从未变红过的门禁与坏掉的门禁在证据上长得一样」所指。
  2. 阶段 1 第 4 步「复原后重跑必须回绿」在**脚本路径**上无记录（rust 测试用 TempDir，不覆盖脚本聚合）。
  3. 平台耦合：`$Exe` 默认 `target/debug/arch-cli.exe` 带 `.exe` 后缀，现状 `drift` job 为 `windows-latest` 故成立；self-test 若移到 Linux runner 必须显式传 `-Exe`。

- **本轮取证自纠（§5 第 7 条坑族的新变体，是否入册待裁决）**：探针最初用 `Select-Object -First 1` 截断 `arch-cli` 的输出管道，PowerShell 因此**提前终止子进程并污染 `$LASTEXITCODE`**，得到「不存在的 `--base` 也返回 1」的错误读数，一度据以怀疑既有 rust 测试的断言。改用 `Out-String` 完整捕获（不提前结束管道）后重测为 **2**，与 rust 测试一致。教训：**断言原生命令退出码时，禁止用 `-First`／`-Last` 之类会提前结束管道的方式截取输出**；`$LASTEXITCODE` 只有在管道被完整消费后才可信。

- **未验证**：`build` / `drift` job 是否有 GitHub runner 的真实执行历史，本轮**未取证**——不能因 rust 测试存在即宣称「阶段 1 已在 CI 生效」。此外阶段 1 第 3 条要求的「注入必红、复原必绿各实测一次并归档命令与退出码」仍未归档，本节读数属**可行性核实**，不等同于本票执行完成。
