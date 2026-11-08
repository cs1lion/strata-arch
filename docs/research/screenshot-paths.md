# 票 11 截图方案调研：gpui 离屏导出、git 版本与 CPU 光栅器

研究问题：Strata 在 `gpui = "=0.2.2"` 下没有公开的窗口帧导出入口。票 11 还挂着三条未定路径：(a) OS 窗口捕获、(b) 换 gpui git 版本、(c) 场景 IR + CPU 光栅器。本轮只回答"哪条路真的可执行、代价是多少"，不写产品代码。

证据标签沿用 `docs/research/animation/README.md` 的约定：

- **【事实·本地】** 本仓库文件、命令输出或已入库证据，附路径与行号。
- **【事实·外部】** Zed 仓库源码/提交/标签，附文件路径、行号或 commit。
- **【推断】** 基于上述事实的判断，未在本项目实测。
- **【未验证】** 本轮没有条件或没有执行。

边界：工作树 `d:\project\1107-shots`，分支 `research/screenshot-paths`（从 `origin/main` 的 `63ea209` 切出）。未修改 `d:\project\1107` 工作树；未改 `crates/`、测试、基线；未跑全量 `cargo test --workspace`；未 commit、未 push、未动远端 issue。正文所有外部结论都来自源码读取与提交/标签元数据，不是二手转述。

---

## 0. 结论先行

### 0.1 一句话

**维持票 11 已裁决的 (d) IR 快照 + (a) Windows 人工捕获；CI 里不设像素基线。** 上游确实有离屏导出能力（不是"推测应该有"），但它不在 `0.2.2` 里，Windows 实现进入稳定版要等到 `v1.18.0`（2026-09-02），且只能从 `test-support` 构建里调用。**「`0.2.2` 没有」不等于「上游不存在」**：离屏导出能力存在于上游，Windows 实现首个可检出标签为 `v1.18.0`，`0.2.2` 只是没有。为一张静态地图截图跨 11 个月升级 gpui，不划算；CPU 光栅器只能出"近似图"，出不了可信的像素基线。

### 0.2 四条路径的最终状态

| 路径 | 本轮结论 | 关键依据 |
| --- | --- | --- |
| (a) OS 窗口捕获 | **保留：仅 Windows 人工证据，不进 CI** | 既有实测（主线工作树）exit 0、1212×835 非黑屏；依赖可见桌面会话与机器字体/DPI |
| (b) 切 gpui git 版本 | **否决：不为一纸截图升级** | 上游能力存在（见第 1 节），但破坏面、feature 门禁与 GPU 依赖都高于收益（见第 2 节） |
| (c) 场景 IR + CPU 光栅器 | **否决：双渲染路径 + 只能近似** | 像素级复刻需要同一套 DirectWrite 文本栈（第 3 节） |
| (d) 仅 IR 快照 | **保留：提交级门禁** | 现有 `scene_snapshot` 已覆盖结构；边界见 4.3 |

---

## 1. 问题 1：gpui 上游到底有没有离屏导出

### 1.1 结论

有，而且是公开 API，不是内部实验代码。先明确口径：本节说的"有"指上游源码中存在公开离屏导出 API；"`0.2.2` 没有"只描述该固定版本的实际状态，**不代表上游不存在这项能力**。Zed `main` 上至少有四种相关能力：

1. `Window::render_to_image()`：把**最近一帧的场景**渲染到离屏纹理并读回 `RgbaImage`。
2. `PlatformHeadlessRenderer`：无窗口的场景渲染器 trait，提供 `render_scene_to_image()` / `render_scene()`。
3. `Window::painted_quads()` / `painted_underlines()`：不落像素，直接取绘制图元做结构断言。
4. `VisualTestPlatform`：真实渲染 + 可控调度器的测试平台（`crates/gpui/src/platform/visual_test.rs:1-6` 的文档注释写明 "real rendering (macOs-only for now)"）。

但三个限制决定了它不能直接救 `0.2.2`：

- 这些 API 全部在 `#[cfg(any(test, feature = "test-support"))]` 或 `bench-support` 下，**发布构建默认没有**。
- Windows 的 `Window::render_to_image` 到 2026-08-26 才实现（commit `74490dac`），首个可检出它的稳定标签是本轮查到的 `v1.18.0`（2026-09-02）。
- `0.2.2` 的 `Scene` 是 `pub(crate)`；`main` 已改成 `pub`。这是跨版本的结构性变化，不是加一个函数。

