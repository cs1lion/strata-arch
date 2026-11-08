# Strata 演示动画研究

研究问题：动画在「帮开发者读懂陌生 TS/JS 仓库结构」这件事上，什么时候真的有用、什么时候只是好看；Strata 应该做哪几种。

证据标签：

- **【事实·本地】** 来自本仓库文件、测试归档或可复现命令，附文件路径与行号。
- **【事实·外部】** 来自公开文档、论文或产品说明，附链接。
- **【推断】** 我的判断，未在本项目实测。
- **【未验证】** 仓库中查不到、或无法在不新增实验的前提下证明。

边界：本轮只读仓库，未修改 `crates/`、测试、基线或 ticket；未执行任何 git 写操作；产出只写在本目录。文中所有本地读数都注明来源；外部的效应量、比例、默认值同样注明出处。

---

## 0. 结论先行

### 0.1 一句话结论

**Strata 值得做的是「用户触发、离散、可暂停、表达状态变化」的少量动画；不值得做的是「持续运动、自动循环、用运动装饰静态结构」的动画。** 现有 G1 是合格的**步进播放器骨架**，但还不是合格的演示动画：符号级路径没有画出来，fly-to 没有接到视口，播放仍是 180ms 定时器步进，不是逐帧动画。

### 0.2 可选择的方案菜单

代价口径（相对，不用工时）：

- **S**：只改渲染/状态，不引入新数据路径或新布局。
- **M**：需要新状态机、新画布或新交互，并补结构断言；不改 core 数据契约。
- **L**：需要新采样/缓存/布局算法，或跨 ticket 契约变更。

#### 推荐可做

| ID | 方案 | 动的是什么 | 解决哪个理解问题 | 依赖 ticket | 代价 | 主要风险（尤其误导） | 组合关系 |
|---|---|---|---|---|---|---|---|
| **P0** | 静态基线：不新增动画 | 无 | 精确阅读、低配机器、无障碍、作为 A/B 对照 | 现有 05/07 静态路径 | S | 无；首屏可能缺少引导 | 所有方案的 fallback，必须始终可用 |
| **P1** | G1 完成版：离散路径步进 + 当前边强调 + 上下文淡出 + 模块/符号同步 | 步骤索引推进时切换当前边、淡出非路径元素、面板同步；**无连续位移** | 「我现在在哪一步、下一步去哪、整条链多长」；降低同时看整张图的注意力分裂 | 06/07（模型已有）；需补 05 的符号级画布 | M | 把 BFS 最短调用链读成运行顺序；符号级边种类丢失；自动播放抢注意力 | 与 P2 可叠加；与 P5 互斥（首屏只允许一个主动效）；与 P3/P4 独立 |
| **P2** | 短镜头转场：150–250ms，仅用户触发，可被 reduced-motion 关闭 | 视口中心/缩放从 A 插值到 B；不是节点运动 | 下钻/跳转后的方向感与 object constancy，减少「我迷路了」 | 05（viewport/命中框）；07 的 fly-to 状态 | M | 眩晕/方向感更差；转场期间点击错位；长转场拖慢 | P1/P4 的修饰器；P3 播放期间禁用自动镜头；不与 R2 叠加 |
| **P4** | 双 commit 结构 diff：静态高亮 + 可选 120ms 淡入 | 选中一对 commit 后，新增/删除/修改的模块以静态色块/轮廓呈现；不移动节点 | 「这个 commit 到底改了什么、影响哪些模块」；比连续播放更精确 | 08/09；13 的演化交互占位 | M | 重命名/节点身份误读；只显示变化不显示幅度；布局不一致 | P3 的替代默认模式（同一时间线，二选一）；与 P1 独立；与 P2 可叠加 |
| **P3** | 时间线离散播放：按 commit 刻度 dwell，不插值 | 时间轴指针按 commit 前进；热力颜色/尺寸离散切换；可选 150ms cross-fade | 「演化顺序、突变还是渐进、哪个 commit 变热」 | 08/09/10；13 | L | 让用户以为 commit 之间连续变化；跳过 commit/空闲期；变化盲视；大仓卡顿 | 与 P4 互斥为默认模式；与 P2 可叠加但播放中锁镜头；与 P1 独立 |
| **P5** | 首屏分层揭示：一次性、可跳过、默认关 | 层带/模块按层序一次性淡入或升起；不循环 | 首屏「从哪里看起」；建立层级心智模型 | 03/05 | S | 暗示层间执行顺序；延迟首屏可读性；对抽象图收益证据弱 | 与 P1 互斥（同一时段只能一个主动效）；只做首次进入的一次性引导 |

#### 明确不做

| ID | 方案 | 动的是什么 | 为什么不做 | 代价 | 组合关系 |
|---|---|---|---|---|---|
| **R1** | 粒子沿调用边流动 | 粒子沿边循环 | 不增加信息；会被读成数据流/执行流（运动因果知觉），持续抢注意力 | M | 不与任何方案叠加，明确拒绝 |
| **R2** | 节点呼吸/脉动 | 透明度/尺寸循环 | 运动是强 popout，持续抢注意力；不能表达变化量；无障碍风险 | S | 拒绝 |
| **R3** | 默认自动循环播放 | 进入页面自动播放/循环 | 用户来不及读；变化盲视；违反「可暂停/停止」的可用性底线 | S | 拒绝作为默认；只允许用户显式点击播放 |
| **R4** | commit 间连续插值/节点 morph | 尺寸/颜色/位置连续过渡 | 没有真实中间帧，等于伪造精度；布局变化会丢对象身份 | L | 拒绝；用 P4 的静态 diff 替代 |

### 0.3 推荐顺序

**P0（始终保留）→ P1 → P2（可选叠加）→ P4 → P3（等 10 的缓存结论）→ P5（可选）；R1–R4 不做。** 理由：P1 直接命中路径理解且模型已有；P2 用很小代价修补「下钻后迷路」；P4 用静态 diff 先回答「改了什么」，不背 O(n²) 播放成本；P3 必须等 10 的采样成本解决；P5 收益最弱。

### 0.4 判断依据（展开在各节）

1. **动画对抽象图示的总体收益很小，且有条件。** 两项 meta-analysis 的效应量分别只有 d=0.37 和 g=0.226；抽象表示（图表、图、地图）收益低于图像式表示；优势主要出现在「动画额外传达了微步骤」或「学习者必须动手」时。
2. **连续运动会带来三类稳定风险。** 变化盲视、注意力被运动捕获、运动被知觉为因果。它们都不是「审美问题」，会直接造成误读。
3. **本项目的演化播放被成本卡住。** 09 的 6 个刻度墙钟 4435ms，根因是每个刻度重跑前缀 `build_history`（O(n²)）；在缓存/增量采样落地前，P3 不能承诺可用。
4. **工业界成熟工具大多不做连续动画。** Gource 是演示工具；CodeCharta 用静态 Compare；CodeBoarding 用离散 commit 高亮；dependency-cruiser/madge 输出静态图。Strata 的差异化应落在「路径理解 + 可检查的变更」，不是「把静态图动起来」。

---

## 1. 现有工具怎么做动画

### 1.1 总表

