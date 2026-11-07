# R2-7 / R2-8 讨论材料（未定稿）

生成时间：2026-11-07
状态：讨论稿；R2-7 动画形式、R2-8 样本仓选择均未拍板，未写代码。

---

## A. R2-8：Demo 仓库是什么，为什么是两个

### A.1 先澄清

**Demo 仓库 = 被本工具解析的目标仓库，是测试语料/基准，不是本工具的开发环境，也不是 UI 示例。** 它与 GPUI 无关：GPUI 决定「我们用什么画界面」，样本仓决定「我们拿什么 TS/JS 代码验证解析器和图构建是否正确」。换 GPUI 不会改变样本仓的语言范围（仍是 TS/JS）。

### A.2 为什么要两个，而不是一个

一个仓库无法同时提供「干净基线」和「复杂边界」：

- 选简单单包仓库：覆盖了 `bin`/`exports` 与相对导入，但漏掉 workspace、路径别名、JSX 调用边、框架入口。
- 选复杂 monorepo：覆盖了复杂边界，但没有一个可解释的干净基线；解析器一出错，很难判断是通用逻辑错还是某个特性没实现。

两个钉死 commit 的仓库提供：

1. **两套 golden 断言**：一个失败时能区分「通用解析错」与「workspace/别名/JSX 特有错」。
2. **两种入口形态**：`package.json` 的 `bin`/`exports` vs 框架应用入口。
3. **两种模块边界**：单包相对导入 vs 跨包 `workspace:*` 导入。
4. **两种语法面**：纯 TS vs TS + JSX/TSX。
5. **两个规模点**：小型仓与中型仓，用于观察索引耗时/内存是否随规模非线性上升。

单仓会漏掉的具体边界：

- workspace 包解析（`workspace:*`、`packages/*`、`pnpm-workspace.yaml`）；
- `tsconfig` path alias（`~/*`、`@/*`、`@excalidraw/*`）；
- JSX/TSX 组件调用边（组件引用不是普通函数调用）；
- barrel/re-export 链（`export *`、`index.ts` 转发）；
- 框架入口（Next.js app/pages、路由文件）与 CLI `bin` 入口的差异；
- 跨包依赖与包级外部节点；
- 中型仓库的规模压力。

### A.3 候选仓实测（可复现命令与读数）

测量命令（本轮实际执行）：