### 1.2 公开入口与签名（Zed main）

| 能力 | 位置 | 签名/门禁 |
| --- | --- | --- |
| 窗口帧导出 | `crates/gpui/src/window.rs:3091-3098` | `#[cfg(any(test, feature = "test-support"))] pub fn render_to_image(&self) -> anyhow::Result<image::RgbaImage>` |
| 平台窗口导出 | `crates/gpui/src/platform.rs:1454-1460` | `fn render_to_image(&self, _scene: &Scene) -> Result<RgbaImage>`，默认实现 `bail!("render_to_image not implemented for this platform")` |
| 无窗口渲染器 | `crates/gpui/src/platform.rs:1463-1482` | `pub trait PlatformHeadlessRenderer { fn render_scene_to_image(&mut self, scene: &Scene, size: Size<DevicePixels>) -> Result<RgbaImage>; fn render_scene(...) -> Result<()>; }`，`#[cfg(any(test, feature = "test-support", feature = "bench-support"))]` |
| 取绘制图元 | `crates/gpui/src/window.rs:3112-3120` | `pub fn painted_quads(&self) -> Vec<Quad>`、`pub fn painted_underlines(&self) -> Vec<Underline>`，test-support |
| 取场景 | `crates/gpui/src/window.rs:3102-3105` | `pub fn rendered_scene(&self) -> &Scene`，bench-support |
| Scene 可见性 | `crates/gpui/src/scene.rs:41` | `pub struct Scene`（`0.2.2` 为 `pub(crate) struct Scene`） |

Windows 的真实实现不是空壳：

- `crates/gpui_windows/src/window.rs:1046-1052`：`#[cfg(feature = "test-support")] fn render_to_image(...)`，委托给 DirectX renderer。
- `crates/gpui_windows/src/directx_renderer.rs:407-470`：注释明确写"Render `scene` to an offscreen CPU image **without presenting** so the window need never be shown or visible"；流程是 `render()` → 建 `D3D11_USAGE_STAGING` 纹理 → `CopyResource` → `Map` → BGRA 转 RGBA。
- macOS：`crates/gpui_macos/src/window.rs:2401`。
- Linux：没有 `Window::render_to_image`；无窗口路线走 `gpui_wgpu::WgpuHeadlessRenderer`（`crates/gpui_wgpu/src/wgpu_renderer.rs:2559-2560` 实现 `PlatformHeadlessRenderer`），由 `gpui_platform::current_headless_renderer()` 选择（`crates/gpui_platform/src/gpui_platform.rs:100-121`；该函数在非 macOS/Linux 平台直接返回 `Ok(None)`，即 Windows 不走这条）。

### 1.3 引入版本（按标签二分，非推测）

| 版本/提交 | 日期 | `Window::render_to_image` | 平台覆盖 |
| --- | --- | --- | --- |
| crates.io `0.2.2` | 2025-10-22 | 无（全 crate 检索 0 命中，既有结论） | 无 |
| `v0.218.7` | 2026-01-12 | 无 | 无 |
| 提交 `558a741d`（PR #46324 "Visual test infrastructure improvements"） | 2026-01-09 | 新增（`crates/gpui/src/window.rs` 新增该公开方法） | 公开 API 先落地 |
| `v0.219.5` | 2026-01-20 | 有（`crates/gpui/src/window.rs:1831`） | 仅 macOS（`platform/mac/window.rs:1618`、`platform/mac/metal_renderer.rs:440`）；Windows 无 |
| `v1.16.3` | 2026-08-25 | 有 | Windows 仍无 |
| 提交 `74490dac`（PR #63012 "gpui_windows: Implement render_to_image for headless window capture"） | 2026-08-26 | 有 | Windows 首次实现 |
| `v1.17.2` | — | 有 | Windows 仍无 |
| `v1.18.0` | 2026-09-02 | 有 | Windows 有（`crates/gpui_windows/src/window.rs:995`） |
| `v1.27.0`（本轮查到的 latest 稳定标签） | 2026-11-04 | 有（`crates/gpui/src/window.rs:2916`） | Windows 有（`crates/gpui_windows/src/window.rs:1045`） |

