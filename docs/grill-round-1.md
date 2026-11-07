# 架构解析与可视化工具 — 需求拷问 Round 1

生成时间：2026-11-07
工作区：d:\project\1107（空目录，非 git 仓库）
本文件仅记录调研、安装与问题清单；未写任何产品代码，未做架构定稿。

---

## 1. 来源定位

### 候选仓库

| 仓库 | 作者 | 判断 |
| --- | --- | --- |
| https://github.com/mattpocock/skills | Matt Pocock（Total TypeScript 作者） | 唯一命中的作者原始仓库；README 自述 "Skills for Real Engineers. Straight from my .agents directory." |
| git.kuksa.dev/edkuksa/skills | edkuksa | 第三方镜像/同步，非作者原始仓库，不作为安装来源 |
| skillrepo.dev / skill-repo.io / openagentskill.com | 第三方索引站 | 只做索引，不作为安装来源 |

检索命令与结果：WebSearch「Matt Pocock agent skills repo "grill-with-docs"」命中 mattpocock/skills；「github "grill-with-docs" skill」命中的 raw 文件均指向 mattpocock/skills 或他人 fork。未发现第二个以 Matt 为名、且包含 grill-with-docs 的原始仓库。

### 仓库元数据（GitHub API 实测）

命令：`Invoke-RestMethod https://api.github.com/repos/mattpocock/skills`

| 字段 | 值 |
| --- | --- |
| full_name | mattpocock/skills |
| html_url | https://github.com/mattpocock/skills |
| description | Skills for Real Engineers. Straight from my .agents directory. |
| stargazers_count | 314099 |
| forks_count | 26340 |
| open_issues_count | 214 |
| created_at | 2026-02-03T11:15:53Z |
| updated_at | 2026-11-05T11:27:56Z |
| pushed_at | 2026-11-05T10:40:01Z |
| default_branch | main |
| license | MIT |

> star/fork 数为 API 在 2026-11-07 的返回快照，未做二次人工核对。

---

## 2. 名称校验

用户输入来自记忆/口述，实际 skill 清单以仓库 `skills/*/SKILL.md` 为准。

| 用户输入 | 实际名称 | 结论 |
| --- | --- | --- |
| grill——withdoec | `grill-with-docs` | 存在；拼写错误 |
| wayfinder | `wayfinder` | 存在；名称完全一致 |
| tospec | `to-spec` | 存在；少一个连字符 |

校验命令：

```powershell
$t = Invoke-RestMethod "https://api.github.com/repos/mattpocock/skills/git/trees/main?recursive=1"
$t.tree | Where-Object { $_.path -like 'skills/*/SKILL.md' } | Select-Object -ExpandProperty path
```

实测共 39 个 `SKILL.md`；`npx skills add ... -l -y` 发现 38 个可安装 skill（`skills/in-progress/software-factory` 因 YAML 解析错误被跳过，不是名称问题）。

---

## 3. 安装记录

### 官方安装方式

README「Installation」对「Any other agent, or editable files」给出的命令：

```bash
npx skills@latest add mattpocock/skills -a <agent>
```

用 `npx -y skills@latest add mattpocock/skills -a __probe__ -y` 探测到 CLI 支持 `trae-cn`；CLI 自带文档 `node_modules/skills/docs/agents.md` 第 69 行明确：

| Agent | `-a` 名称 | 项目级目录 | 全局目录 |
| --- | --- | --- | --- |
| Trae CN | `trae-cn` | `.trae/skills/` | `~/.trae-cn/skills/` |

