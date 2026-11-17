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

## 阶段 2 的前置能力落地与判据读数（2026-11-17 实测登记；上方阶段 2 原判据与失败判据一字不改）

上方阶段 2 的「执行前必读（2026-11-16）」引用块与文末「阶段 2 的部分完成与设计修订」继续有效。本节登记此后落地的前置能力、实测读数，以及**本笔仍未满足票面之处**。

- **前置能力已落地（三次提交，构成一条链）**：`dff5b38` 给 `eval/frame-timing.ps1` 的 `-AbCycle` 加 token 校验（未知段名直接 exit 2，不再静默落进 `from_env()` 回落）与降格段**能力断言**（缺 `ab:skip` 凭证即 RED）；`c2c4d46` 让 `RenderSkip` 参与同进程段轮换（仅 `skip:<子集>` 段名覆写，其余段沿用 env 值以保 `-Mode ab -Skip x` 语义）、每帧在 `render` 起始单点解析并缓存、在每个段边界发 `ab:skip=<本帧实际生效集合>`；`111e8e5` 把配对规则与七例判据固化为 `eval/ab-skip-selftest.ps1`。
- **阶段 1 第 3 条的归档要求已闭环**（对上一节末句的追加更正，原文保留）：逐例命令与退出码已入仓为 `docs/evidence/24-drift-selftest-20261117.json`（`ff734a2`），档名指针已写入 `acceptance-gaps.md` §8 第 20 条（`6828802`），并由 `ev-drift-selftest-file`（`288fa01`）与 `ev-drift-selftest-json`（`d092b97`）两条登记项守护。
- **正对照必须同 tuning 才有效**：上方阶段 2 第 1 条设想的 `base ↔ skip:edges` 形态经读码与实测判定为**混轴**——`base` 显式关闭边批处理与视口裁剪，而 `skip:*` 段走 `_ => from_env()` 回落即两者全开，差值主要来自 tuning。可用形态改为 `BA ↔ skip:edges`（两侧 tuning 相同）。该口径已写入 `eval-harness.md` §6.1 第 8 条。
- **实测读数（两独立批次，同参数：`-SegmentFrames 20`、cycle 模式 `(BA, skip:edges, skip:edges, BA)` 重复 6 次、`K=2`、同号门槛 0.8334）**：

  | 项 | 批次 A（`out/ab-p3-20261117/`） | 批次 B（`out/ab-skip-selftest-20261117-133021/`） |
  | --- | --- | --- |
  | 正对照 pairs / unpaired | 12 / 1（`BA`） | 12 / 1（`BA`） |
  | median `mean(gap)` / 门槛 / 同号 | 4.368ms / 2.451ms / 12-12 显著 | 6.578ms / 2.729ms / 12-12 显著 |
  | p95 `mean(gap)` / 门槛 / 同号 | 6.491ms / 4.665ms / 11-12 显著 | 13.551ms / 11.757ms / 12-12 显著 |
  | 同配置负对照 | median −0.272 不显著；p95 1.738 不显著 | median −0.336 不显著；p95 1.014 不显著 |
  | 顺序偏置 | delta 0.271 ≪ std 2.123；两组各 6 | delta 0.765 ≪ std 2.364；两组各 6 |
  | 换边重算（P3） | mean 反号 | mean 反号 |
  | 判据整体 | 九例全 PASS、退出码 0 | 九例全 PASS、退出码 0 |

  结论边界：**判据决策跨批复现，效应量未复现**（median 4.368→6.578ms、p95 6.491→13.551ms，约 2 倍）。据 §6.1 第 8 条，此处只可宣称「方向与显著性稳定」，**不得**给出降格收益幅度，也不得据此宣布降格无收益。
- **仍未满足票面之处（不得把本笔记作阶段 2 已完成）**：票面阶段 2 第 2 条要求「≥12 段 × 60 帧」。本轮实际是 **12 对配对、每批仅 20 帧（19 个间隔）**，样本规模低于票面口径；本笔的产物是「判据可重放 + 方向稳定」，不是票面口径下的完整测量。要满足票面需 `-SegmentFrames 60` 且重复数使配对数 ≥12，单轮时长相应增加，属可执行但**尚未执行**的余项。
- **阶段 2 状态（据上修订）**：负对照此前已过；**正对照能力已具备、判据已入仓可重放，但票面样本规模未达** ⇒ 记为「部分完成（负对照通过；正对照判据通过、票面口径样本量未达）」，仍不得记为已完成。
- **未验证**：这些判据从未在 GitHub runner 执行（且需可见 Windows 桌面，本就不进 CI）；`eval/ab-skip-selftest.ps1` 自身尚无登记项守护，删除它或改写其判据文本不会使 `cite-check` 变红；其运行摘要只落在被 gitignore 的 `out/ab-skip-selftest-*/summary.json`，未随证据入仓。