证据说明：标签有无通过对 `raw.githubusercontent.com` 拉取对应标签的 `window.rs` 并检索 `fn render_to_image` 得到；引入提交通过 GitHub commits API 的 `path` + 时间窗查询得到。`74490dac` 的逐文件 patch 在查询时遇到 GitHub API 限流（403），但提交列表的 message 与 `v1.17.2`/`v1.18.0` 的文件存在性足以定位版本边界。【事实·外部】

#### 1.3.1 版本号口径的风险（未验证，不作结论）

crates.io 上 `gpui` 的最新版本是 `0.2.2`（2025-10-22），而同一上游仓库已出现 `v1.27.0` 这类标签（2026-11-04）。这说明「crates.io 版本号」与「上游 tag」不是同一套口径：**不能把 `0.2.2` 读成上游当前版本，也不能把 `v1.27.0` 读成 crates.io 已发布版本**。两者是否同源、`0.2.2` 对应哪个 tag、发布通道为何停留在 `0.2.2`，本轮**没有核实清楚**，只作为风险提示记录，不作为结论；后续引用版本号时必须写明来源是 crates.io 还是 git tag。

### 1.4 与 `0.2.2` 的差距

- `0.2.2` 把平台后端放在 `gpui` crate 内部（`src/platform/windows/...`）；Zed main 已拆成 `gpui_platform`、`gpui_windows`、`gpui_macos`、`gpui_linux`、`gpui_wgpu` 等独立 crate（`crates/` 目录列表）。应用创建方式也随之变为 `gpui_platform::application()` / `gpui_platform::headless()`（`crates/gpui_platform/src/gpui_platform.rs:16-22`），而不是 `0.2.2` 的 `Application::new()`。
- `0.2.2` 的 `Scene` 是 `pub(crate)`（`docs/acceptance-gaps.md` 2.2 节），`TestWindow::draw(&Scene)` 直接丢弃场景；main 的 `Scene` 已 `pub`，并且多了 `painted_quads` 这类"取图元不落像素"的测试钩子。
- 这不是"同一个 API 换了个版本号"，而是平台层重组 + 测试基础设施重写。

---

## 2. 问题 2：切 git 版本的代价

### 2.1 锁定策略

- Cargo 写法只能是 `gpui = { git = "https://github.com/zed-industries/zed", tag = "v1.18.0" }` 或 `rev = "<sha>"`。`--locked` 仍能工作，`Cargo.lock` 会钉住解析出的 git rev。
- 标签是可移动的；如果真要锁，应锁定 `rev`，而不是 `tag`。这与项目现有"`Cargo.lock` 已提交、`--locked` 构建"的纪律兼容，但样本从 crates.io 变成"整仓 git fetch"。
- `0.2.2` 的 `--locked` 语义不变；变的是依赖来源与拉取成本。

### 2.2 破坏面

- 平台 crate 拆分（见 1.4）：`Application` 构造、窗口创建、文本系统入口都可能要改。
- 截图能力只在 `test-support` 下：Windows 实现是 `#[cfg(feature = "test-support")]`（`crates/gpui_windows/src/window.rs:1046`），而 `gpui_platform` 的 `test-support` 才会把它转发给 `gpui_windows`（`crates/gpui_platform/Cargo.toml:23`：`test-support = ["gpui/test-support", "gpui_macos/test-support", "gpui_windows/test-support", "gpui_wgpu/test-support"]`）。也就是说，截图测试必须让 `gpui` 与 `gpui_platform` 两个依赖都开 `test-support`，而不是产品二进制默认具备。
- `gpui` 自己的 `test-support` 特性会拉入一批测试依赖（`crates/gpui/Cargo.toml:22-33` 的 feature 列表），测试构建的依赖图会比现在大。
- 版本跨度：`0.2.2`（2025-10-22）到 `v1.18.0`（2026-09-02）约 11 个月；到 `v1.27.0` 约 12.5 个月。中间包含平台重组、`Scene` 公开化、视觉测试基础设施重写。

### 2.3 构建时间

**未验证。** 本轮没有建 git 依赖探针，也没有编译 Zed 的 gpui。可以确认的成本因子：

