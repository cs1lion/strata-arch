# 29: 锁死 `main.rs` 的 α 乘法点（乘法而非替换）

**状态**：done-with-gap（8 条内联乘法已搬进 lib 纯函数并有 6 条单测锁定；**画面像素未抓屏复核**，按用户指令本批不跑）。

**What to build:** 让"渲染侧 α 只能被乘、不能被替换"这条语义在**所有**实际绘制路径上都被自动化测试与文本守护钉住——此前只有矩形模块节点那一个调用点受 `fade_alpha_contract.rs` 保护，符号节点、`map` 层带与 TimelineView 节点共 8 条同型乘法语句裸奔（挂在 `docs/acceptance-gaps.md` 上多轮）。

---

## 1. 五要素

| 要素 | 内容 |
| --- | --- |
| 命令 | `cargo test -p arch-view --test alpha_scale_contract`（语义锁定）＋ `pwsh -NoProfile -File eval/cite-check.ps1`（文本形态锁定） |
| 固定样本 | `crates/arch-view/tests/alpha_scale_contract.rs` 的 6 条用例，输入取产品真实常量：TimelineView `node_opacity = 0.55`、层带 `band_style` 的 `0.78 / 0.28 / 0.90`、`playback::CONTEXT_OPACITY = 0.68`；不引入新常数 |
| 期望读数或行为 | 6 条用例全绿；`rg '\.a \*=' crates/arch-view/src/main.rs` 命中 **0**；`main.rs` 内 `apply_fade_alpha(` 命中 2、`scale_alpha(` 命中 2、`scale_alpha3(` 命中 1；`cargo test --workspace` 由 40 个 result 行变 **41**、passed 251 → **257** |
| 失败判据 | 出现下列任一即失败：helper 被改成 `a = opacity`（替换）；被改成部分通道缩放（漏乘）；被加地板/钳制（M3 型盲区）；`main.rs` 重新出现内联 `*.a *=`；`scale_alpha3` 与 `apply_fade_alpha` 语义分叉；缩放顺带改动 h/s/l 或 RGB；某条用例被删除或改名（由 `alpha-test-*` 守护咬） |
| 自动化状态 | **已自动化**（单测 6 条进 `cargo test`，10 条守护进 `cite-check`，两者都在 CI job 内）。仍未自动化的是像素侧：本批未抓屏，故"搬迁后画面一字未变"只有 by-construction 论证 + 单测，无本会话实测 |

---

## 2. 范围纠正（先于一切：上一轮草案有两处失实）

- 草案称"9 个裸点，含 1912 / 3141 行 `overlay.opacity *= overlay_reveal`"。实测 `main.rs` **不存在**该语句；全仓唯一的 `opacity *=` 在 `arch_view_canvas.rs:499`（canvas 渲染器路径，不在"锁 main.rs"的任务范围内）。
- 草案称"2937 已把 overlay 交给 `apply_fade_alpha`"。不实：`main.rs` 此前只有 2890 一个调用点。
- ⇒ 真实清单：`rg -c '\.a \*=' main.rs` = **8 条语句 / 3 个绘制块**（块 A 2 条、块 B 3 条、块 C 3 条）。逐块归属、行号与 opacity 来源见 `docs/evidence/29-alpha-scale-sites-20261119.json`。草案里"删除面 23 行"也随之下修为 **17 行**（实测 numstat 16/17，逐行核对全是被搬走的 `let mut` + 乘法 + 1 条过期英文注释）。

## 3. 为什么这是"锁定"而不是"重写"

`let mut bg = item.bg; bg.a *= item.opacity;` 与 `let bg = scale_alpha(item.bg, item.opacity)` 是**逐字相同的算术**，无分支、无副作用 ⇒ 等价性 by-construction，不依赖抓屏。编译器另抓到一处真实细节：块 B 的 `opacity` 是 `&f32`（元组解构自 `&band_draws`，原式靠 `impl MulAssign<&f32> for f32` 才编得过），故调用必须写 `*opacity`，值同一、语义不变（首版提交因此失败：`E0308 expected f32, found &f32`）。

## 4. 判别性设计与反证（关键：每条测试都要能被弄红）

所有喂入的 `a` 都 ≠ 1.0——`a = 1` 时乘法与替换同值，测试失去判别性（`fade_alpha_contract.rs` T2/T3 的既有教训）。注入缺陷实测（对已提交实现，之后 `git checkout --` 还原并复测全绿）：

| # | 注入 | 结果 |
| --- | --- | --- |
| M1 | `scale_alpha` 改替换 `out.a = opacity` | 当时 5 条 **全红**。原预期"等价性测试可能仍绿"**是错的**：`scale_alpha3` 内部即调用 `scale_alpha`，与 `apply_fade_alpha` 必然分叉，被 T3 咬住 |
| M2 | `scale_alpha3` 只乘第一个通道 | 恰 2 条红（T2、T3）、3 条绿 ⇒ 用例不冗余，各守一段 |
| M3 | `(a * opacity).max(0.05)` 加地板 | **首跑 5 条全绿 ⇒ 真实盲区**：既有输入的乘积都高于地板。据此补 **T6** `scale_alpha_has_no_floor_or_clamp`（喂 `0.02×0.5=0.01`、`1.0×0.0=0`、`0.04×1.0=0.04`），重放 M3 ⇒ **恰 T6 红**（5 绿 1 红） |

T6 是被 M3 逼出来的，不是凭空加的——用例 doc 注释与本档 `mutations[M3]` 都写明了这一点。

## 5. 提交链

| 笔 | 提交 | 层 | 删除面 |
| --- | --- | --- | --- |
| 1 | `49c5432` | `refactor(arch-view):` | main.rs 16/17、canvas 24/0（17 行全部为被搬走的内联语句，已逐行核对） |
| 2 | `43962d0` | `test(arch-view):` | 新测试文件 193/0、`fade_alpha_contract.rs` 头注释补正 2/**0** |
| 3 | 本批 docs | `docs:` | 票 29 + 证据档 + §8 第 45 项 + README 行，**0 deletions** |
| 4 | 本批 registry | `eval(registry):` | 追加 10 条守护，A 口径 `expect 165 → 175`（唯一删除即 expect 行） |

顺序说明：原计划"registry → docs"，实测**必须颠倒**。因为 `cite-check` 对 `file-exists` 类守护不只验文件存在，还要求 `needle` 在 `doc`（`docs/acceptance-gaps.md`）里出现 ≥1 次——即"登记路径必须已被文档引用"。先登记会立刻红（本批实测到：`alpha-test-file FAIL 文档未出现登记字面`）。

## 6. 未做与未验证

- **未抓屏**：`27/27` 与 P1 读数（`changed=7.893%`、`hotCells=428`、`meanMax=22.32`）在搬迁后是否逐字不变，本批未观测；由用户另行授权后跑一次 `pixel-evidence` 才算闭合。
- **未判定**：块 B 的"双层 reveal"叠加（`band_style` 内部已按 `0.78·reveal_alpha` 写 alpha，外层又乘一次 `reveal`）是缺陷还是刻意设计——本票只保证算术与搬前一致，未改行为。要重查须先证明产品意图，不得顺手改。
- **无守护**：`arch_view_canvas.rs:499` 的 `e.opacity *= reveal_alpha` 仍在裸奔（本票范围外，留此指针）。
- **未做**：`main.rs:2890` 那处调用仍是全限定名（未顺手改成短名——多 1 行删除，超出声明的删除面）。
- 未 push、未开 PR。