### 票面口径补跑（2026-11-17 当日追加；上方读数与状态表述保留不改）

上节「仍未满足票面之处」所述样本规模差异已按票面第 2 条口径补齐：`-SegmentFrames 60 -Cycles 6`（24 个段实例、12 对配对、每批 60 帧 = 1500 帧，`panFrames=1560`），两次 frame-timing 运行退出码均为 0，`ab-skip-selftest` 整体退出码 0、九例全 PASS，能力凭证 `want=13 got=26 specs=[none skip:edges]`。

| 判据 | 20 帧批次（上方表） | **60 帧票面口径** |
| --- | --- | --- |
| P1 p95 `mean(gap)` / 门槛 / 同号 | 13.551 / 11.757 / 12-12 | **9.053 / 3.264 / 12-12**，rel **49.02%** |
| P2 median `mean(gap)` / 门槛 / 同号（只记录） | 6.578 / 2.729 / 12-12 | **6.226 / 1.704 / 12-12**，rel **42.76%** |
| N1 同配置 median | −0.336 / 1.445 不显著 | **+0.012 / 0.515 不显著**，同号 5/12 |
| N1 同配置 p95 | 1.014 / 5.203 不显著 | **0.657 / 2.299 不显著**，同号 6/12 |
| OB 顺序偏置 | delta 0.765 vs std 2.364 | **delta 0.872 vs std 1.476**；p95 delta 0.393 vs std 2.827；两组各 6（非空转） |
| P3 换边 | mean 反号 | mean 反号（6.226→−6.226；9.053→−9.053） |

**检验力提高的直接证据**：把每批从 20 帧提到 60 帧后，正对照 median 的 `std(gap)` 由 2.122/2.364ms 降到 **1.476ms**、门槛由 2.451/2.729ms 降到 **1.704ms**；同配置负对照的门槛由约 1.2–1.4ms 收到 **0.515ms**，且其 `mean(gap)` 为 **+0.012ms**——接近完美零效应。这正是 §6.1 第 7 条 (b)→(a) 类的转化：同一判据在分辨率提高后，「不显著」开始具备「可排除该量级效应」的含义。

**状态更新（追加，不改写上方表述）**：本票阶段 2 的核心目标——「显著性判定必须既能判显著、也能判不可判定」——**已按票面样本口径达成**：正对照（同 tuning 的 `BA ↔ skip:edges`）在 p95 与 median 两侧均以 12/12 同号过门槛，负对照（同配置）两侧均不过门槛且同号不足。同时文末「阶段 2 的部分完成与设计修订」一节中「本阶段剩余缺口……『同进程交替 + 降格档』正对照仍未实现」一项，其前提条件（扩展 `-Mode ab` 使 `RenderSkip` 参与 segment 轮换）已由 `dff5b38`+`c2c4d46` 完成，该剩余缺口就此闭合；原文按规矩保留不改。

**仍不得据本票说什么**：(a) 效应量不得当产品收益结论——本档降格是**诊断用受控实验**，非发布特性，且三次批次 median `mean(gap)` 为 4.368 / 6.578 / 6.226ms、p95 为 6.491 / 13.551 / 9.053ms，跨批波动仍达 2 倍；(b) 真实产品档（路线 C 的 `labels+edges` 常开降格）收益与代价仍归票 23 阶段 C/C′ 判定，不在本票范围；(c) 判据从未在 GitHub runner 执行（本就不进 CI）。

## 阶段 3 的执行结果：基线不成立，但两侧灵敏度已证明（2026-11-17 实测登记；上方阶段 3 原判据与失败判据一字不改）