- Cargo 的 git 依赖会拉取整个 `zed-industries/zed` 仓库，而不是只拉 `crates/gpui`；仓库体积与拉取时间本轮未测。
- 依赖图新增 `gpui_platform`、`gpui_windows`（Windows）、`gpui_wgpu`（Linux）等 crate；Windows 侧还引入 DirectX/windows-rs 绑定。
- 本轮环境 `git ls-remote https://github.com/zed-industries/zed.git HEAD` 返回 `b5cd310b...`，说明 `github.com:443` 当前可达（与项目早期记录的"不可达"不同）；但可达不等于能在 CI 稳定拉取，也不等于能在这台机器上编译通过。

### 2.4 许可证

`gpui_platform` 的 `Cargo.toml:6` 为 `license = "Apache-2.0"`；crates.io `gpui 0.2.2` 元数据同为 Apache-2.0。许可证不是阻碍。【事实·外部】

### 2.5 CI/运行时风险（比构建更关键）

- Windows 的 `render_to_image` 需要 D3D11 设备。`crates/gpui_windows/src/directx_devices.rs:106-140` 只枚举真实适配器，`142-193` 要求 Feature Level 10.1+ 且支持 `StructuredBuffer`；源码里没有 WARP 软件回退分支。
- 无 GPU 的 `windows-latest` runner 是否能枚举到可用适配器，**未验证**。如果不能，截图测试会在 CI 直接失败，而不是"图不理想"。
- 即使能出图，`render_to_image` 读回的是 GPU 渲染结果，仍受驱动/字体/DPI 影响；它解决的是"不需要可见窗口"，不解决"跨机器像素一致"。

### 2.6 值不值

**不值。** 理由按优先级：

1. 截图目标只是"一张静态分层地图"，不是逐帧动画或交互态；为它承担的是一条 11 个月的升级链。
2. 升级后截图仍只能作为 `test-support` 测试，不是产品能力；而且仍受 GPU/DPI/字体影响，做不了严格像素基线。
3. 项目红线要求"不引入与 GPUI 分叉的第二渲染路径"；升级本身不违反，但升级的收益（一次人工可替代的截图）与风险（pre-1.0 破坏性变更、CI 无 GPU）不对称。
4. 若将来因为别的原因（新 API、安全修复）计划升级到 ≥ `v1.18.0`，截图测试可以作为升级 PR 的附带收益，而不是升级的理由。

---

## 3. 问题 3：CPU 光栅器路线的真实成本

### 3.1 输入规模

场景 IR 的实际字段（`crates/arch-view/src/main.rs:512-519` 的 `SceneArtifact`、`crates/arch-view/src/layout.rs:63-111`）：

- `layout.width/height`
- `bands[]`：`layer`、`rect`、`nodeCount`、`zIndex`、`isUnclassified`、`isExternal`
- `nodes[]`：`id`、`layer`、`rect`、`labelRect`、`hitRect`、`zIndex`、`visualRole`、`isUnclassified`、`isExternal`
- `edges[]`：`source`、`target`、`directionAdjusted`、`from`、`to`、`zIndex`
- `overlapArea`、`overlapAreaThreshold`

三个样本的实际规模：

| 样本 | 模块/节点 | 边 | 画布 | 文本串数量 | 来源 |
| --- | --- | --- | --- | --- | --- |
| `tests/fixtures/layers` | 26 | 19 | 1200×800 | 8 层带 + 26 节点 = 34 | 已入库 `docs/evidence/11-scene-layers.json` |
| `privatenumber/tsx` | 136 | 379 | 1200×1184（按布局常量计算） | 约 139 | 主工作树未入库产物 `out/layers-tsx.json` + `layout.rs:127-142` 常量推算【推断】 |
| `excalidraw/excalidraw` | 802 | 4363 | 1200×5984（按布局常量计算） | 约 806 | 主工作树未入库产物 `out/layers-excalidraw.json` + 同上【推断】 |

说明：tsx/excalidraw 的数值来自主工作树已有的层产物（`out/` 未入库），画布高度按 `layout_map` 的 `band_header=24`、`band_padding=10`、`node_height=40`、`node_gap_y=10`、`band_gap=16`、每行 7 列计算；本轮没有对这两个样本实际跑 `arch-view scene`，所以标【推断】。

### 3.2 IR 不是渲染描述

