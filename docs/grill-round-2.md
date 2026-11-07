# 架构解析与可视化工具 — 需求拷问 Round 2

生成时间：2026-11-07
状态：Round 2 问题清单 + GPUI 形态影响分析 + 验收标准制度；未写代码。

Round 2 开始前已裁决：

- 交付形态：**纯桌面端（Rust core + CLI + GPUI）**，取消本地 Web UI，不做可交互 HTML 导出（`docs/adr/0002-desktop-only.md`）。
- 技术栈：Rust core + GPUI（`docs/adr/0001-rust-core-gpui.md`）。
- 其余 Round 1 裁决见 `docs/grill-round-1.md` 第 5.1 节。

---

## 1. GPUI 形态确认后的影响

### 1.1 事实（有出处）

| 事实 | 证据 |
| --- | --- |
| `gpui` crate 当前版本 0.2.2，许可 Apache-2.0，pre-1.0 | crates.io API `crates/gpui/0.2.2` 的 `license`、`num` 字段 |
| README 明示版本之间常有破坏性变更 | `crates/gpui/README.md`：`GPUI is currently pre-1.0... breaking changes between versions` |
| 桌面后端：macOS（Metal）、Linux/FreeBSD（Wayland/X11）、Windows（Win32/DirectWrite） | `crates/gpui/README.md` 的 Supported platforms |
| 自定义绘制：`canvas` 的 prepaint/paint 回调，配合 `Window::paint_quad`、`paint_path` | `crates/gpui/src/elements/canvas.rs`、`crates/gpui/src/window.rs` |
| 命中测试：`Window::insert_hitbox` | `crates/gpui/src/window.rs` |
| 逐帧驱动：`Window::request_animation_frame` | `crates/gpui/src/window.rs` |
| 动画 API：`with_animation(ElementId, Animation, impl Fn(Self, f32) -> Self)` | GPUI Book 的 `with_animation` 一节 |
| SVG 资产：`svg` 元素支持 path/data/transformation | `crates/gpui/src/elements/svg.rs` |
| 离屏取图：`Window::render_to_image() -> anyhow::Result<image::RgbaImage>` | `crates/gpui/src/window.rs` |
| 交互测试：`gpui::test` 的 `run_test` / `TestAppContext` / `observe` | `crates/gpui/src/test.rs` |

### 1.2 推断（明确标注，不是事实）

- **没有现成图渲染能力**：GPUI 只提供 quad/path/text/hitbox 等底层原语，没有节点-边布局、力导向、平移缩放、标签避让或图查询 API。分层布局、调用图布局、边路由、标签排布都要自己实现。
- **没有 Web 目标**：README 只列桌面窗口后端，没有 wasm/浏览器渲染路径。因此「导出可交互 HTML」不是换个 renderer 就能得到，而是第二条产品线；ADR-0002 已排除。
- **动画没有 CSS/JS 生态**：只能用 GPUI 的 `Animation` + `with_animation` + `request_animation_frame`，帧循环与缓动都自己控制；没有现成图库的过渡动画可复用。

### 1.3 对后续设计的直接影响

| 维度 | 影响 |
| --- | --- |
| 图形渲染 | 分层地图与调用图都走 `canvas` 自绘；平移/缩放是自维护的变换矩阵，命中测试用 `insert_hitbox` 与变换后的坐标做映射；没有现成图库 |
| 动画 | 只做可解释的「路径播放」；帧由 GPUI 驱动，禁止 `Date.now`/`requestAnimationFrame` 这类 Web 习惯；需要可关闭开关与 reduced-motion 处理 |
| 跨平台 | 三平台后端都存在，但 Windows 后端成熟度未在本机验证；开发机是 Windows，MVP 应以 Windows 为主验收，macOS/Linux 先做编译与启动冒烟 |
| 与 CLI 的关系 | CLI 必须是独立二进制，不能链接 GPUI 桌面依赖；解析、索引、图算法放共享 core crate，GUI 只读索引产物 |
| Web 输出 | 没有 Web UI，也没有可交互 HTML；最多输出 JSON/图片等静态产物，供其它工具消费 |
| 测试 | `render_to_image` 让离屏截图成为可能，但尚未在本机/CI 验证；UI 视觉基线不能照搬浏览器截图方案 |

---

## 2. Round 2 问题（frontier）

### R2-1 — 调用图的节点粒度与身份

❓ **R2-1** — 调用图的节点是什么？边是什么？