因此目标目录不是猜测：项目级 = `d:\project\1107\.trae\skills\`。

### 实际执行与卡点

先按官方 CLI 安装：

```powershell
npx -y skills@latest add mattpocock/skills -a trae-cn -s '*' -y --json
```

失败，原样输出：

```text
Failed to clone https://github.com/mattpocock/skills.git
fatal: unable to access 'https://github.com/mattpocock/skills.git/':
Failed to connect to github.com port 443 after 21117 ms: Could not connect to server
```

连通性旁证：

```text
Test-NetConnection github.com -Port 443 -InformationLevel Quiet
警告: TCP connect to (20.205.243.166 : 443) failed
False
```

同一环境下 `api.github.com` 与 `raw.githubusercontent.com` 可访问（README 与仓库元数据均由这两个域名取回）。沙箱关闭后重试仍失败，因此不是沙箱权限问题，是本机到 github.com:443 的链路问题。

### 回退安装方式（手动放置目录）

按 README「editable files / manual updates」语义，用 GitHub API 归档包落地，等价于 CLI 克隆后复制：

```powershell
Invoke-WebRequest 'https://api.github.com/repos/mattpocock/skills/zipball/main' -OutFile $env:TEMP\matt-skills-main.zip
Expand-Archive ...   # 解压得到 mattpocock-skills-4ae2317
Copy-Item "$root\skills\engineering\*"     "d:\project\1107\.trae\skills" -Recurse
Copy-Item "$root\skills\productivity\*"    "d:\project\1107\.trae\skills" -Recurse
```

选择只装 `engineering + productivity` 两组，依据是 README 对 Gemini CLI 的官方安装命令也只安装这两条路径；`in-progress`（8 个）和 `misc`（4 个）未安装。`deprecated` 目录未安装。

### 安装结果（实测）

```text
installed_dirs=27
grill_with_docs_exists=True
wayfinder_exists=True
to_spec_exists=True
grill-with-docs/SKILL.md SHA256 = 7DE372C13488F1EE96CC11CD8907B56B6809CC93EEF776EEDDD37DE6B6CBE3FE
```

27 个目录：

```text
ask-matt, code-review, codebase-design, diagnosing-bugs, domain-modeling,
grill-me, grill-with-docs, grilling, handoff, implement, implement-spec,
improve-codebase-architecture, pr, prototype, research, retro,
setup-matt-pocock-skills, tdd, teach, to-questionnaire, to-spec, to-tickets,
triage, wait-what, wayfinder, wizard, writing-for-agents
```

影响面：

- 只新增 `d:\project\1107\.trae\skills\`，未覆盖同名目录。
- 未写入全局 `C:\Users\18712\.trae-cn\skills\`；现有全局 skill（ego-browser、find-skills、hypit、poteto-mode）未被改动。
- 安装不需要管理员权限，未写系统目录。

已知限制：

- 本会话启动时 skill 注册表已加载，新装入的项目级 skill 可能要重开会话/重载后才出现在 `/` 列表；本报告与后续拷问是直接读取并遵循 `.trae/skills/*/SKILL.md` 文本执行的。
- `github.com:443` 不可达期间，`npx skills update` 同样会失败；更新只能继续走 `api.github.com` 归档包手动覆盖。

---

## 4. 链路：从模糊想法到分工开发

依据各 skill 的 `SKILL.md` 原文（不是 README 摘要）整理。

### 4.1 顺序与产物

| # | Skill | 输入 | 产出物 | 是否需要用户拍板 |
| --- | --- | --- | --- | --- |
| 0 | `setup-matt-pocock-skills` | 当前仓库状态 | `docs/agents/issue-tracker.md`、`docs/agents/domain.md`（`triage` 已装时还有 `triage-labels.md`）；`AGENTS.md`/`CLAUDE.md` 的 `## Agent skills` 块 | 是：issue tracker 选型；是否沿用默认 triage 标签；多 context 布局（本仓库非 monorepo，默认单 context 不必问） |
| 1 | `grill-with-docs` | 一句话想法 + 工作区 | 达成共识的决策清单；`GLOSSARY.md`；按三条标准才写的 ADR（`docs/adr/`） | 是：每一轮拷问中的所有产品决策 |
| 1b | `grilling` + `domain-modeling` | 同上 | 被 `grill-with-docs` 内部调用；前者负责设计树/轮次，后者负责术语与 ADR 落盘 | 是（继承第 1 步） |
| 1c | `prototype` / `handoff` | 必须跑起来才能回答的问题 | 一次性原型 + 手递文档；原型作为 primary source 保留在 `prototype/<name>` 分支 | 是：选哪个问题去原型、是否接受原型结论 |
| 2 | `wayfinder` | 超出单会话容量的模糊想法 | `wayfinder:map` 映射 issue + 子决策 ticket（research/prototype/grilling/task）+ 原生 blocking 边；关闭 ticket 时写 resolution 并回填 map | 是：命名 destination；每个 HITL ticket 的裁决；scope 边界 |
| 3 | `to-spec` | 已澄清的对话 / wayfinder map | spec（Problem / Solution / User Stories / Implementation Decisions / Testing Decisions / Out of Scope / Further Notes），发布到 tracker 并打 `ready-for-agent` | 是：测试 seam 是否与用户预期一致；spec 发布前确认 |
| 4 | `to-tickets` | spec 或计划 | 可独立验证的纵向切片 ticket，每个声明 blocking edges；本地 tracker 写 `.scratch/<feature>/issues/<NN>-<slug>.md` | 是：粒度、blocking 边是否正确、是否拆分/合并 |
| 5 | `implement` | 单个 ticket | 代码 + 测试 + 提交；内部驱动 `tdd`，收尾 `code-review` | 一般否；ticket 指代不清时需澄清 |
| 5b | `implement-spec` | spec + 全部 tickets | 一条 integration 分支；按任务图并行 implementer subagent、merger subagent；收尾 `code-review` | 可选：是否开 draft PR、是否用并行实现 |
| 6 | `pr` | 变更 diff | PR 正文：最小可视化、before/after 证据、单向门/双向门与爆炸半径 | 是：合并决断 |
| 7 | `retro` | 本次会话 | 对 agent 环境（导航、检查、规范、steering、工具）的改进建议，按严重度排序 | 是：采纳哪些改进 |

横向支线（不在主链上，可插入）：

- `research`：后台 agent 查一手资料，产出带引用的 Markdown；结果喂回 `grill-with-docs`。
- `to-questionnaire`：问题卡在别人脑子里时，产出问卷。
- `triage`：处理外部来的 issue/PR，产出 agent-ready issue，再交给 `implement`。
- `improve-codebase-architecture`：定期扫描可加深模块的机会，选一个后回到 `grill-with-docs`。
- `ask-matt`：路由 skill，不确定走哪条时先问它。

### 4.2 本项目建议路径

```text
setup（先定 tracker，需 git init）
  → grill-with-docs（多轮，产出 GLOSSARY + 关键 ADR）
  → wayfinder（这是典型的"超大且多雾"greenfield，需 map + 决策 ticket）
  → to-spec（把 map 的决策收敛成可构建规格）
  → to-tickets（纵向切片 + blocking 边）
  → implement / implement-spec（tdd + code-review）
  → pr → retro
```

说明：`ask-matt` 对 wayfinder 的适用条件写的是「huge, foggy effort: a greenfield project or a huge feature build, too big for one session」「wayfinder 只产出决策，不产出交付物」；本项目符合前半句，但**是否真的需要 wayfinder，要在 Round 1/2 的问询后由你拍板**，不是我现在替你定。

### 4.3 顺序约束

- 第 1–4 步应在同一个未压缩上下文里完成，直到 `to-tickets` 之后再清空/压缩。
- `wayfinder` 单会话最多解决一个 ticket（research 类型除外）。
- `setup` 每仓库只跑一次；换 tracker 才需要重跑。
- `implement` 每个 ticket 一个全新上下文；`implement-spec` 每个子 agent 一个全新上下文。

---

## 5. Grill Round 1（本轮 frontier）

规则：每轮问题只包含「前置决策已定、现在就能问」的节点；依赖本轮答案的问题留到 Round 2。以下每题均给出推荐答案，回答「是」即接受推荐。

### Q1 — 目标用户与首要场景

- **在问什么**：这个工具首先服务谁、解决他们的哪件事。
- **为什么必须你回答**：它决定「有用」的判据。onboarding 优化的是引导路径与解释；治理优化的是规则、指标、CI；教学优化的是叙事与动画。
- **选项含义**：
  - A 个人开发者第一次读陌生仓库（onboarding）
  - B 团队架构治理/评审（循环依赖、分层违规、影响面）
  - C 教学/演示（讲清架构故事）
  - D 三者都要
- **推荐**：A。第一版只把「陌生人 30 分钟内看懂一个仓库」做扎实；B/C 作为后续视图，不并列进 MVP。
- **代价**：选 D 会让 MVP 的验收标准无法同时收敛。

### Q2 — 交付形态

- **在问什么**：用户怎么拿到并运行这个工具。
- **为什么必须你回答**：决定安装摩擦、是否需要上传代码、UI 能力边界与发布方式。
- **选项含义**：
  - A 本地 CLI + 本地 Web UI：一条命令索引，浏览器打开可交互图
  - B IDE 扩展（VS Code / Trae）
  - C 纯 Web 服务：上传仓库或填 Git URL
  - D 桌面 App
- **推荐**：A。本地优先，零上传，最容易做动画与交互，也最符合开发者对代码隐私的预期。
- **代价**：B 受 IDE API 限制；C 直接引入隐私与服务端成本；D 分发成本最高。

### Q3 — 解析深度（MVP 边界）

- **在问什么**：第一版解析到哪一层：依赖、调用、数据流。
- **为什么必须你回答**：数据流的静态精度天然低（动态派发、反射、DI、闭包），实现成本却最高；它决定 IR 设计与可视化承诺。
- **选项含义**：
  - A 依赖图 + 调用图
  - B A + 数据流（赋值、参数、返回值传播）
  - C A + B + 运行时 trace
- **推荐**：A。数据流放第二阶段，并且只承诺「可解释的近似」，不承诺完备。
- **代价**：选 B/C 需要先定义数据流的语义边界（跨函数？跨文件？字段级？），否则无法验收。

### Q4 — 首批支持语言

- **在问什么**：第一版解析哪些语言。
- **为什么必须你回答**：解析器成本、语义精度、demo 仓库选择都由它决定。
- **选项含义**：
  - A 仅 TS/JS
  - B TS/JS + Python
  - C 多语言（tree-sitter / LSP）
  - D 其他组合（请指定）
- **推荐**：A。TS/JS 生态的模块解析规则明确，容易做出可信 demo；IR 预留语言无关层，后续再扩。
- **代价**：选 C 会把 MVP 变成「解析器工程」，可视化被推迟。

### Q5 — 主视图与动画的角色

- **在问什么**：用户第一眼看到什么；动画是装饰还是理解工具。
- **为什么必须你回答**：这是产品的核心表达；不同选择对应完全不同的数据结构与交互。
- **选项含义**：
  - A 分层模块地图为主视图，点击下钻调用图，选中路径播放调用/数据流动画
  - B 力导向全局图（节点+边，可拖拽）
  - C 以时序/调用链动画为主
  - D 3D 场景
- **推荐**：A。分层图回答「系统由什么组成」，下钻回答「具体怎么跑」，路径动画回答「为什么走到这里」。
- **代价**：B 在稍大规模下会变成「毛线球」，难以支撑 onboarding 叙事；D 可读性与实现成本都最差。

### Q6 — 规模与性能目标

- **在问什么**：第一版要能吃多大的仓库，索引时间预算多少。
- **为什么必须你回答**：决定存储（内存图 / SQLite / 图数据库）、是否需要增量索引、以及可视化降采样策略。
- **选项含义**：
  - A 中小仓库：≤5k 文件 / ≤20k 符号，冷索引 <10s
  - B 中型：≤20k 文件
  - C monorepo：100k+ 文件
- **推荐**：A。先证明交互与解释有效，再为规模换存储。
- **代价**：选 C 会迫使先解决增量索引、持久化与图查询，产品验证被推后。

### Q7 — 差异化与「上手闭环」

- **在问什么**：和 dependency-cruiser / madge / Sourcetrail / Sourcegraph 相比，用户为什么用你这个。
- **为什么必须你回答**：避免做出「又一个依赖图画布」；它定义 MVP 的成功判据。
- **选项含义**：
  - A 引导式路径：选入口与目标，工具解释中间层次、调用链与数据流向，并给出可读注解
  - B 自由探索图：以画布浏览为主
  - C 架构规则与 CI 门禁
  - D A + B
- **推荐**：A，B 作为辅助浏览。成功判据建议写成「给定陌生仓库与一个目标函数，用户能在 N 分钟内说清调用路径与所属模块」。
- **代价**：选 B 会与现有工具正面竞争且无差异；选 C 是另一个产品。

### Q8 — 技术栈约束

- **在问什么**：实现语言与运行时。
- **为什么必须你回答**：影响解析器生态、索引性能、你本人的维护成本与面试/演示价值。
- **选项含义**：
  - A Rust core（解析/索引/图算法）+ TS/React UI
  - B Node/TS 全栈
  - C Python 后端 + TS UI
  - D 其他
- **推荐**：A。你的既有栈是 Rust/Go，Rust 侧可用 tree-sitter/swc 做解析与图计算，UI 侧做动画更顺手。
- **代价**：B 上手最快但大仓库索引与并发弱；C 解析生态好但分发与性能一般。

### 本轮不提问（依赖上面的答案，留到 Round 2）

1. 数据流的具体语义边界（依赖 Q3）
2. 图存储选型：内存 / SQLite / DuckDB / 图数据库（依赖 Q6、Q8）
3. 增量索引与缓存失效策略（依赖 Q6）
4. 多仓库/跨服务支持（依赖 Q1、Q6）
5. 语言扩展接口与插件机制（依赖 Q4、Q8）
6. 动画规范：帧率、降级、可关闭（依赖 Q5）
7. MVP 验收用的具体开源仓库与基准（依赖 Q1、Q4、Q7）
8. 发布与许可：开源 / 内部工具 / 商业（依赖 Q2）

---

## 5.1 Round 1 裁决结果（2026-11-07）

用户答复：Q1–Q7 全部接受推荐；Q8 改为「Rust core + GPUI」。

| # | 裁决 | 状态 |
| --- | --- | --- |
| Q1 | 目标用户 = 个人开发者第一次读陌生仓库（onboarding） | 已定 |
| Q2 | 交付形态 | 定稿：纯桌面端（CLI + GPUI），取消 Web UI 与可交互 HTML 导出（ADR-0002） |
| Q3 | MVP 解析深度 = 依赖图 + 调用图；数据流延后 | 已定 |
| Q4 | 首批语言 = 仅 TS/JS | 已定 |
| Q5 | 主视图 = 分层模块地图 + 下钻调用图 + 路径动画 | 已定 |
| Q6 | 规模目标 = ≤5k 文件 / ≤20k 符号，冷索引 <10s | 已定 |
| Q7 | 差异化 = 引导式路径（自由浏览为辅助），目标为 onboarding 闭环 | 已定 |
| Q8 | 技术栈 = Rust core + GPUI（用户修改推荐项） | 已定（ADR-0001） |

冲突说明：GPUI 是原生桌面渲染框架，无法同时交付「本地 Web UI」。该冲突已由用户确认解决：**纯桌面端（Rust core + CLI + GPUI），取消 Web UI，不做可交互 HTML 导出**，见 `docs/adr/0002-desktop-only.md`。

---

## 6. 环境阻塞与待确认

1. **工作区不是 git 仓库**（`git rev-parse` 退出码 128）。`setup-matt-pocock-skills` 若选 GitHub tracker，需要先 `git init` 并建远端；若选 local markdown tracker，则写 `.scratch/`，不强制 git。
2. `gh` 已安装且已登录 `cs1lion`（scopes 含 `repo`）；GitHub tracker 在 API 侧可用，但 `git clone/push` 依赖 `github.com:443`，当前不可达（SSH 协议是否可用未验证）。
3. 本会话 skill 注册表可能未加载新装项目级 skill；`/grill-with-docs` 可能需要重开会话。内容已落盘并已在本次按原文执行。
4. 尚未选择 demo 仓库，因此无法给出任何解析性能/覆盖率实测数字。

---

## 7. 状态声明

- 未写代码。
- 已创建 `GLOSSARY.md`（仅收录 Round 1 已裁决的 3 个术语）与 `docs/adr/0001-rust-core-gpui.md`（技术栈裁决）；模块/层/调用边等未裁决术语未写入。
- 未创建 wayfinder map / ticket（destination 未由用户命名）。
- 未初始化 git，未创建远端仓库。