IR 只描述布局，不描述外观。颜色、字号、圆角、边框、透明度、文本对齐、z-order 的绘制语义都在视图代码里：

| 绘制语义 | 现值 | 代码位置 |
| --- | --- | --- |
| 画布底色 | `0x0d1117` | `main.rs:378, 653, 963`；常量 `view.rs:17` |
| 层带底色 | 6 种 + unclassified/external 特殊色 | `view.rs:50-58`；`main.rs:1524-1527` |
| 层带分隔线 | 底部 1px | `main.rs:1542-1543` |
| 层带标签 | `text_xs`（12px，`0.75rem`），左上 padding 8/4 | `main.rs:1544-1551`；字号定义 `gpui-0.2.2/src/styled.rs:433-437` |
| 边颜色 | 强 `0xf2f5f8` / 方向修正 `0xd98a3d` / 普通 `0x3d4b5c` | `main.rs:1464-1470` |
| 边线宽 | 强 2px，普通 1px | `main.rs:1473` |
| 边虚线 | `[4px, 2px]`，仅方向修正边 | `main.rs:1474-1476` |
| 边透明度 | alpha 合成 | `main.rs:1445-1450` |
| 节点尺寸/间距 | 150×40；gap 12/10；圆角 4px；1px 边框 | `layout.rs:127-142`；`main.rs:1574-1588` |
| 节点填充 | 按层 6 色 + unclassified | `view.rs:33-47` |
| 节点标签 | 12px，垂直居中，左右 6px，超出裁剪 | `main.rs:1580-1596` |
| 节点角标 | 6px 圆点，右上内缩 3px | `main.rs:1599-1613` |
| 标签可见阈值 | `zoom >= 0.6` | `view.rs:14, 392-394` |
| 选中/邻居高亮边框 | 交互态，不在 IR | `main.rs:1506-1520` |
| z-order | 层带 < 边 < 节点 | `main.rs:1039-1142`；IR 的 `zIndex` 见 `layout.rs:175,195,252` |

这意味着 CPU 光栅器要么复制一套样式常量（第二份真值，必然漂移），要么扩展 IR schema 把颜色/字号/文本样式也写进去（契约变更 + 票 11/03 测试同步）。两条都不是"写个脚本"的量级。

### 3.3 口径 A：能出"近似图"

范围：只渲染默认视口（zoom 1、pan 0）的静态模块地图，不覆盖下钻、选择、播放、diff、动画。

需要的技术：

- 矩形/圆角矩形填充与描边、线段描边、虚线、抗锯齿、alpha 合成：`tiny-skia`（光栅化）与 `lyon`（路径细分）可直接覆盖。
- 圆形角标：同一条路径。
- 文本：`tiny-skia` 不带文本。需要 `fontdb` + `cosmic-text`/`swash` 自行排版、测量、裁剪；字号 12px，字体要按平台选（Windows 的 GPUI 用 DirectWrite 的系统 UI 字体，`0.2.2` 的 `src/platform/windows/direct_write.rs:1819-1833` 明确写 "Segoe UI"）。
- 标签居中与裁剪：要复刻 `flex + items_center + px(6)` 与 `overflow_hidden` 的盒模型行为，不能只按 `labelRect` 画。

代价口径（沿用 animation 研究的 S/M/L）：**S–M**。S 的部分（图元、虚线、AA）很小；M 的部分在文本排版/裁剪与样式真值同步。产出"人眼看起来像"，**不能**与 GPUI 输出做像素比较。

### 3.4 口径 B：能出"可用于像素基线的图"

要满足"CI 像素门禁"，必须让 CPU 输出与 GPUI 输出在像素级可比。缺口是结构性的：

1. **文本栈不同**：`0.2.2` 在 Windows 走 DirectWrite（`src/platform/windows/direct_write.rs`），CPU 路线通常走 cosmic-text/swash。字形选择、hinting、subpixel positioning、整像素舍入都不同。
2. **抗锯齿不同**：GPUI 缓存 subpixel variant（`src/text_system.rs:767-779`；`src/platform/windows/direct_write.rs:727-729` 的 `SUBPIXEL_VARIANTS_X/Y`），CPU 光栅器默认是灰度 AA。
3. **合成不同**：GPUI 的 alpha/边框/圆角最终由 GPU 管线合成；CPU 端即使逐图元一致，边缘像素也会不同。
4. **DPI/字体版本**：系统字体（Segoe UI）随 Windows 版本变化；DPI 缩放会改变所有舍入。
5. **样式真值不在 IR**：见 3.2，CPU 端还要再维护一份颜色/字号/布局常量。