- **票面第 1 条假设的「24/24 绿基线」在本机不成立，故阶段 3 未闭环**。基线命令 `pwsh -NoProfile -File eval/pixel-evidence.ps1 -OutDir out/pe-baseline-20261117`，实得 `passed=21 failed=3`、**退出码 1**、单次耗时约 48s。三项失败读数：
  - `p1-frames-distinguishable`：`changed=1.520%`（下限 2.0%）、`hotCells=18`（下限 10 已过）、`bbox=(36,141)-(2560,1816)`
  - `search-focus-closes-panel`：`focus=311`（上限 100）
  - `search-clear-removes-panel-and-label`：`panel=110`（上限 100）、`label=51 offLabel=51`
  - 通过的帧差侧证据：`static-baseline-noise` `changed=0.000000%`（逐位一致）、`p3-adjacent-frame-moves` `maxChanged=0.752%`（下限 0.2%）
  - 归因与处置：本机显示面宽 2560（bbox 右界即整幅宽度），而阈值注释里的实测值（如 changed=0.000000%、单行 tint ≈2400 宽）来自另一台标定机；P1 的 1.520% 落在 2.0% 之下属**标定漂移**而非判据失灵。按票面第 4 条禁止项，**未调任何阈值、未加过滤**，因此本阶段判为「前置未满足」，不得记为已完成。
- **阶段 3 的实质目标（帧差判据必须双向可翻转）已用受控副本证明**，两处注入均不触碰阈值：
  - **抹掉已声明状态差**：副本把 P1 on 侧的 `$env:STRATA_EVIDENCE_ANIMATION = "1"` 改为 `$null` ⇒ `p1-frames-distinguishable` 读数由 `changed=1.520% / hotCells=18` 变为 **`changed=0.000% / hotCells=0 / bbox=(-1,-1)`**，副本整体退出码 1。⇒ 基线那 1.520% 确实归因于该已声明状态差，不是抓屏抖动。
  - **注入已声明状态差**：副本给静帧对照的 AppArgs 加 `--autoplay`，并把 `WarmupMs 1200 → 80` 使采样落在播放期间 ⇒ `static-baseline-noise` 由 **`changed=0.000000%`** 翻到 **`changed=0.721491%`**（上限 0.1% 的 7.2 倍）、`hotCells=37`，副本整体退出码 1。⇒ 噪声上限判据不是恒真式。
  - **未碰阈值的证明**：基线与三个注入副本共四次运行，各自 `report.json` 内嵌的 `thresholds` 快照逐项一致——`P1MinChangedRatio=0.02`、`P1MinHotCells=10`、`P3MinChangedRatio=0.002`、`P3MinHotCells=1`、`StaticMaxChangedRatio=0.001`、`ChangeTolerance=6`；另在副本删除前按符号定界抽取 `$Thresholds` 定义块（66 行）与源做 `StringComparison.Ordinal` 逐字符比较为相等，副本与源的差异行数分别 2/2/4，全部落在状态差与采样窗口两处。这条证据来自运行产物本身，不依赖行号（行号会随脚本改动漂移）。副本已删（经系统回收站，可恢复）。
- **一次无效注入（阴性结果，按规矩保留）**：只加 `--autoplay` 而不改 `WarmupMs` 时，静帧对照仍 `changed=0.000000%` PASS——两帧都落在播放完成之后。⇒ 「已知差异」必须落在采样窗口内才算构造成功；这是注入设计的缺陷，不是判据失灵。首次尝试还因把说明性注释追加到**带反引号续行的语句行尾**（注释吞掉续行符，`-Frames` 退回默认值 0）而使副本在写出 report 之前抛错终止，遂改用「只替换参数数组内部」的注入点。
- **新发现的不稳定面**：`search-focus-closes-panel` 读数在两次运行间由 `311` 跳到 `10389`；另一次运行多出 `search-focus-label-visible` 失败（`bg=0 text=0 rows=-1--1`）而基线没有。⇒ search 态两项在本机跨运行不可复现，不能作为阶段 3 的任何结论支撑，须单列处理。
- **阶段 3 状态**：**未闭环**。要闭环需要下列之一，且均属判据变更、须单独批准：在标定机上取 24/24 绿基线；或按本机环境重新标定 `P1MinChangedRatio` 与 search 两项阈值（这会改动 `pixel-evidence.ps1` 的阈值区，票面第 4 条明令禁止顺手做）。本笔**没有**新增 evidence 文件，只登记读数；判据从未在 CI 执行（需可见 Windows 桌面）。