- A 文件/模块级：节点 = 文件或模块，边 = 文件间 import/call 聚合。
- B 符号级：节点 = 函数/方法/类，边 = 具体调用点。
- C 双层：符号级是底层真相，模块级是聚合视图；主视图看模块，下钻看符号。

为什么必须你回答：Q3 已定「依赖 + 调用」，但「调用」停在文件还是函数，决定数据模型、图规模、以及引导式路径能否回答「谁调用了谁」。20k 符号的规模下，B 会显著放大节点与边。

➡️ 推荐 **C**。引导式路径必须能指到具体函数；模块级聚合负责首屏可读性。符号身份、重载、匿名函数、箭头函数的 ID 规则留到 Round 3。

---

### R2-2 — TS/JS 模块解析的边界

❓ **R2-2** — 第一版解析哪些导入/引用？

- A 相对导入 + `tsconfig` paths + workspace packages；`node_modules` 收敛为包级节点；动态 `import()` 尽力解析；`eval`/反射忽略。
- B 只解析相对导入 + `node_modules` 包级。
- C 引入 tsc/LSP 级语义解析（类型检查器精度）。

为什么必须你回答：决定精度与成本；B 会漏掉 monorepo 别名与 workspace 包，C 会把 MVP 变成编译器集成项目。

➡️ 推荐 **A**。无法静态解析的边要显式标记为 `unresolved` 并计数，不能静默丢弃。

---

### R2-3 — 入口与目标如何确定

❓ **R2-3** — 引导式路径的起点和终点从哪来？

- A 自动候选 + 用户选择：MVP 候选 = `package.json` 的 `main`/`bin`/`exports`、`index.*`、公开导出符号；框架路由（Next/Express 等）后置。
- B 完全手动输入文件/符号。
- C 自动选唯一入口，不给用户选。

为什么必须你回答：入口定义直接决定「上手会话」是否成立；同一个仓库可能有多个合理入口。

➡️ 推荐 **A**。自动给出候选并标注来源，用户拍板；MVP 不承诺框架路由识别。

---

### R2-4 — 「层」的定义

❓ **R2-4** — 分层模块地图里的「层」怎么来？

- A 目录/命名语义 + 依赖方向校正；识别不了的模块进入 `unclassified`，不硬猜。
- B 纯依赖方向自动分层（L0…Ln），不命名。
- C 必须由用户维护 layers 配置。

为什么必须你回答：「层」是产品核心术语；A 可能误判，B 抽象但客观，C 准确但给第一次使用者增加配置负担。

➡️ 推荐 **A**。层名来自可验证的目录语义，冲突时以依赖方向为准；`unclassified` 必须在图上可见，不能悄悄塞进某一层。

---

### R2-5 — CLI 与索引产物的形态

❓ **R2-5** — CLI 和 GUI 怎么分工，索引落什么格式？

- A 双二进制：`arch-cli`（headless）+ `arch-view`（GPUI），共享 core crate；产物为版本化 JSON（如 `.arch/index.json`），内存建图。
- B 单二进制 + 子命令，JSON 产物。
- C 双二进制 + SQLite 单文件 + 增量索引。
- D 只有 GUI，不提供 CLI。

为什么必须你回答：决定 CI 可用性、测试方式、以及 GUI 是否被桌面依赖绑死。

➡️ 推荐 **A**。≤20k 符号可全量放内存，JSON 便于 golden 断言与人工检查；SQLite/增量属于下一阶段。

---

### R2-6 — 跨平台 MVP 范围

❓ **R2-6** — 第一版承诺哪些平台？

- A Windows 为主验收（开发机就是 Windows），macOS/Linux 只做编译 + 启动冒烟。
- B 三平台同等验收。
- C 仅 Windows。

为什么必须你回答：GPUI 三平台后端都有，但成熟度不同；B 的 QA/CI 成本高，C 会限制分发。

➡️ 推荐 **A**。风险要写明：GPUI 的 Windows 后端尚未在本机验证；第一个技术 ticket 应先跑通最小窗口与 `render_to_image`，失败则回到本问题重选。

---

### R2-7 — 动画与交互规范

> 状态：讨论中，具体方案、GPUI 实现代价与组合见 `docs/grill-round-2-discussion.md` 第 B 节；未拍板。

❓ **R2-7** — 动画做到什么程度？

- A 只有「路径播放」：300–600ms、ease-out，可暂停/跳过/关闭；无空闲动画；遵循 reduced motion；60fps 目标，p95 帧时间 ≤16.7ms。
- B 多种动画：节点入场、边生长、镜头飞行等。
- C 不做动画，只做静态高亮。

