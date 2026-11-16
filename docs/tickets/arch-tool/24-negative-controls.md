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

## 未验证的

- 三阶段均未执行，无任何本轮读数。
- 阶段 2 若 `frame-timing.ps1 -Mode ab` 现实现不支持「同一构建同一档」的负对照形态，需先扩展脚本（属 `eval/` 改动，独立提交），扩展前不判本票进展。