```powershell
gh api repos/<owner>/<repo> --jq '{full_name,license:.license.spdx_id,stars:.stargazers_count,pushed_at,default_branch}'
$br = gh api repos/<owner>/<repo> --jq '.default_branch'
gh api "repos/<owner>/<repo>/git/trees/$br`?recursive=1" --jq '{truncated: .truncated, ts: ([.tree[]|select(.type=="blob" and (.path|test("\\.(ts|tsx)$")))]|length), tsx: ([.tree[]|select(.type=="blob" and (.path|test("\\.tsx$")))]|length), total: ([.tree[]|select(.type=="blob")]|length)}'
```

> 注意：`ts` 计数已包含 `.tsx`；`tsx` 是其中子集。下表写「TS/TSX」。

| 候选 | 许可 | star | 最近 push | TS/TSX | 总文件 | 结构特征（已读配置文件） |
| --- | --- | --- | --- | --- | --- | --- |
| `privatenumber/tsx` | MIT | 12181 | 2026-09-25 | 91 | 229 | 单包；`bin: ./dist/cli.mjs`；复杂 `exports` 条件映射；`tsconfig` 无 paths |
| `egoist/tsup` | MIT | 11303 | 2026-11-07 | 46 | 78 | 单包；`bin`/`exports` 未核 |
| `excalidraw/excalidraw` | MIT | 135260 | 2026-11-06 | 690（TSX 323） | 1328 | npm `workspaces`（`excalidraw-app`、`packages/*`、`examples/*`）；`tsconfig` paths `@excalidraw/*`；JSX |
| `trpc/trpc` | MIT | 40780 | 2026-11-06 | 1312（TSX 336） | 1920 | pnpm workspace（`packages/*`、`examples/*`、`www`、`tooling/*`）；`tsconfig` paths `@trpc/*`；TSX |
| `t3-oss/create-t3-turbo` | MIT | 6110 | 2026-11-06 | 70（TSX 22） | 141 | pnpm workspace（`apps/*`、`packages/*`、`tooling/*`）；Next.js `~/*` 别名；`workspace:*` 依赖；React 19 |
| `vercel/commerce` | MIT | 14312 | 2026-10-19 | 65（TSX 45） | 78 | 单包 Next.js；`tsconfig` 未发现 paths（**需要复核**） |
| `shadcn-ui/taxonomy` | MIT | 19291 | 2026-04-20 | 125（TSX 94） | 188 | 单包 Next.js；`@/*` 别名；React |
| `shadcn-ui/ui` | MIT | 127541 | 2026-11-07 | 4301（TSX 3674） | 6411 | 大型 monorepo；总文件超过 5k 目标 |

### A.4 组合建议（供讨论，不拍板）

- **组合 1（结构 + 规模）**：`privatenumber/tsx` + `excalidraw/excalidraw`。
  - 覆盖：CLI 入口/`exports` vs React monorepo 入口；相对导入 vs npm workspace + `@excalidraw/*` 路径别名；纯 TS vs TSX；229 vs 1328 文件。
  - 风险：excalidraw 是 npm workspaces，不是 pnpm；能覆盖 workspace 与别名，但不覆盖 `workspace:*` 协议。
- **组合 2（结构 + workspace 协议）**：`privatenumber/tsx` + `trpc/trpc`。
  - 覆盖：pnpm workspace + `@trpc/*` 路径别名 + TSX + 1920 文件；规模比 excalidraw 更大。
  - 风险：trpc 是库 monorepo，不是典型 React 应用；框架入口的代表性弱于 excalidraw。
- **组合 3（最小结构覆盖）**：`privatenumber/tsx` + `t3-oss/create-t3-turbo`。
  - 覆盖：workspace + `~/*` 别名 + `workspace:*` + Next.js + React；但规模只有 141 文件，性能覆盖不足，需要额外压测语料。

**未验证项**：以上仓库的实际可解析性、`unresolved` 比例、AST 特性覆盖、解析耗时，均未测（本轮没有解析器代码）。许可均为 MIT，但若要 vendoring 进测试仓，仍需逐仓核对 LICENSE 与 NOTICE。建议钉 commit、测试时按需下载，而不是整仓拷贝。

**需要你拍板**：选哪一组；是否允许出现第三个「性能压测语料」；是否接受 MIT 样本仓进 CI。

---

## B. R2-7：动画形式的具体设想

### B.1 约束

- 纯 GPUI canvas 自绘，无现成图库；布局、边路由、标签、命中测试都自己实现。
- 动画只由 GPUI 帧驱动（`with_animation` / `request_animation_frame`），禁止 Web 式 `Date.now`/`requestAnimationFrame`。
- 动画必须可关闭；关闭后信息不能丢失。
- 60fps 目标，p95 帧时间 ≤16.7ms（待你确认预算）。

### B.2 候选方案

| # | 方案 | 动的是画面里的什么 | GPUI 实现代价 | 主要坑 | 理解价值 | MVP 判断 |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | **路径沿边播放** | 一个高亮标记沿调用/依赖边从入口走向目标；经过的节点与边依次点亮，未经过的变暗 | 低–中：边需要折线/弧长参数化，按 `t` 插值 | 边路由重叠；标签遮挡；长边速度；帧预算 | 高：直接回答「怎么走」 | **建议进** |
| 2 | **逐步追光（step-through）** | 不连续播放；用户点「下一步」，画面只切换一步：当前边高亮、节点点亮、旁注更新 | 低：状态机 + 静态高亮 | 需要调用顺序数据；无动画也可能被误认为「没做完」 | 最高：可核对每一步 | **建议进** |
| 3 | **镜头 fly-to** | 视口平移/缩放插值，把选中节点或整条路径移到视野中心 | 低：变换矩阵插值 + 命中测试同步 | 晕动；丢失全局上下文；动画中点击错位 | 中：省去手动找 | 建议可选，默认瞬时 |
| 4 | **聚焦 + 上下文淡出** | 选中路径外的节点/边透明度下降，相关邻居保持，其余变灰 | 低：按图距离算 alpha | 对比度不足；标签可读性；色盲 | 中高：降噪 | **建议进** |
| 5 | **分层逐层升起** | 分层地图从下往上逐层平移/淡入/展开 | 低：层组 transform + alpha | 暗示虚假的时间顺序；布局需预留空间；循环播放很吵 | 低–中 | 仅首屏一次，可关 |
| 6 | **边生长** | 边从源节点向目标节点逐段画出 | 低：折线长度插值 | 每次布局变化都重播会烦；长边慢 | 低–中 | 可选，默认关 |
| 7 | **展开/折叠 morph** | 模块节点在原地展开成内部图，节点位置从聚合态过渡到明细态 | 中–高：两套布局 + 节点对应关系 | 边交叉；标签碰撞；方向感丢失 | 高但风险大 | 延后 |
| 8 | **双 commit 差分动画** | 两次索引之间新增/删除/改动的节点与边以不同状态进入/退出 | 高：第二份索引、稳定符号 ID、git 集成 | ID 不稳定；布局跳动；语义 diff 误判 | 高（评审场景） | 延后，超出 onboarding MVP |
| 9 | **依赖边粒子流动** | 小粒子沿边持续移动，表示「有流向」 | 高：每帧 O(粒子数) 绘制；边多时爆预算 | 视觉噪声；**会被误读为运行时数据流**，而数据流是二阶段 | 低（甚至误导） | 不做 |
| 10 | **热点节点呼吸** | 重要/命中节点尺寸或透明度周期性变化 | 极低：sin 曲线 | 吸引注意力但不提供信息；无障碍问题 | 低 | 不做（用静态聚焦环替代） |

### B.3 可组合方案组

| 组 | 组成 | 定位 |
| --- | --- | --- |
| **G1 理解优先（推荐 MVP）** | 1 路径沿边播放 + 2 逐步追光 + 4 聚焦/淡出 + 3 镜头 fly-to（可选、可瞬时） | 每个动画都回答「走到哪、为什么」；共享同一路径模型，增量成本低；可冻结 `t` 做截图断言 |
| **G2 轻量润色** | 5 分层升起（仅首屏一次）+ 6 边生长（仅布局首次） | 好看但可以关；默认关闭或只播一次 |
| **G3 延后** | 7 展开/折叠 morph + 8 双 commit 差分 | 依赖稳定 ID/布局对应关系；不是 onboarding 必需 |
| **G4 演示好看但不解决理解** | 9 粒子流动 + 10 呼吸；5/6 循环播放 | 增加视觉噪声，容易掩盖信息；9 还会误导为数据流 |
| **G5 高风险** | 7、8 | 实现成本高、回归面大，需要专门验收 |

### B.4 倾向与理由（不替用户拍板）

倾向 **G1**，理由：

1. 1 与 2 共享「路径 + 边折线 + 步骤索引」模型，实现增量小；
2. 4 是纯绘制层 alpha，几乎零成本，直接降低噪声；
3. 3 是视口变换，容易做，也容易关；
4. 全部可以冻结在固定 `t`/步数，便于 `render_to_image` 截图与结构断言；
5. G2 只保留「首屏一次」，避免每次布局变化都重播。

不建议进 MVP：9（误导 + 帧预算）、10（装饰）、7/8（成本与依赖未就绪）。

### B.5 跨方案共同坑

- **边路由**：先做确定性直线/正交折线；曲线与避障后置。路由必须确定性，否则截图基线不可复现。
- **标签避让**：低缩放隐藏标签（LOD）；标签碰撞检测；文本布局缓存；GPUI 文本度量跨平台差异（见跨平台项）。
- **命中测试**：节点用 `insert_hitbox`；边用点到折线距离；变换矩阵必须用当前帧，不是动画起点；动画中禁止出现「看得见但点不到」。
- **帧预算**：视口裁剪、离屏节点不绘制、每帧不分配、文本缓存；9 的粒子是反例。
- **测试冻结**：提供 `animation_t`/`step` 注入，测试在固定 t 出图；关闭动画后画面必须与「动画结束态」一致。
- **可关闭**：设置项 + reduced motion；关闭后所有信息仍可见（高亮、旁注、路径列表）。

### B.6 需要你拍板

1. MVP 动画组选 G1，还是 G1 + G2？
2. 镜头 fly-to 默认开启还是默认瞬时？
3. 动画默认开启还是默认关闭？
4. 是否接受「逐步追光」作为主交互（它可能比连续动画更符合 onboarding）？
5. 帧预算 16.7ms 还是 33ms？