为什么必须你回答：Q5 已选路径动画，但动画是理解工具还是装饰，决定实现量与验收口径。

➡️ 推荐 **A**。动画必须能一键关闭，关闭后信息不丢失。

---

### R2-8 — Demo 仓库与验收场景

> 状态：讨论中，样本仓定义、单仓漏项与实测候选见 `docs/grill-round-2-discussion.md` 第 A 节；未拍板。

❓ **R2-8** — 用哪些仓库做可复现验收？

- A 两个公开仓库（一个小型 TS CLI + 一个中型 React 应用），钉死 commit，写 golden 断言；候选由我先实测筛选（文件数、许可、可解析性），你拍板。
- B 只用一个中型仓库。
- C 用你的私有仓库（不进 CI）。

为什么必须你回答：没有固定样本，所有解析/性能读数都不可复现；私有仓库又无法进 CI。

➡️ 推荐 **A**。候选仓库的 star/文件数/许可由我实测后给出，不能凭印象选。

---

## 3. 验收标准制度（用户新增硬性要求）

适用于后续 `to-spec` 与 `to-tickets`：**每张 ticket 自带定制验收标准**，必须写清「命令 + 输入样本 + 期望读数/行为 + 失败判据 + 自动化状态」，禁止「功能正常」「体验流畅」这类描述。下表只是口径分类，不是可直接套用的模板；每张 ticket 仍要按自身性质重写。

| 工作性质 | 判据 | 命令 / 工具 | 期望读数或行为 | 失败判据 | 自动化状态 |
| --- | --- | --- | --- | --- | --- |
| 模块解析 | 固定样本仓的解析边集合 | `cargo test -p arch-core --test resolution_golden` | 指定边存在、`unresolved` 数量符合基线、无多余边 | 缺边/多边/计数漂移 | 可自动化 |
| 调用图构建 | 符号与调用边 golden | `cargo test -p arch-core --test callgraph_golden` | 指定调用链存在、节点/边计数等于基线 | 任一断言 diff | 可自动化 |
| 层推断 | 已知层的样本模块 | `cargo test -p arch-core --test layers_golden` | 层归属正确、`unclassified` 列表等于基线 | 误分层或漏列 | 可自动化 |
| 索引性能 | 冷启动墙钟 + 峰值内存 | `cargo run --release -p arch-cli -- index <fixture>`；内存用平台工具采集 | 墙钟 ≤10s；峰值 RSS 上限按 fixture 定 | 超时或超内存 | 墙钟可自动化；峰值内存跨平台采集未搭好 |
| CLI 契约 | 退出码 + stdout JSON schema | `assert_cmd` + `serde_json` schema 校验 | 成功=0，输入错误=2，内部错误=3；stdout 为合法 JSON | 退出码不符或 schema 不符 | 可自动化 |
| UI 渲染 | 离屏截图 + 布局结构 | `Window::render_to_image` + golden PNG/布局边界断言 | 节点/边数量与包围盒符合基线；像素差在阈值内 | 结构不符或像素差超阈值 | 方法存在，未在本机/CI 验证 |
| UI 交互 | 模拟点击/滚轮后的状态 | `gpui::test` 的 `run_test`/`TestAppContext` | 选中节点、缩放级别、路径高亮集合符合预期 | 状态不符 | 可自动化（待验证） |
| 动画 | 帧时间 + 关闭开关 | 在 render 路径埋点 `Instant`，测试内统计 | p95 ≤16.7ms；关闭开关后画面不再变化 | 超帧预算或开关无效 | 部分可自动化（依赖硬件） |
| 端到端上手 | 脚本化会话的路径产物 | CLI 输出路径 JSON，按入口/目标断言 | 路径包含指定符号与层，顺序正确 | 缺节点/顺序错 | 可自动化（GUI 部分除外） |
| 跨平台 | 编译 + 启动冒烟 | CI matrix：`cargo build` + 启动 3 秒退出 | 三平台编译通过、窗口能起 | 编译失败或启动崩溃 | 部分可自动化 |

当前**无法自动验证**的项及原因：