结论：**L，而且不可靠。** 想真正消除 1–3，只能复用 GPUI 的文本系统与 GPU 渲染器——那就退化成 (a)/(b)，不再是 (c)。这与票 11 已写下的红线一致：选 (c) 必须"两条路径共享同一场景 IR；IR 变更必须同时更新两侧；发布级跑像素对照"，否则不接受（`docs/tickets/arch-tool/11-cross-platform-gpui-smoke.md:97-101`）。本轮评估认为这个约束下的维护成本高于收益。

---

## 4. 推荐

### 4.1 明确选择

**选 (d) + (a)：IR 快照做提交级门禁，Windows OS 捕获只做人工证据；CI 不设像素基线。** 这与票 11 在 2026-11-08 的裁决一致，本轮新增的上游证据不改变结论，只是把"为什么不用上游能力"从"没有"修正为"有，但不值得为它升级"。

### 4.2 被否方案的原话级理由

- (b) 的原话：`未选（否决：破坏版本固定）`（`11-cross-platform-gpui-smoke.md:93`）。本轮补充：`v1.18.0` 起 Windows 确实有 `render_to_image`，但只存在于 `test-support`，且需要 D3D11 适配器；版本固定被破坏后仍拿不到严格像素一致性。
- (c) 的原话：`未选（否决：双渲染路径）`（`11-cross-platform-gpui-smoke.md:94`）。本轮补充：IR 不含样式，CPU 端必然产生第二份样式真值；像素级对齐又需要复用 DirectWrite，等于绕回 (a)/(b)。
- (a) 的原话：`已选：仅 Windows 人工证据，不进 CI`（`11-cross-platform-gpui-smoke.md:92`）。保留，但必须写明它包含窗口边框（实测 1212×835 对应 1200×800 客户区），不能当像素基线。

### 4.3 结构断言覆盖到哪、覆盖不到哪

覆盖（已自动化）：

- 节点矩形、层带矩形、边的端点、标签包围盒、命中框、z-order（`crates/arch-view/tests/scene_snapshot.rs:46-116`）。
- 同输入两次运行字节一致（`scene_snapshot.rs:118-139`）。
- 布局不变量：同层节点重叠面积为 0（`layout.rs:227-234`；`tests/map_layout.rs`）。
- 交互模型：命中测试、缩放阈值、选择/邻居高亮、播放状态与透明度对比度（tickets 05/07/15/16/17 的模型断言）。

不覆盖：

- 光栅化、抗锯齿、圆角与边框的真实像素。
- 字体度量、字形选择、文本裁剪的实际结果（IR 只有 `labelRect`，没有文本宽度）。
- GPU 驱动差异、DPI 缩放、系统主题/字体版本。
- 动画的真实帧数与时长（`with_animation` 的最终态可以断言，逐帧像素不行）。
- 窗口边框/标题栏/阴影（`capture-window.ps1` 的 `GetWindowRect` 包含它们，`eval/capture-window.ps1:74-98`）。

### 4.4 如果将来一定要像素

最小可行条件（按代价排序）：

1. **先有升级理由**：把 gpui 升到 ≥ `v1.18.0` 作为独立升级 PR，截图只作为附带验证，不单独驱动升级。
2. 固定 `rev`（不是 `tag`）、固定 DPI、固定字体（随仓库分发字体或锁定系统字体版本）、固定窗口客户区尺寸。
3. 只在**自托管 Windows + 有 GPU** 的 runner 上跑；不阻塞主分支合并，只产出 artifact 与人工复核。
4. 用"像素差/结构相似度 + 容差"，而不是逐像素相等；阈值改动必须独立提交并记录旧读数/新读数（项目红线第 6 条）。
5. 如果上述任何一条做不到，就不要把它叫"像素基线"。

### 4.5 对票 11 文本的修订建议（本轮未改票）