| 工具 | 动的是什么 | 表达什么信息 | 要解决的理解问题 | 证据 |
|---|---|---|---|---|
| Gource | 时间轴自动推进；目录树枝生长/文件叶出现；开发者头像移动到文件；镜头 overview/track 跟随 | 谁在何时改了哪些路径；活动密度与协作 | 宏观历史节奏、热点路径；**不是**调用关系或数据流 | [gource.io](https://gource.io/)、[Controls wiki](https://github.com/acaudwell/Gource/wiki/Controls) |
| CodeCharta | 基本无时间动画；静态 3D code city；Compare 用两套 map 的 delta 着色 | 面积/高度/颜色编码指标；delta 颜色表达增长/缩减 | 「现在哪里大/复杂/热」和「两个版本差在哪」 | [Views](https://codecharta.com/docs/visualization/user-controls/views/)、[Compare](https://codecharta.com/docs/visualization/user-controls/compare/) |
| pomber/git-history | 文件级 commit 序列的 step-through；diff 行高亮切换 | 某文件随时间的具体行变化；commit 顺序 | 「这行代码什么时候、为什么变」 | [README](https://github.com/pomber/git-history)、[产品说明](https://ross.abutalabs.com/products/git-history) |
| dependency-cruiser | 无；输出 DOT/SVG/JSON 等静态格式 | 模块依赖、循环依赖、规则违规 | 「谁依赖谁、有没有环」 | [README 示例](https://gitlab.com/sverweij/dependency-cruiser/-/blob/develop/README.md) |
| madge | 无；输出 SVG/DOT | 模块依赖图、循环依赖 | 同上 | [README](https://github.com/pahen/madge) |
| CodeBoarding | 时间线滑块 step through commits；diagram 的组件/文件/方法「light up」变更 | commit 级变更归属；架构图与历史对齐 | 「这个 commit 改了哪些组件」 | [Open VSX 说明](https://open-vsx.org/extension/CodeBoarding/codeboarding) |

### 1.2 Gource：时间驱动的「生长树」

**【事实·外部】** Gource 把仓库显示为以根目录为中心的动画树：目录是树枝，文件是叶子，开发者出现在他们贡献的文件处；时间按 `--seconds-per-day` 推进，默认每天 10 秒。它有两种镜头模式：`overview` 保持整个仓库可见，`track` 跟随当前活跃用户或选中用户；默认 `overview`。时间线可以 seek、可以暂停（空格），暂停后仍可移动镜头检查文件与用户。它还会自动跳过无聊区间：默认 3 秒没有活动就前进到下一个条目；默认 60 秒未被触碰的文件会「过期」消失；文件更新前用户会先移动到新文件区域（`--max-file-lag` 默认 5 秒）。[gource.io](https://gource.io/)、[Controls wiki](https://github.com/acaudwell/Gource/wiki/Controls)

**【事实·外部】** 第三方评测把 Gource 定性为「历史演示工具，不是分析工具」，并建议需要 3D 加速显卡。[hysenlabs 评测](https://hysenlabs.com/projects/acaudwell-gource)

**【事实·外部】** Armenti 与 Lanza 在软件演化动画论文中直接批评 Gource：它把 Git 仓库活动可视化得动画化且有美感，但**粒度粗**，且**动画不允许交互和检查正在显示的变更**。[Arme 2024](https://www.inf.usi.ch/faculty/lanza/PUBS/P/Arme2024b.pdf)

**可迁移的设计**：时间压缩（auto-skip）、暂停/seek、镜头模式可选。**不可迁移的设计**：自动镜头 + 持续运动作为默认分析界面。

### 1.3 CodeCharta：静态 code city + 静态 Compare

**【事实·外部】** CodeCharta 的官方视图是 Metric（3D code map 或平面布局）、Domain（词云）、Dependencies（按层级排布的依赖图）。Metric 视图有 Explore / Compare / 3D Print 三种模式；Compare 加载两套 map，用绿/红/灰表示高度指标的增减，Inspector 显示每个指标的 delta。官方文档中没有时间轴播放或动画播放控件。[Views](https://codecharta.com/docs/visualization/user-controls/views/)、[Compare](https://codecharta.com/docs/visualization/user-controls/compare/)

**【未验证】** 不能断言实现层完全没有 CSS 过渡；只能确认官方文档没有把「时间动画」作为功能。就「演化理解」而言，它选择的是**静态并排比较**，不是连续播放。

**可迁移的设计**：用 delta 颜色和 Inspector 数字回答「变在哪、变多少」，比让用户追动画更精确。这直接支持把 P4 排在 P3 前面。

### 1.4 git-history 类：离散 step-through

**【事实·外部】** pomber/git-history 的入口页面展示的是一个动画 GIF；产品说明把它描述为「animated step-through of changes」的 diff 查看器，可按 commit 浏览某个文件的历史。[README](https://github.com/pomber/git-history)、[产品说明](https://ross.abutalabs.com/products/git-history)

**【事实·外部】** Armenti 与 Lanza 指出：commit 的常规表示仍是文本 diff；文本 diff 阅读负担重，而动画可以表示文件版本之间的真实动态。他们的方案把单个 commit 拆成 pre-commit、prologue、corpus、epilogue、post-commit 五个阶段，用淡入/淡出、生长/收缩、颜色激活来表达新增、删除、重命名、修改；整段动画的经验时长为 5–10 秒，大部分时间给 corpus；查看器支持暂停、回退、单步和检查。[Arme 2024](https://www.inf.usi.ch/faculty/lanza/PUBS/P/Arme2024b.pdf)

**可迁移的设计**：离散 step + 可暂停/回退 + 阶段化表达。**关键差别**：它动的是**文件条的长度/颜色**，不是把静态调用边变成粒子流。前者承载了真实数据（行数、变更类型），后者不承载新数据。

### 1.5 依赖图工具：静态输出是常态

**【事实·外部】** dependency-cruiser 的 README 示例是 `depcruise --include-only "^src" --output-type dot src | dot -T svg > dependencygraph.svg`：先生成 DOT，再用 GraphViz 出 SVG。它的核心输出是模块与依赖的静态图或 JSON。[README](https://gitlab.com/sverweij/dependency-cruiser/-/blob/develop/README.md)

**【事实·外部】** madge 的 README 明确 Graphviz 只在需要生成 SVG/DOT 等视觉图时才需要；API 可返回 DOT/SVG，CLI 示例是 `madge --image graph.svg path/src/app.js` 和 `--dot`。[README](https://github.com/pahen/madge)

**【推断】** 依赖图工具的共性是不做时间动画，因为它们的核心任务是**结构查询**（谁依赖谁、有没有环、规则是否违规），静态布局 + 交互筛选已经够用；动画不增加结构信息。

### 1.6 CodeBoarding：commit 级离散高亮

**【事实·外部】** CodeBoarding 的 Git Commit Diff View 描述为：用时间线滑块 step through 最近 commits，diagram 会「light up」以显示每个 commit 改了哪些组件、文件和方法；增量分析后图会重构（例如组件的方法全被删除，组件会从图中移除）。[Open VSX 说明](https://open-vsx.org/extension/CodeBoarding/codeboarding)

**【推断】** 这是「离散高亮 + 图重构」，不是连续运动。它解决的是**变更归属**，与 Strata 的 P4 同构；实现上比 P3 的连续播放便宜，也更不容易触发变化盲视。

### 1.7 小结：值得抄与不值得抄

**值得抄**

1. 时间压缩 + 暂停/seek（Gource）。
2. 离散 step-through + diff 高亮（git-history、CodeBoarding）。
3. 静态并排/delta 比较（CodeCharta）。
4. 短转场保持 object constancy（见 2.2 的 node-link 实验）。

**不值得抄**

1. 自动镜头作为默认分析界面（Gource 的演示定位，不是分析定位）。
2. 粒子沿边流动（没有新增信息，且会被读成数据流）。
3. 持续呼吸/脉动（注意力捕获）。
4. 把 3D 树/城市当作结构理解的主视图（空间隐喻会引入额外解释成本）。

---

## 2. 动画到底帮不帮理解

### 2.1 总体证据：小效应，且有条件

**【事实·外部】** Höffler 与 Leutner 的 meta-analysis：26 项研究、76 个配对比较，动画相对静态图片的加权效应量 **d=0.37（95% CI 0.25–0.49）**，属于小到中等；moderator 分析显示动画在**程序性/运动性知识**上更有效。[Höffler & Leutner 2007](https://wikicap.ulb.be/images/e/ef/Hoffler_Leutner_2007.pdf)

**【事实·外部】** Berney 与 Bétrancourt 的更新 meta-analysis：61 项研究、N=7036、140 个配对比较，总体效应量 **Hedges's g=0.226（95% CI 0.12–0.33）**。两个重要 moderator：抽象表示（analytical pictures、formal notation、symbols、charts、diagrams、graphs、maps）的收益**低于**图像式表示；系统控制节奏的动画在学习结果上**优于**学习者控制节奏的动画。[Berney & Bétrancourt 2016](https://tecfa.unige.ch/perso/sandra/pdf/Earli2016_berney_betrancourt_FINAL.pdf)

**【事实·外部】** Tversky、Morrison 与 Bétrancourt 的综述结论更不乐观：动画相对静态图的研究「并不令人鼓舞」；当动画看似更优时，仔细检查常发现动画**多传达了信息**（尤其是大步骤之间的微步骤）或**包含交互**，而不是「仅仅因为动了」。他们提出 Apprehension（要能被准确感知）和 Expression（要表达对的内容）两条原则。[综述 PDF](https://courses.ischool.berkeley.edu/i247/f05/readings/Morrison_AnimationFacilitateLearning.pdf)

**【事实·外部】** Hundhausen、Douglas 与 Stasko 对 24 个算法可视化实验的 meta-study 发现：**学生如何使用可视化，比可视化展示了什么，对效果影响更大**；操作表征特征的实验，显著学习效果远不如操作学习者活动的实验可靠。[Hundhausen et al. 2002](https://doi.org/10.1006/jvlc.2002.0237)

**对 Strata 的取舍【推断】**：

- 代码架构图是**抽象表示**（节点、边、层、热力），落在 Berney 所说收益较低的一类；不要期待「加上动画就能提升理解」。
- 动画要有效，必须承载**静态图没有的信息**：步骤顺序、commit 之间的变化、当前步骤的入边。只让静态元素动起来，不增加信息。
- 「可交互/可操作」比「自动播放」更接近 Hundhausen 的结论；P1/P4 的步进与选择，比 P3 的自动播放更可能有正收益。
- Berney 的「系统控制节奏优于学习者控制」是在**学习结果**实验里；Strata 是**分析工具**，用户带着具体问题来，控制权仍然必要。这个冲突应通过人测分辨，不能直接套用。

### 2.2 短转场与 object constancy

**【事实·外部】** Shanmugasundaram 的硕士论文做了四个实验，比较 node-link 图在有无平滑视图转场下的表现。结论：平滑转场可以显著改善用户表现，例如平滑条件下的错误数是离散移动条件的**一半**；**短转场与长转场一样有效**，说明可以避免动画的一部分成本。[University of Manitoba](https://hci.cs.umanitoba.ca/publications/details/quantifying-the-effects-of-smooth-view-transitions-on-perceptual-constancy)

**对 Strata 的取舍【推断】**：P2 应做**短**（150–250ms）、**用户触发**、**可取消**的视口转场；不要做电影式长镜头。转场只改变视口，不改变节点位置，因此不会引入「节点在移动」的误读。

### 2.3 变化盲视：动画不会自动让变化被看见

**【事实·外部】** Simons 与 Levin 的经典综述：人并不把每次注视的细节整合成稳定的跨视图表征，因此会漏掉明显变化。[Simons & Levin 1997](https://home.cs.colorado.edu/~mozer/Teaching/syllabi/3702/readings/SimonsLevin1997.pdf)

**【事实·外部】** Nowell 等人的 InfoVis 案例研究把 change blindness 引入信息可视化语境，说明可视化系统中的变化同样会被漏掉。[IEEE Xplore 963274](https://ieeexplore.ieee.org/document/963274)

**【事实·外部】** Fish、Goldsberry 与 Battersby 的动画 choropleth 实验：参与者经常**错误识别**哪些地区发生了变化，而且**对自己的报告持续过度自信**；作者建议用 tweening（帧间渐变）来缓解。[课程页对论文的总结](https://courses.ems.psu.edu/geog486/node/924)、论文 *Change Blindness in Animated Choropleth Maps: An Empirical Study* (2011)

**对 Strata 的取舍【推断】**：

- 不要用「连续运动」来承载「哪个模块变了」。变化必须同时有**静态可回看的痕迹**（旧值 ghost、diff 面板、数字 delta）。
- P4 的静态高亮 + delta 数字，比 P3 的自动播放更抗变化盲视。
- 人测必须测**置信度校准**：用户说「我看到了」但答错，是比答错更危险的失败模式。

### 2.4 运动的显著性与因果误读

**【事实·外部】** Gutwin、Cockburn 与 Coveney 的 CHI 2017 研究：运动是强 popout 刺激，即使强度很低、视角很偏，识别准确率仍然较高；颜色和形状在周边视野中衰减得更快。[Peripheral Popout](http://library.usc.edu.ph/ACM/CHI%202017/1proc/p208.pdf)

**【事实·外部】** Michotte 的 launching effect 及其注册重复实验表明：简单的运动几何图形就能让人**在知觉层面**产生因果印象（一个物体「导致」另一个物体运动）。[注册重复研究](https://pmc.ncbi.nlm.nih.gov/articles/PMC12434928/)

**对 Strata 的取舍【推断】**：

- 任何沿调用边持续运动的粒子，都会被一部分用户读成「数据在流动」或「控制流在传递」，而 Strata 的边只是**静态调用关系**。这不是「用户没看懂」，是运动本身的语义过载。
- 运动可以用来表达**用户刚做的动作**（我点了下一步，当前边亮起），但不能用来表达**结构本身**（这条边一直存在）。
- 这也解释了为什么 R1 即使「好看」也应拒绝：它制造了错误的因果模型，而且很难通过图例纠正。

### 2.5 工程界的收敛

**【事实·外部】** Gource 被定位为演示工具，不是分析工具；Armenti 批评它粒度粗、不能交互检查。

**【事实·外部】** CodeCharta 用静态 Compare；CodeBoarding 用离散 commit 高亮；dependency-cruiser/madge 输出静态图。

**【推断】** 成熟工具的共同选择是：**结构用静态图 + 交互筛选；变化用离散 diff/高亮；连续动画只用于演示或过程教学**。Strata 应把这套分工抄过来，而不是把「所有东西都做成动画」。

---

## 3. 反例与陷阱

| # | 陷阱 | 证据 | 为什么会误导 | 应对方式 |
|---|---|---|---|---|
| T1 | 粒子沿调用边流动被读成「数据流」 | 运动因果知觉（[Michotte 重复实验](https://pmc.ncbi.nlm.nih.gov/articles/PMC12434928/)）；Gource 的边也不是数据流【推断】 | 静态调用边被加上了「有东西在流」的因果语义；用户会把调用关系当成运行时数据/控制流 | 不做 R1；如果未来做真实数据流，必须换视觉通道并明确图例，绝不与调用边共用 |
| T2 | 节点呼吸/脉动抢注意力 | 运动是强 popout（[Gutwin et al. CHI 2017](http://library.usc.edu.ph/ACM/CHI%202017/1proc/p208.pdf)） | 持续运动会把注意力从用户当前任务拉走；呼吸不表达任何可比较的量 | 不做 R2；热力用静态色/尺寸；若必须提示选中，只做 2 个周期的短脉冲并自动停止 |
| T3 | 动画关不掉/默认自动播放 | WCAG 2.2.2：自动开始、持续 >5 秒、与其他内容并行的移动/闪烁/滚动信息，必须可暂停/停止/隐藏；WCAG 2.3.3：交互触发的运动必须可关闭，除非必要 | 用户无法静读；前庭敏感用户可能不适；演示模式侵入分析模式 | 全局「动画」开关；自动播放默认关；尊重系统 reduced-motion；设置持久化；关闭后不调度动画帧 |
| T4 | 自动播放长历史，空闲区间把注意力磨掉 | Gource 默认 3 秒无活动就 auto-skip；默认每天 10 秒；文件 60 秒未触碰会过期（[Controls](https://github.com/acaudwell/Gource/wiki/Controls)） | 长段无变化会让用户走神；等真正变化出现时已经漏看 | 压缩空闲区间；在变化处 dwell；显示 commit 数/日期；允许 pause/seek；默认手动 |
| T5 | 连续插值伪造「中间状态」 | 09 的采样是离散刻度；没有真实中间产物（[09 L49](file:///d:/project/1107/docs/tickets/arch-tool/09-evolution-timeline-heat-ui.md#L49)） | 用户会以为看到了 commit 之间的连续变化，实际只是插值 | 只做离散 tick；UI 明示「sampled at N commits」；不做 R4；缺数据时标 `degraded` |
| T6 | 自动镜头飞入导致迷路/眩晕 | 平滑转场有效但短转场同样有效（[Manitoba](https://hci.cs.umanitoba.ca/publications/details/quantifying-the-effects-of-smooth-view-transitions-on-perceptual-constancy)）；WCAG 2.3.3 | 长镜头让用户失去全局位置；转场中点击命中框错位 | P2 只做 150–250ms、用户触发、可取消；保留 overview inset；播放中锁镜头；reduced-motion 直接切 |
| T7 | 路径步进被读成「执行顺序」 | 06 的路径是 BFS 最短调用链，不是运行时序（[06 L34](file:///d:/project/1107/docs/tickets/arch-tool/06-entry-target-guided-path-query.md#L34)）【事实·本地】 | 动画天生按时间展开，用户会把步骤顺序当成程序执行顺序 | 文案写「静态最短调用链」，不写「执行」；显示每步入边种类与方向修正；不要默认自动播放 |
| T8 | 变化盲视 + 过度自信 | Fish et al. 2011：参与者常错认变化位置且过度自信（[课程页](https://courses.ems.psu.edu/geog486/node/924)） | 动画一闪而过，用户以为看到了；实际记忆的是错的 | 旧值 ghost/onion skin；diff 面板与数字 delta；允许回看；人测测置信度校准 |
| T9 | 演示价值压过分析价值 | Gource 是 presentation not analysis（[评测](https://hysenlabs.com/projects/acaudwell-gource)）；Armenti 批评其不可交互检查 | 团队会为「演示好看」加动画，挤占精确阅读路径 | 静态默认；动画是可选「讲解模式」；任何动画都必须有等价的静态读法 |

---

## 4. 本项目可做的动画清单

### 4.1 现状盘点（事实）

**模型层【事实·本地】**

- `PlaybackState` 已有 `highlighted_nodes()`、`highlighted_edges()`、`current_edge()`、`camera_center()`、`request_frame()`；动画关闭时 `request_frame()` 恒为 false。[playback.rs](file:///d:/project/1107/crates/arch-view/src/playback.rs#L125-L179)
- `node_opacity()` 只按「是否在路径上」返回两个固定不透明度，**不随当前步骤变化**；它也不看 `animation_enabled`。[playback.rs](file:///d:/project/1107/crates/arch-view/src/playback.rs#L159-L166)

**模块视图【事实·本地】**

- 模块场景先画全部边（`strong: false`），再叠加路径边；当前步的入边 `strong = overlay.current == index + 2`，用白线/粗线强调。[main.rs](file:///d:/project/1107/crates/arch-view/src/main.rs#L758-L785)
- 非路径节点按 `node_opacity` 淡出；路径节点保持可见。[main.rs](file:///d:/project/1107/crates/arch-view/src/main.rs#L787-L810)

**符号视图【事实·本地】**

- 符号场景把 `symbols.edges` 全部画成 `strong: false`、`adjusted: false`，**没有当前边强调**；路径只通过 `node_opacity` 影响节点。[main.rs](file:///d:/project/1107/crates/arch-view/src/main.rs#L812-L847)
- 这与 07 的验收描述「符号级追光」不一致：07 自己也标注「UI 路径叠加停在模块级，符号级追光只在数据层」。[07 L49](file:///d:/project/1107/docs/tickets/arch-tool/07-path-playback-stepthrough-ui.md#L49)

**播放与开关【事实·本地】**

- 播放用 `Timer::after(Duration::from_millis(180))` 循环推进，**不是逐帧动画**；`animation_enabled` 不参与这个 ticker 的判断，所以「动画: 关」并不会阻止播放步进。[main.rs](file:///d:/project/1107/crates/arch-view/src/main.rs#L898-L937)、[main.rs](file:///d:/project/1107/crates/arch-view/src/main.rs#L953-L970)
- `fly-to` 开关只改模型标志，`camera_center()` 没有接到 `CanvasState`/viewport；UI 里没有消费 `camera_center`。[main.rs](file:///d:/project/1107/crates/arch-view/src/main.rs#L972-L990)、[playback.rs](file:///d:/project/1107/crates/arch-view/src/playback.rs#L168-L179)
- UI 里没有调用 `request_animation_frame`；07 的「关闭后仍有帧调度」是模型断言（`request_frame` 返回 false），不等于 UI 的 ticker 行为。[07 L17](file:///d:/project/1107/docs/tickets/arch-tool/07-path-playback-stepthrough-ui.md#L17)、[07 L48](file:///d:/project/1107/docs/tickets/arch-tool/07-path-playback-stepthrough-ui.md#L48)

**时间线【事实·本地】**

- `build_timeline_entries` 先跑一次 `git log --reverse --first-parent`，然后**对每个提交**用 `until=@time+1` 再跑一次 `build_history`；这是 O(n²) 前缀采样。[main.rs](file:///d:/project/1107/crates/arch-view/src/main.rs#L245-L293)、[09 L49](file:///d:/project/1107/docs/tickets/arch-tool/09-evolution-timeline-heat-ui.md#L49)
- 时间线状态模型支持 `scrub_to`、`heat`、`leave_timeline`，但布局只建一次；早期刻度里不存在的模块在视图里不会「消失」，只会是中性热力（`heat()` 对缺失模块返回 `HEAT_NEUTRAL`，视图遍历的是 HEAD 布局的全部节点）。[timeline.rs](file:///d:/project/1107/crates/arch-view/src/timeline.rs#L96-L168)、[main.rs](file:///d:/project/1107/crates/arch-view/src/main.rs#L502-L516)

**GPUI 能力【事实·本地】**

- `with_animation`、`request_animation_frame`、`Animation` 在 `gpui-0.2.2` 源码中存在；`Animation` 支持 duration、oneshot、repeat、easing，按墙钟时间推进，并每帧请求 `request_animation_frame`。**没有**公开的离屏导出入口；`Scene` 为 `pub(crate)`，`render_to_image` 0 命中。[acceptance-gaps L85-L89](file:///d:/project/1107/docs/acceptance-gaps.md#L85-L89)、[acceptance-gaps L132](file:///d:/project/1107/docs/acceptance-gaps.md#L132)

### 4.2 候选方案详情

#### P0 静态基线（始终保留）

- **动的是什么**：无。
- **解决什么**：精确阅读、低配、无障碍、A/B 对照；也是所有动画的 fallback。
- **依赖**：现有 05/07 静态路径与 09 静态时间线刻度。
- **代价**：S（0 新增）。
- **风险**：没有引导，首屏可能不知道看哪里；这不是动画能单方面解决的，属于信息架构问题。
- **组合**：所有方案都必须能在 P0 下退化；动画关闭后应等价于 P0。

#### P1 G1 完成版（第一优先）

- **动的是什么**：步骤索引推进时，当前边高亮切换、非路径元素淡出、底部面板同步；**没有连续位移**。
- **解决什么**：「我现在在哪一步、下一步去哪、整条链多长」；把注意力从整张图收敛到当前一步。
- **依赖**：06/07（路径与步骤模型已有）；05（符号级视图）；需要补一个符号级路径画布。
- **代价**：M。
- **风险**：BFS 最短调用链被读成运行时序；符号级边种类/方向修正丢失；自动播放抢注意力。应对：文案写「静态最短调用链」；符号级也画当前边与边种类；默认手动步进。
- **组合**：与 P2 可叠加（P2 只做用户触发的镜头转场）；与 P5 互斥（首屏只允许一个主动效）；与 P3/P4 独立。

**落地要点**：

1. 把 `highlighted_nodes/edges` 与 `current_edge` 真正接到符号场景的边画布，而不是只改节点透明度。
2. 关闭动画时显示**静态完整路径**（所有路径边可见、当前边不高亮），并停止 ticker；当前实现只淡出非路径节点，语义不完整。
3. 面板显示「第 k/N 步」、当前符号、进入边的种类；不要显示「执行」字样。
4. 补结构断言：每一步的当前边集合、关闭后 ticker 不推进、结束态等于完整路径。

#### P2 短镜头转场（P1 的可选修饰器）

- **动的是什么**：视口中心/缩放的插值；不是节点位置。
- **解决什么**：下钻/跳转后的方向感与 object constancy；减少「我从模块视图跳到符号视图后不知道自己在哪」。
- **依赖**：05 的 `CanvasState`/viewport 与命中框；07 的 `camera_center` 模型。
- **代价**：M（视口 tween、命中框同步、取消/中断、reduced-motion）。
- **风险**：长转场导致眩晕/迷路；转场中点击错位；自动镜头打断阅读。应对：150–250ms；只由用户动作触发；可取消；保留 overview inset；播放中锁镜头；reduced-motion 直接切。
- **组合**：P1/P4 的修饰器；P3 播放期间禁用自动镜头；不与 R2 叠加。

#### P4 双 commit 结构 diff（演化理解的默认模式）

- **动的是什么**：选中一对 commit 后，新增/删除/修改的模块以静态色块/轮廓呈现；可选 120ms 淡入；不移动节点。
- **解决什么**：「这个 commit 到底改了什么、影响哪些模块」；把变更归属到具体 commit。
- **依赖**：08 的历史产物、09 的时间线刻度；13 的演化交互占位。
- **代价**：M（diff 计算、视图叠加、重命名归并、测试）。
- **风险**：重命名/节点身份误读；只显示变化不显示幅度；布局不一致。应对：稳定 ID + 显式「renamed」标签；同时显示 churn/lines 数字；布局与 HEAD 一致，不按 commit 重排。
- **组合**：P3 的替代默认模式（同一时间线，二选一）；与 P1 独立；与 P2 可叠加。

**与 CodeBoarding 的对应【推断】**：CodeBoarding 的「timeline slider + diagram light up」就是这一类；它验证了「离散高亮」在产品上可行，而没有验证「连续播放」在架构理解上有效。

#### P3 时间线离散播放（等 P10 缓存结论）

- **动的是什么**：时间轴指针按 commit 前进；热力颜色/尺寸离散切换；可选 150ms cross-fade。
- **解决什么**：「演化顺序、突变还是渐进、哪个 commit 变热」。
- **依赖**：08/09/10；13。
- **代价**：L（必须先解决 O(n²) 采样；再加 ticker、scrub/play 同步、degraded 标记）。
- **风险**：让用户以为 commit 之间连续变化；跳过 commit/空闲期；变化盲视；大仓卡顿。应对：离散 tick + 明示采样数；空闲压缩；变化处 dwell；旧值 ghost；大仓 `degraded`。
- **组合**：与 P4 互斥为默认模式；与 P2 可叠加但播放中锁镜头；与 P1 独立。

#### P5 首屏分层揭示（可选，最低优先）

- **动的是什么**：层带/模块按层序一次性淡入或升起；不循环。
- **解决什么**：首屏「从哪里看起」；建立层级心智模型。
- **依赖**：03 的层推断与 05 的模块视图。
- **代价**：S（一次性 opacity/offset；默认关）。
- **风险**：暗示层间执行顺序；延迟首屏可读性；对抽象图的收益证据弱。应对：只做一次、可跳过、默认关；文案说明「层是推断结果」。
- **组合**：与 P1 互斥（同一时段只能一个主动效）；只做首次进入的一次性引导。

#### R1–R4（明确不做）

见 0.2 的「明确不做」表。核心原因不是「实现不了」，而是「没有新增信息 + 有稳定误读风险」：

- R1 运动因果知觉 → 数据流误读。
- R2 运动 popout → 注意力捕获。
- R3 自动循环 → 违反 WCAG 2.2.2 的可用性底线。
- R4 连续插值 → 没有真实中间帧，伪造精度。

### 4.3 G1 够不够

**结论：作为「步进播放器骨架」够用；作为「演示动画」不够。** 具体理由：

| 维度 | 现状 | 够不够 |
|---|---|---|
| 步骤模型 | `current_edge`、`highlighted_nodes/edges`、`back/advance` 都在 | 够 |
| 模块视图 | 路径边 + 当前边白线 | 基本够 |
| 符号视图 | 路径边全部 `strong: false`，没有当前边强调 | **不够** |
| 关闭动画 | 只改模型标志；UI ticker 仍按 180ms 推进，当前边仍按 `overlay.current` 高亮；没有「关闭后冻结为静态完整路径」的独立模式 | **不够** |
| fly-to | 模型有 `camera_center`，UI 未消费 | **不够** |
| 连续性 | 180ms 定时器步进，不是逐帧插值 | 作为「步进」够，作为「动画」不够 |
| 帧时间证据 | `out/frame-timing.json` 是 `model-update-only`，不是 GPU 帧 | **不能用来证明流畅度** |
| 测试 | 模型状态断言全绿；UI 只有 smoke | 模型够，UI 不够 |

**因此**：G1 当前适合作为 P1 的**控制条件/基线**，不适合作为对外演示的「动画」。要变成 P1，需要补：符号级路径画布、符号级当前边、关闭后的静态完整路径、真正的全局动画开关、可选的短镜头转场、以及 UI 级结构断言。

### 4.4 组合矩阵

|  | P1 | P2 | P3 | P4 | P5 |
|---|---|---|---|---|---|
| **P1** | — | 可叠加（P2 作修饰器） | 独立 | 独立 | **互斥**（首屏只允许一个主动效） |
| **P2** | 可叠加 | — | 可叠加但播放中锁镜头 | 可叠加 | 可叠加（P5 仅首屏） |
| **P3** | 独立 | 可叠加但播放中锁镜头 | — | **互斥为默认模式**（可共存为按钮，不同时动） | 独立 |
| **P4** | 独立 | 可叠加 | **互斥为默认模式** | — | 独立 |
| **P5** | **互斥** | 可叠加 | 独立 | 独立 | — |

**解释**：P3 与 P4 是同一时间线的两种读法，默认只能选一个；P1 与 P5 都是「首屏主动效」，同时播放会互相抢注意力；P2 是修饰器，不单独构成一个理解方案。

---

## 5. 性能与能力的硬约束

### 5.1 读数表

| 读数 | 值 | 出处 | 口径/限制 |
|---|---|---|---|
| 时间线 6 刻度 | 墙钟 **4435ms**（含 500ms 自动关闭） | [09 L42](file:///d:/project/1107/docs/tickets/arch-tool/09-evolution-timeline-heat-ui.md#L42) | fixture `tests/fixtures/history` HEAD `1de5415`，first-parent 6 个；每个刻度各跑一次前缀 `build_history` |
| 时间线复杂度 | **O(n²)** 前缀采样 | [09 L49](file:///d:/project/1107/docs/tickets/arch-tool/09-evolution-timeline-heat-ui.md#L49)、[n3 L44](file:///d:/project/1107/docs/proposals/n3-evolution-playback.md#L44) | 每刻度重跑 `build_history(until=@time+1)`；大仓不可用 |
| 路径播放模型单帧（归档） | 中位数 **0.1µs**；p95 **0.2µs**（3 步与 20 步均如此） | [out/frame-timing.json](file:///d:/project/1107/out/frame-timing.json#L1) | `scope: model-update-only`；**不是 GPU 帧** |
| 路径播放模型单帧（文档文字） | 3 步 p95 **0.3µs**、20 步 p95 **0.2µs** | [07 L40](file:///d:/project/1107/docs/tickets/arch-tool/07-path-playback-stepthrough-ui.md#L40) | 与 `out/frame-timing.json` 的 3 步 0.2µs 不一致；以归档 JSON 为原始读数，文档文字存在漂移 |
| 5k 合成下钻启动 | 中位数 **127.5ms**、p95 **130.4ms**；下钻 0.1/0.2ms、返回 0.0/0.0ms | [NIGHTLOG L75](file:///d:/project/1107/docs/process/NIGHTLOG.md#L75) | 5k 合成；16 CPU / AMD64 Family 25 Model 117；debug |
| 窗口 smoke（`fixtures/layers`） | **2116ms**（含 500ms 自动关闭） | [NIGHTLOG L75](file:///d:/project/1107/docs/process/NIGHTLOG.md#L75) | 5k 合成启动读数与这条 smoke 不是同一 fixture；smoke 只证明窗口能起，不是动画帧率 |
| tsx index | 批次 1 中位数 **69.4ms**；批次 2 **69.5ms** | [10 L11](file:///d:/project/1107/docs/tickets/arch-tool/10-performance-memory-baseline.md#L11)、[10 L30-L38](file:///d:/project/1107/docs/tickets/arch-tool/10-performance-memory-baseline.md#L30-L38) | 3 次运行中位数；冷启动首轮 93.6/101.4ms 已保留为噪声 |
| excalidraw index | 批次 1 中位数 **278.3ms**；批次 2 **293.4ms** | [10 L11](file:///d:/project/1107/docs/tickets/arch-tool/10-performance-memory-baseline.md#L11)、[10 L30-L38](file:///d:/project/1107/docs/tickets/arch-tool/10-performance-memory-baseline.md#L30-L38) | 波动 5.4%，≤20% |
| 超限仓库 | 5001 文件 → exit 2、stdout 空、stderr 拒绝 | [10 L14](file:///d:/project/1107/docs/tickets/arch-tool/10-performance-memory-baseline.md#L14) | 上限 5000；不是静默降级 |
| 合成 history | 100 文件 × 500 提交 **0.99s**；本仓 19 提交 0.66s | [NIGHTLOG L37](file:///d:/project/1107/docs/process/NIGHTLOG.md#L37) | excalidraw 完整克隆**未验证**（L2 缓存无 `.git`） |
| 离屏导出 | **不可用**：`render_to_image` 0 命中；`Scene` 为 `pub(crate)` | [acceptance-gaps L85-L89](file:///d:/project/1107/docs/acceptance-gaps.md#L85-L89)、[11 L11](file:///d:/project/1107/docs/tickets/arch-tool/11-cross-platform-gpui-smoke.md#L11) | gpui 0.2.2 无公开入口；截图路径待 11 选型 |
| GPUI 动画 API | `with_animation`、`request_animation_frame`、`Animation` 存在 | [acceptance-gaps L132](file:///d:/project/1107/docs/acceptance-gaps.md#L132) | 按墙钟时间推进；没有 timeline scrub API |
| 帧预算 | p95 ≤ **16.7ms**（推荐值） | [07 L15](file:///d:/project/1107/docs/tickets/arch-tool/07-path-playback-stepthrough-ui.md#L15)、[07 L50](file:///d:/project/1107/docs/tickets/arch-tool/07-path-playback-stepthrough-ui.md#L50) | **待用户确认**；当前只归档模型更新耗时 |
| 真实帧时间 | **未验证** | [spec L207](file:///d:/project/1107/docs/spec-architecture-tool.md#L207)、[07 L48](file:///d:/project/1107/docs/tickets/arch-tool/07-path-playback-stepthrough-ui.md#L48) | 没有 GPU 帧埋点；`gpui::test` 事件自动化未跑通 |
| 播放 tick | **180ms** | [main.rs L907](file:///d:/project/1107/crates/arch-view/src/main.rs#L907) | 离散步进；不是逐帧 |

### 5.2 每条约束对动画的影响

1. **09 的 O(n²) 采样是 P3 的硬阻塞。** 6 刻度 4435ms 已经不可接受；大仓只会更差。P3 必须先有 10 的缓存/增量方案，否则只能做「小仓受限版本 + `degraded`」。
2. **07 的 µs 读数是状态机，不是渲染。** 0.1µs 中位数 / 0.2µs p95 只能证明「模型更新不是瓶颈」，不能证明「动画流畅」。任何关于流畅度的结论都必须重新测真实帧。
3. **gpui 0.2.2 无离屏导出。** 动画的视觉正确性不能进 CI 像素门禁；只能做状态断言 + 人工 smoke + ticket 11 的截图路径选型。这是「怎么评估」一节的约束。
4. **布局是静态的。** 当前模块位置来自 HEAD 层；时间线早期刻度不会让模块消失，只会热力中性。要做「节点出现/消失/移动」的结构动画（R4 的 morph 变体），需要按 commit 重建布局，代价是 L。
5. **符号级路径没有画布。** P1 的主要工作量在这里，不在模型。
6. **fly-to 只到模型。** P2 不是「加个开关」，而是要把 `camera_center` 接进 `CanvasState`，并处理命中框、取消、reduced-motion。
7. **5k 每帧成本未验证。** 5k 的启动是 127.5ms，但「5k 节点逐帧重绘」没有任何读数；不能承诺 60fps。R1/R2 如果作用于全图，风险更大。
8. **16.7ms 预算是推荐值，不是已确认契约。** 不能把它写成「已达成」；只能写成「待确认的预算」。

### 5.3 未验证清单

- **GPU 真实帧时间**：没有离屏导出，也没有 UI 帧埋点；`out/frame-timing.json` 是模型更新。
- **5k 节点逐帧重绘成本**：无读数。
- **`with_animation` 同时作用于 N 个节点的开销**：无读数；【推断】每个元素每帧请求一次 `request_animation_frame`，N 大时可能放大，需实测。
- **excalidraw 完整历史的 `build_history` 成本**：L2 缓存无 `.git`，未测；只有合成 100×500 = 0.99s。
- **跨平台动画表现**：Windows 只做了启动 smoke；mac/Linux 未运行。
- **粒子/呼吸是否真的会被误读**：这是【推断】，有 Michotte/popout 的间接证据，但没有在本项目用户上测过。
- **P1/P2/P4 是否真的提升理解**：需要 6 的人测；当前没有任何本项目读数。

---

## 6. 怎么判断动画有没有用

### 6.1 不能用的判据

- **「两次运行像素差」**：只能证明渲染确定性/回归，不能证明理解。静态图和误导动画都可以像素稳定。
- **「帧率 ≥60fps」**：只能证明流畅，不能证明信息传达正确；快而错的动画更危险。
- **「用户说好看/喜欢」**：偏好不等于理解；Fish et al. 的过度自信就是反例。
- **「动画组答题更快」单独看**：速度提升可能来自猜测；必须同时看准确率和置信度校准。

像素差可以保留为 **L0 回归**（静态末帧、关闭动画后的等价性），但不能作为效果判据。

### 6.2 三层评估

| 层 | 目的 | 手段 | 能证明什么 | 不能证明什么 |
|---|---|---|---|---|
| **L0 结构/状态** | 动画是否按契约发生 | `cargo test -p arch-view` 状态断言；步索引 ↔ 高亮集合；关闭后 ticker 不推进；结束态 == 静态全路径；reduced-motion 分支 | 正确性、可关闭性、末态一致 | 用户是否理解 |
| **L1 预算** | 动画是否拖慢工具 | 模型更新耗时 + 真实帧间隔（UI 内 `Instant` 日志）；p50/p95；5k/20k 规模 | 性能是否退化、是否超预算 | 信息是否传达 |
| **L2 人测** | 动画是否让理解更快/更准 | 固定任务 + 答案 key + 计时 + 置信度 + 误答分类；A/B 或 within-subject | 理解效果、误读类型 | 大规模普适性（小样本） |

**L1 的具体做法【推断】**：在 `arch-view` 的渲染回调里记录 `Instant::now()` 的帧间隔，写到 `out/animation-frame-log.json`；不要用模型耗时替代。由于 gpui 0.2.2 无离屏导出，L1 只能在本机跑，不能进 CI 硬门禁；这与 07 的「帧时间仅本机归档」一致。

### 6.3 人测协议（扩展 ticket 12）

ticket 12 已定义 3 个任务、5 名参与者、每任务 ≥4/5 的推荐阈值、`out/usability.json` 归档。[12 L11-L15](file:///d:/project/1107/docs/tickets/arch-tool/12-usability-task-test.md#L11-L15) 动画评估**复用 12 的任务 1–3 与冻结 key**，新增任务 4（commit 归属）和任务 5（语义误读）；这样动画条件与静态条件的结果可以直接对比，不用另起一套不可比的题。

**设计**：within-subject，条件顺序拉丁方平衡；同一 participant 做所有条件，任务内容等价但实例不同（避免记忆）。参与者 5 人起步（与 12 一致，待用户确认）；若要做统计，建议 8–12 人，但小样本只报个体与中位数，不硬做 p 值。

**条件与任务映射**：

| 条件 | 覆盖任务 | 说明 |
|---|---|---|
| **C0 静态基线**（P0） | T1/T2/T3 | 对照；所有动画条件都要和它比 |
| **C1 P1 离散路径步进** | T1/T2/T3 | 主实验条件 |
| **C2 C1 + P2 短镜头转场** | T1/T3 | 只测「迷路」是否减少 |
| **C3 P4 commit diff 静态高亮** | T4/T2 | 测变更归属，不测连续运动 |
| **C4 P3 时间线离散播放**（仅在缓存落地后） | T2/T3 | 测演化顺序，不测路径 |

**任务与记分**：

| 任务 | 问什么 | 答案 key | 记分 | 误答分类 |
|---|---|---|---|---|
| **T1 路径理解**（复用 12 任务 1） | 从入口到目标符号，经过哪些符号/模块与层？ | 冻结的模块/符号有序序列 + 层序列 | 序列完全匹配 = 1；编辑距离/位置加权部分分；记录耗时 | 顺序颠倒、漏步、幻觉边、把方向修正读反 |
| **T2 演化热点**（复用 12 任务 2） | 窗口内被反复修改最多的 3 个模块是哪些？依据是什么？ | 冻结的 top-3 + 指标区间 | 集合 F1；记录耗时 | 漏热点、把 churn 读成大小、把采样刻度读成连续变化 |
| **T3 长期未动**（复用 12 任务 3） | 多年未动的 2 个模块是哪些？依据是什么？ | 冻结的 2 个模块 + 指标区间 | 集合精确匹配；记录耗时 | 把低 churn 误当未动、漏掉重命名 |
| **T4 commit 归属**（新增，P4 专用） | 选定的 commit 改了哪些模块？新增/删除/修改各是什么？ | 冻结的模块集合 + 变更类型 | precision/recall；记录耗时 | 漏改、把重命名读成删除+新增、把相邻 commit 混淆 |
| **T5 语义误读**（新增，仅含运动的候选） | 动画里沿边移动的点代表什么：数据流、执行顺序、还是静态调用关系？并追问「你为什么这么想」 | 正确答案 = 静态调用关系 | 正确归因率；错误归因按类型计数 | 数据流、执行顺序、因果传递 |

**主/次指标**：

- **主**：T1–T4 的答案准确率（与 key 一致的比例）。
- **次**：完成时间中位数；置信度（1–5）与正确率的校准（高置信错答率）；T5 的错误归因率。
- **判定**：动画条件必须在**准确率不劣于静态**（每任务允许至多 1 人差异，具体阈值冻结）且**时间中位数至少快 20%** 时，才算「有用」；若准确率下降，无论多快都判失败。20% 是提议值，必须测试前冻结，不能事后改。
- **否定结果**：如果 P1/P4 没有达到，保留静态默认，动画降级为可选演示；不得为了「动画有效」改 key 或改任务。

**归档**：`out/animation-eval.json`，字段至少包含：`participant`（匿名）、`condition`、`order`、`task`、`answer`、`key`、`score`、`time_ms`、`confidence`、`error_type`、`views_used`、`reduced_motion`。原始记录（纸面/录屏）留存可复核；主仓只放汇总，原始参与者数据不入库。

**失败判据**：

- 答案 key 在测试后被修改。
- 证据缺失（无计时、无原始记录、无 condition 记录）。
- 动画无法关闭，或关闭后仍推进/仍调度帧。
- 准确率下降但报告只强调「更快/更好看」。
- 只报总体、不报分任务/分参与者。

### 6.4 防作弊与报告纪律

1. **key 先冻结**：测试前写死 T1–T5 的答案 key 与评分脚本；测试后不得改。
2. **不用像素差当效果**：像素差只做 L0 回归。
3. **分任务报告**：每个任务、每个条件、每个 participant 都列；不允许只给一个平均分。
4. **保留负结果**：P1/P2/P4 如果无效，写「未达成」并保留读数；不得删除条件或换任务。
5. **置信度必测**：高置信错答是动画最危险的失败模式（Fish et al. 的过度自信）。
6. **预注册**：测试前写下 H1（P1 让 T1 更快且不更错）、H2（P4 让 T4 更准）、H3（P2 降低 T1 的迷路误答）、H4（R1 增加 T5 的数据流误读）；以及样本量、停止规则、分析方式。

---

## 7. 未决问题

1. **P1 的符号级路径画布**：复用现有 `edge_canvas` 还是新建符号场景画布？符号级边种类/方向修正如何画？
2. **P2 的实现路径**：用 `with_animation` 包住容器 transform，还是手写 viewport 插值？前者按墙钟、难做确定性测试；后者可控但要多写状态。
3. **P3 的缓存设计**：增量 `build_history`、按 commit 产物缓存、还是只缓存 module churn？10 的结论出来前不选。
4. **P4 的 diff 粒度**：只做模块级，还是下钻到符号级？符号级需要 04 的调用图 diff，成本更高。
5. **reduced-motion 的产品形态**：跟随系统、应用内开关、还是两者都有？WCAG 2.3.3 要求可关闭，建议两者都有。
6. **「理解」的通过阈值**：ticket 12 的 ≥4/5 是否直接适用于动画 A/B？5 人样本的统计力很弱，是否需要 8 人？
7. **真实帧时间怎么测**：gpui 0.2.2 无离屏导出；是用 UI 内埋点、OS 捕获，还是等 11 选定截图路径？
8. **P1 与 P4 的默认关系**：路径理解与演化理解是两个入口；是否需要「路径模式 / 演化模式」二选一，而不是同时开两个主动效？
9. **是否要做 G2 首屏分层揭示**：它不在用户本轮要求里，且证据最弱；建议等 P1/P4 的人测结果再决定。

---

## 附录 A：来源

### 本地

- [07 路径播放与逐步追光](file:///d:/project/1107/docs/tickets/arch-tool/07-path-playback-stepthrough-ui.md)
- [09 演化时间线热力 UI](file:///d:/project/1107/docs/tickets/arch-tool/09-evolution-timeline-heat-ui.md)
- [10 性能与内存基线](file:///d:/project/1107/docs/tickets/arch-tool/10-performance-memory-baseline.md)
- [11 跨平台 GPUI smoke](file:///d:/project/1107/docs/tickets/arch-tool/11-cross-platform-gpui-smoke.md)
- [12 可用性任务测试](file:///d:/project/1107/docs/tickets/arch-tool/12-usability-task-test.md)
- [06 入口/目标/最短路径](file:///d:/project/1107/docs/tickets/arch-tool/06-entry-target-guided-path-query.md)
- [NIGHTLOG](file:///d:/project/1107/docs/process/NIGHTLOG.md)
- [acceptance-gaps](file:///d:/project/1107/docs/acceptance-gaps.md)
- [n3 演化播放讨论稿](file:///d:/project/1107/docs/proposals/n3-evolution-playback.md)
- [spec 动画策略](file:///d:/project/1107/docs/spec-architecture-tool.md)
- [out/frame-timing.json](file:///d:/project/1107/out/frame-timing.json)
- [playback.rs](file:///d:/project/1107/crates/arch-view/src/playback.rs)
- [main.rs](file:///d:/project/1107/crates/arch-view/src/main.rs)
- [timeline.rs](file:///d:/project/1107/crates/arch-view/src/timeline.rs)

### 外部

- [Gource](https://gource.io/)
- [Gource Controls wiki](https://github.com/acaudwell/Gource/wiki/Controls)
- [Gource 评测（presentation not analysis）](https://hysenlabs.com/projects/acaudwell-gource)
- [CodeCharta Views](https://codecharta.com/docs/visualization/user-controls/views/)
- [CodeCharta Compare](https://codecharta.com/docs/visualization/user-controls/compare/)
- [pomber/git-history](https://github.com/pomber/git-history)
- [git-history 产品说明](https://ross.abutalabs.com/products/git-history)
- [dependency-cruiser README](https://gitlab.com/sverweij/dependency-cruiser/-/blob/develop/README.md)
- [madge README](https://github.com/pahen/madge)
- [CodeBoarding Open VSX](https://open-vsx.org/extension/CodeBoarding/codeboarding)
- [Armenti & Lanza, Using Interactive Animations to Analyze Fine-grained Software Evolution](https://www.inf.usi.ch/faculty/lanza/PUBS/P/Arme2024b.pdf)
- [Höffler & Leutner 2007](https://wikicap.ulb.be/images/e/ef/Hoffler_Leutner_2007.pdf)
- [Berney & Bétrancourt 2016](https://tecfa.unige.ch/perso/sandra/pdf/Earli2016_berney_betrancourt_FINAL.pdf)
- [Tversky/Morrison/Bétrancourt 综述 PDF](https://courses.ischool.berkeley.edu/i247/f05/readings/Morrison_AnimationFacilitateLearning.pdf)
- [Hundhausen, Douglas & Stasko 2002](https://doi.org/10.1006/jvlc.2002.0237)
- [Smooth view transitions in node-link diagrams (Manitoba, 2007)](https://hci.cs.umanitoba.ca/publications/details/quantifying-the-effects-of-smooth-view-transitions-on-perceptual-constancy)
- [Simons & Levin 1997, Change blindness](https://home.cs.colorado.edu/~mozer/Teaching/syllabi/3702/readings/SimonsLevin1997.pdf)
- [Nowell et al., Change blindness in information visualization (IEEE 963274)](https://ieeexplore.ieee.org/document/963274)
- [Fish et al. 2011 课程总结](https://courses.ems.psu.edu/geog486/node/924)
- [Gutwin, Cockburn & Coveney, Peripheral Popout (CHI 2017)](http://library.usc.edu.ph/ACM/CHI%202017/1proc/p208.pdf)
- [Michotte 因果知觉注册重复研究](https://pmc.ncbi.nlm.nih.gov/articles/PMC12434928/)
- [WCAG 2.2.2 Pause, Stop, Hide](https://w3c.github.io/wcag/understanding/pause-stop-hide.html)
- [WCAG 2.3.3 Animation from Interactions](https://w3c.github.io/wcag/understanding/animation-from-interactions)
- [WCAG C39 prefers-reduced-motion](https://w3c.github.io/wcag/techniques/css/C39)

---

## 附录 B：证据边界速查

| 命题 | 类型 | 依据 |
|---|---|---|
| Gource 是时间驱动的生长树，默认每天 10 秒、自动跳 3 秒、文件 60 秒过期 | 事实·外部 | Gource 官网 + Controls wiki |
| CodeCharta 用静态 Compare，官方文档无时间播放控件 | 事实·外部 | CodeCharta Views/Compare |
| 依赖图工具输出静态图 | 事实·外部 | dependency-cruiser/madge README |
| CodeBoarding 用时间线滑块 + 离散高亮 | 事实·外部 | Open VSX |
| 动画对学习有小到中等效应 | 事实·外部 | Höffler & Leutner 2007；Berney & Bétrancourt 2016 |
| 抽象图示收益低于图像式表示 | 事实·外部 | Berney & Bétrancourt 2016 |
| 平滑转场可把错误减半，短转场同样有效 | 事实·外部 | Manitoba 2007 |
| 变化盲视会让人漏看动画中的变化，且过度自信 | 事实·外部 | Simons & Levin 1997；Fish et al. 2011 |
| 运动是强 popout，低强度/周边视野也有效 | 事实·外部 | Gutwin et al. CHI 2017 |
| 运动可产生知觉层面的因果印象 | 事实·外部 | Michotte 重复实验 |
| G1 的 UI 只到模块级，符号级追光只在数据层 | 事实·本地 | 07 L49 + main.rs 758–847 |
| 播放是 180ms 离散步进，不是逐帧 | 事实·本地 | main.rs L907 |
| 动画开关不影响 UI ticker | 事实·本地 | main.rs L898–970 |
| 时间线 6 刻度 4435ms，O(n²) | 事实·本地 | 09 L42/L49 |
| 模型帧中位数 0.1µs / p95 0.2µs，不是 GPU 帧 | 事实·本地 | out/frame-timing.json |
| gpui 0.2.2 无公开离屏导出 | 事实·本地 | acceptance-gaps L85–89 |
| 粒子会被读成数据流 | 推断 | Michotte 因果知觉 + Strata 边语义 |
| P1/P2/P4 能提升本项目理解 | 未验证 | 需要 6 的人测 |
| 5k 节点动画帧率 | 未验证 | 无读数 |