1. **峰值内存跨平台采集**：Windows 没有 Unix `time -v`；需要 Job Object/ETW 或专用探针，尚未搭建。
2. **GPU 截图的 CI 稳定性**：`render_to_image` API 存在，但未在本机/CI 跑过；无头环境与 GPU 差异可能导致像素不稳定。
3. **动画流畅度**：依赖本机硬件与驱动，只能记录本机 p95，不能作为 CI 硬判据。
4. **「用户能理解」**：onboarding 的主观目标需要人工可用性测试，不能自动判定。
5. **跨平台视觉一致性**：需要各平台基线与硬件，当前没有。
6. **GPUI 升级回归**：尚无截图基线，pre-1.0 破坏性变更只能靠编译 + 冒烟先兜底。

> 以上 6 项的逐项后果、替代口径、接受风险边界、成本与时机见 `docs/acceptance-gaps.md`。

---

## 4. 延后到 Round 3+ 的问题

1. 图布局算法：分层布局 vs 力导向 vs 混合（依赖 R2-1、R2-4）。
2. 符号 ID 规范：重载、匿名函数、箭头函数、默认导出（依赖 R2-1）。
3. 数据流语义边界（Q3 已定延后；具体到跨函数/字段级再问）。
4. 增量索引与 watch 模式（依赖 R2-5）。
5. 语言扩展接口（依赖 Q4 与 R2-2）。
6. 发布与许可：开源协议、二进制分发方式（依赖 R2-6）。
7. 可访问性与国际化（GPUI 下成本未评估）。
8. 具体性能预算：内存上限、样本仓规模（依赖 R2-8）。
9. 遥测与崩溃上报：默认关闭还是不做（隐私取向，需裁决）。

---

## 4.5 Round 2 收口（2026-11-07）

| 题 | 定稿 | 标注 |
| --- | --- | --- |
| R2-1 调用图粒度 | C 双层：符号级真相 + 模块级聚合 | 按推荐定稿，用户未提异议 |
| R2-2 解析边界 | A 相对导入 + tsconfig paths + workspace + 外部包级 + 动态 import 尽力，无法解析显式 `unresolved` | 按推荐定稿 |
| R2-3 入口/目标 | A 自动候选 + 用户选择；框架路由后置 | 按推荐定稿 |
| R2-4 层定义 | A 目录语义 + 依赖方向校正；无法识别进 `unclassified` 且必须可见 | 按推荐定稿 |
| R2-5 CLI 与产物 | A 双二进制 + 共享 core + 版本化 JSON；全量内存图 | 按推荐定稿 |
| R2-6 跨平台 | A Windows 主验收，macOS/Linux 编译 + 启动冒烟 | 按推荐定稿 |
| R2-7 动画 | G1：路径沿边播放 + 逐步追光 + 聚焦淡出 + 可选 fly-to；G2 仅首屏一次且默认关；G3 延后；G4 不做 | **依据讨论稿倾向，未逐项确认** |
| R2-8 样本仓 | 组合 1：`privatenumber/tsx` + `excalidraw/excalidraw`，钉 commit | **依据讨论稿倾向，未逐项确认** |

新增范围：基于 git 历史的演化展示，已纳入 spec，作为独立能力；动画细节与探索式交互只占位，不做设计定稿。

6 个待定参数（ticket 中显式标注「待用户确认」，以下只是推荐默认值）：

| 参数 | 推荐默认值 | 状态 |
| --- | --- | --- |
| 内存预算 | 512MB 硬上限 | 待用户确认 |
| 截图门禁 | 结构断言必须；像素差门禁先关闭 | 待用户确认 |
| 帧预算 + 默认开关 | p95 ≤16.7ms；动画默认关闭 | 待用户确认 |
| 可用性测试阈值 | 5 人、每个任务 ≥4/5 成功 | 待用户确认 |
| 平台矩阵 | Windows 主验收；macOS/Linux 编译 + 启动冒烟 | 待用户确认 |
| GPUI 升级节奏 | 按需升级、单独 PR、`--locked`、禁自动依赖更新 | 待用户确认 |

---

## 5. 状态声明

- 未写代码，未初始化 git。
- 已落盘：`GLOSSARY.md`（3 个已裁决术语）、`docs/adr/0001-rust-core-gpui.md`、`docs/adr/0002-desktop-only.md`。
- 已落盘（2026-11-07）：`docs/spec-architecture-tool.md`；ticket 清单 `docs/tickets/arch-tool/01`–`13`。
- 未落盘：wayfinder map / destination（未执行 wayfinder，直接由 Round 1/2 收敛到 spec）。
- R2-7 / R2-8 标注「依据讨论稿倾向，未逐项确认」；6 个参数在所有相关 ticket 中标「待用户确认」，不是已定值。
- 本文不替用户拍板任何 Round 2 问题；推荐项仅作为默认答案。