- 把 (b) 的备注补成：上游已有公开离屏 API（`Window::render_to_image`），但只在 `test-support`、Windows 首个稳定版 `v1.18.0`、且需 D3D11；"破坏版本固定"的否决理由保持不变，升级仍不划算。
- 在 `docs/acceptance-gaps.md` 2.2 节保留旧结论，同时加一条"2026-11-08 复核：Zed main 已提供 `Window::render_to_image`，见 `docs/research/screenshot-paths.md`"，避免后续读者把"0.2.2 没有"误读成"上游没有"。
- 明确 (a) 的产物包含窗口边框，不能用于像素对比。

---

## 5. 做到的 / 没做到的 / 未验证的

### 做到的

- 定位并核实上游离屏导出入口：`Window::render_to_image`（`crates/gpui/src/window.rs:3091-3098`）、`PlatformHeadlessRenderer`（`crates/gpui/src/platform.rs:1463-1482`）、Windows DirectX 读回（`crates/gpui_windows/src/directx_renderer.rs:407-470`）、macOS Metal（`crates/gpui_macos/src/window.rs:2401`）。
- 用标签二分确定版本边界：`v0.218.7` 无、`v0.219.5` 有（仅 macOS）；Windows `v1.17.2` 无、`v1.18.0` 有；引入提交 `558a741d`（2026-01-09）与 `74490dac`（2026-08-26）。
- 核实 `0.2.2` 与 main 的结构差异（平台 crate 拆分、`Scene` 从 `pub(crate)` 变 `pub`）。
- 核实 feature 门禁与转发链（`gpui_platform/Cargo.toml:23`、`gpui_windows/src/window.rs:1046`）。
- 清点 CPU 光栅器需要复刻的绘制语义，附本地代码行号；给出三个样本的规模与画布尺寸。
- 给出明确推荐、被否方案的原话级理由与结构断言的覆盖边界。

### 没做到的

- 没有建 git 依赖探针，也没有实际编译/运行 Zed `main` 或 `v1.18.0` 的 gpui；因此"Windows 上 `render_to_image` 能出非黑图"仍未在本机验证。
- 没有写 CPU 光栅器原型，也没有生成任何 PNG；第 3 节的成本是源码清点后的估算，不是实测工时。
- 没有跑 `cargo test --workspace`（按约束，主线并行开发中）。
- 没有对 tsx/excalidraw 实际生成场景 IR；两行画布尺寸是按 `layout.rs` 常量推算。
- 没有修改票 11 或 `acceptance-gaps.md`（只给修订建议）。

### 未验证的

- git 版本的实际构建时间、依赖下载体积、`--locked` 下的解析稳定性。
- CI 的 `windows-latest` 是否能枚举到满足 `StructuredBuffer` 要求的 D3D11 适配器（`directx_devices.rs:142-193` 无 WARP 回退）。
- 在普通应用（非测试）里同时开启 `gpui/test-support` 与 `gpui_platform/test-support` 的实际编译结果与产物污染范围。
- `render_to_image` 在无可见窗口/无桌面会话（headless）下的真实行为；源码注释说可以，但没有运行证据。
- macOS/Linux 的离屏能力是否能在各自 CI runner 上稳定工作。

---

## 附录：本轮可复现的只读命令

```powershell
# 本轮未执行以下命令，仅记录复现方式（会触发编译）
cargo run -p arch-view -- scene tests\fixtures\layers --out docs\evidence\11-scene-layers.json
# 已入库产物读数（直接读取 docs\evidence\11-scene-layers.json）：
# bands=8 nodes=26 edges=19 width=1200 height=800
```

外部引用（均为固定标签/提交，路径可在对应 ref 下检索）：

- Zed `main`：`crates/gpui/src/window.rs`、`crates/gpui/src/platform.rs`、`crates/gpui/src/scene.rs`、`crates/gpui_windows/src/window.rs`、`crates/gpui_windows/src/directx_renderer.rs`、`crates/gpui_windows/src/directx_devices.rs`、`crates/gpui_platform/src/gpui_platform.rs`、`crates/gpui_platform/Cargo.toml`、`crates/gpui_wgpu/src/wgpu_renderer.rs`。
- 提交：`558a741d`（2026-01-09）、`74490dac`（2026-08-26）。
- 标签：`v0.218.7`、`v0.219.5`、`v1.16.3`、`v1.17.2`、`v1.18.0`、`v1.27.0`。
