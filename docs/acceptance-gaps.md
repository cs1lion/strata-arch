# 无法自动验证的 6 项验收：影响、替代口径与风险边界

生成时间：2026-11-07
适用范围：Round 2 验收标准制度中标注为「无法/暂无法自动验证」的 6 项。
状态：未写代码；本文所有命令除特别标注外均为计划形态，尚未在本项目执行。

> **更正记录（2026-11-07，后续实测定稿）**：本文原断言 `Window::render_to_image` 存在，经对 `gpui-0.2.2` 源码全量检索与本机编译运行验证推翻；gpui 0.2.2 **不存在公开的离屏导出入口**。详见第 2.2 节「更正记录」。ticket 03 的截图验收已移除并移交 ticket 11 重新定义。

## 0. 证据等级

| 等级 | 含义 | 本文中的例子 |
| --- | --- | --- |
| 已验证事实 | 本轮实际读取过源码/接口/元数据 | `gpui 0.2.2`、Apache-2.0、pre-1.0；`gpui::test` 的 `run_test` 存在；**gpui 0.2.2 无公开离屏导出入口**（原写 `render_to_image` 存在，已更正，见 2.2） |
| 推断 | 由已验证事实或平台常识推导，未运行验证 | 无头 CI 下 GPU 出图可能不稳定；字体度量差异会导致标签裁切；采样法可能漏掉内存尖峰 |
| 未验证 | 本轮没有环境或代码可跑 | 以下命令均未执行（注：GPUI 已在 2026-11-07 编译并起最小窗口，见 2.2） |

---

## 1. 峰值内存跨平台采集

### 1.1 验证不了的后果（推断）

- **会溜过去的缺陷**：索引阶段把整个仓库读入内存后，符号表、字符串、边列表存在多份副本；JSON 序列化又复制一遍；GUI 反复索引不释放。墙钟达标但峰值内存是产物的 3–10 倍，或存在泄漏。
- **暴露阶段**：用户机器内存不足时 OOM/卡死；GUI 第二次索引时越来越慢；CI 机器内存大，永远看不到。
- **代价**：高。若架构已按「全量内存图 + 一次性 JSON」定型，后期降内存要改数据结构、加流式写入或字符串驻留，属于伤筋动骨。

### 1.2 可落地的替代口径

| 口径 | 做法与命令 | 置信度 | 盲区 |
| --- | --- | --- | --- |
| **硬上限运行（推荐主判据）** | Linux：`bash -c 'ulimit -v 524288; ./target/release/arch-cli index <fixture>'`，退出码必须为 0；Windows 需要 Job Object 内存上限的小型 harness（**尚未编写，当前无可执行命令**）；macOS 用 `ulimit -v` 但语义与 Linux 不完全一致 | 高：能证明「不超过上限」；低：不能给出真实峰值 | `ulimit -v` 限制虚拟内存，mmap/分配器预留会导致误杀；Windows harness 未实现 |
| **分配器峰值（dhat）** | 加 `dhat` 依赖与 `dhat-heap` feature 后：`cargo run --release -p arch-cli --features dhat-heap -- index <fixture>`，读取生成的 `dhat-heap.json` | 中高：能定位 Rust 堆分配峰值与热点 | 不含 allocator 元数据、mmap、GPU/驱动内存；feature 尚未加入，当前不可执行 |
| **OS 采样峰值** | Linux：`/usr/bin/time -v ./target/release/arch-cli index <fixture> 2> mem.txt`，读 `Maximum resident set size`；macOS：`/usr/bin/time -l ...` 读 `maximum resident set size`；Windows：`Start-Process` 后每 50ms 读 `(Get-Process -Id $p.Id).WorkingSet64` 取最大值 | 中：真实 RSS，但 Windows 采样可能漏掉短尖峰 | 采样间隔决定精度；Windows 的 `PeakWorkingSet64` 在进程退出后不一定可读，需轮询 |
| **人工协议 + 证据** | 固定 fixture、固定 commit、冷启动，任务管理器/活动监视器截图，记录 3 次读数与机器规格，归档 `docs/evidence/` | 低：一次性旁证 | 不可重复、不可进 CI；只能证明「当时那台机器」 |

**Windows 采样命令（可执行，未在本项目跑过）**：

```powershell
$p = Start-Process -FilePath .\target\release\arch-cli.exe `
  -ArgumentList @('index','<fixture>') -PassThru -NoNewWindow
$peak = 0
while (-not $p.HasExited) {
  try { $ws = (Get-Process -Id $p.Id).WorkingSet64 } catch { break }
  if ($ws -gt $peak) { $peak = $ws }
  Start-Sleep -Milliseconds 50
}
"peak_working_set_bytes=$peak exit=$($p.ExitCode)"
```

### 1.3 「本轮接受风险」的边界

没有峰值内存判据时，只能在以下范围默认放过：

1. 样本仓 ≤1000 个文件；
2. 只跑单次冷索引，不跑 GUI 反复索引；
3. 不对外宣称「支持 5k 文件」或「低内存」。

一旦出现以下任一情况，**必须停下来人工确认，不得默认放过**：

- 要宣称支持 >2k 文件或 >20k 符号；
- 要发第一个发布版；
- 任何用户报告 OOM、索引卡死、GUI 反复索引变慢；
- 引入新的全量缓存/图副本/序列化路径。

### 1.4 成本、时机、决策

- **成本**：低（加 `/usr/bin/time` 或 PowerShell 采样 + 一个 fixture）；中（Windows Job Object harness + CI 矩阵）。
- **时机**：MVP 前定义内存预算与硬上限 fixture；MVP 后补 Windows harness；发布前跑三平台读数。
- **需要你决策**：是（a）内存预算取多少（建议先给 512MB 上限，超了再讨论）；（b）是否把「硬上限运行」作为合并门禁。

---

## 2. GPU 截图在 CI/无头环境的稳定性

### 2.1 验证不了的后果（推断）

- **会溜过去的缺陷**：标签被裁切、边穿过节点、层与层重叠、主题色错误、DPI 缩放后布局错位、动画结束态与静态态不一致。功能测试全绿，画面不可用。
- **暴露阶段**：演示或用户首次打开；GPU 差异导致「我这儿正常，你那儿错」。
- **代价**：中高。视觉问题往往要改布局/路由/标签算法，返工面大。

### 2.2 可落地的替代口径

> **更正记录（2026-11-07）**
>
> - 原文断言：「`Window::render_to_image() -> anyhow::Result<image::RgbaImage>` 存在于 `crates/gpui/src/window.rs`」。
> - 实测更正：**gpui 0.2.2 不存在公开的离屏导出入口**。对 `gpui-0.2.2` 整个 crate 源码检索 `render_to_image` 为 0 命中；`Scene` 为 `pub(crate)`；`TestWindow::draw(&Scene)` 直接丢弃场景，没有公开出图路径；crates.io 上最新版本仍为 0.2.2。
> - 本机验证（Windows）：`gpui = "=0.2.2"` 编译通过；最小窗口 `open_window` 成功，运行 2 秒后由 `cx.quit()` 干净退出。Windows 后端可用，截图能力不可用。
> - 影响：ticket 03 的截图验收已移除并移交 ticket 11；ticket 11 的截图路径为开放决策项（OS 捕获 / gpui git 版本 / 场景 IR + CPU 光栅器 / 仅 IR 快照）。
> - 本条更正用于防止旧断言被再次引用；后续引用截图能力时以本节为准。

| 口径 | 做法与命令 | 置信度 | 盲区 |
| --- | --- | --- | --- |
| **结构断言（推荐主判据）** | 对布局模型断言：节点矩形、边端点、标签包围盒、z-order、命中框、按角色的颜色。计划命令：`cargo test -p arch-view --test layout_assertions`（测试尚未编写） | 高：GPU 无关、确定性高、能抓布局/路由/标签错误 | 抓不到光栅化、抗锯齿、GPU 驱动差异、颜色管理 |
| **本机截图基线（当前不可用）** | 需要窗口帧导出；`render_to_image` 在 gpui 0.2.2 不存在，路径待 ticket 11 选型；计划命令：`cargo test -p arch-view --test render_golden -- --nocapture`（尚未编写） | 中高：能抓真实渲染回归 | 当前无公开 API；只在生成基线的机器/GPU 上可信；基线会随 GPUI 升级漂移 |
| **无头/CI 冒烟** | 只验证「能创建窗口/能出图/不崩溃」，不比对像素；计划 CI job `arch-view-render-smoke` | 中：证明可运行 | 不证明画面正确；无头 GPU/软件渲染可能直接失败 |
| **一次性平台探针** | 每个目标 OS 跑一次，归档 PNG、GPU 型号、驱动版本、窗口/DPI 设置到 `docs/evidence/` | 中：跨平台旁证 | 非连续，不能防回归 |
| **人工视觉协议** | 10 条固定检查项（标签不裁切、边不穿节点、层不重叠、选中态可见、缩放后不糊、动画结束态正确等），改前/改后截图对照归档 | 低到中：能抓明显问题 | 依赖人、不可批量 |

### 2.2.1 本地像素证据链（2026-11-08 追加，人工项，不进 CI）

`eval/pixel-evidence.ps1` 复用 `eval/capture-window.ps1` 的 Windows `PrintWindow` 抓屏，在可见桌面会话下跑八个状态并做像素断言。产物写 `out/pixel-evidence/`（被 `.gitignore` 排除，不入库）。命令：`pwsh -File eval/pixel-evidence.ps1`；最近一次实测 **24/24 通过**（`out/pixel-evidence/report.json`）。

| 状态 | 判定方式 | 阈值与依据 | 实测读数（2728×1879，DPI 216 / 2.25x） |
| --- | --- | --- | --- |
| 加载态非白屏 | 中心区（x 20–80%，y 25–75%）亮像素 + 文字行带；整帧平均亮度排除白屏 | 亮像素亮度 ≥200（深色主题正文 `#f3f3f3`，亮度 243）；≥600 像素、≥3 行带。依据：2.25x 下 14px 字号三行，每行至少约 200 像素；白屏平均亮度接近 255 | meanLum=24.67；bright=10012，bands=3，bbox=(1014,877)-(1694,1053)；抓取时刻 stdout 无 `load:done` |
| 失败态错误文案 | 中心区橙色（hue 范围 R140–255 / G60–160 / B20–100）像素计数 | ≥2000。依据：三行 CJK 错误文案按 14px/2.25x 估算每行 ≥300 像素、三行 ≥900，取约 2x 余量 | orange=17482，bbox=(689,851)-(2038,1082)；画面文案 `数据准备失败` / `git log 失败: fatal: cannot change to 'no-such-repo-for-evidence': No such file or directory` |
| diff 三色 | 精确色 ±8 的像素计数 | 每色 ≥2000。依据：单行 tint 约 2400×54≈1.2e5 像素，2000≈单行 1.5%，排除孤立抗锯齿像素 | `#2f6b3f`=461903 / `#7a3138`=306721 / `#7a5b24`=154087；合成 repo `added=3 removed=2 modified=1`（history fixture 的 `modified=0` 不足以证三色） |
| P1 上下文淡出前后帧 | 关闭态 vs 开启态（`STRATA_EVIDENCE_ANIMATION=1`）整帧帧差 | changedRatio ≥2% 且 32×32 热点单元（≥30% 变化）≥10。依据：16 节点中 13 个非路径节点透明度 0.25→0.68，节点瓦片+标签约占画面 10%，取 1/5 余量；13 个节点各至少覆盖 1 格 | changed=7.886%，meanAbs=1.256，maxCell=100%，hotCells=454，bbox=(37,142)-(2560,1816) |
| P3 播放推进 | autoplay 相邻帧帧差；另跑同状态静态基线 | 至少一对相邻帧 changedRatio ≥0.2% 且热点单元 ≥1。依据：单个时间线热力格约 0.4% 画面；静态基线噪声上限 0.1%，与 0.2% 拉开 2x | maxChanged=1.288%，maxCell=100%，hotCells=88；同 tick 对 0.000%；静态基线 changed=0.000000% |
| 搜索：有结果 / 无匹配（ticket 20，excalidraw 802 模块） | 左上角 30%×25% 区域内唯一底色 `#161b22` 的像素数与纵向范围 | 有结果与无匹配各 ≥20000 像素；有结果的底边比无匹配低 ≥200px（6 行命中 ≈310px 的保守下限） | off=0、results=115865、nomatch=97493；底边 448 vs 221（差 227） |
| 搜索：聚焦态含标签（ticket 20） | 右下角标签底色像素 + 正文白像素；左上角面板底色应归零 | 标签底色 ≥5000、正文白 ≥300；面板底色 ≤100 | 标签 bg=34154、text=2028（行 1767–1837）；面板 bg=0；stdout `phase: search:label module=external:@site/src upstream=3 downstream=0` |
| 搜索：清除后（ticket 20） | 同区域底色像素回到基线 | 面板/标签底色 ≤100 | 面板=0、标签=0（基线与清除后一致） |

**限制与事实边界（不当自动化包装）：**

- **揭示默认开启后的抓帧口径（2026-11-09）**：`map` 默认播放 900ms 时长驱动的分层揭示，任何在 `content:presented` 抓取的截图都会落到动画中途。证据链统一等 `phase: reveal:done`（`map-pixel-compare.ps1` 默认），`capture-window.ps1` 默认加 `--no-reveal`。需要中途帧或精确栅格时显式用 `--no-reveal`，或等待明确的阶段标记；`pixel-evidence.ps1` 的 map 抓帧都是路径播放态，播放时揭示本就关闭。

- **需要可见桌面会话**：无头 CI 不可用；整条链路登记为人工项，不进 CI 门禁。
- **抓屏进程必须 DPI-aware**：PowerShell 默认 DPI-unaware 时 `GetWindowRect` 返回虚拟尺寸（1212×835），而 `PrintWindow` 按物理像素渲染（2728×1879），截图右下被裁切，底部播放面板与居中状态文案全部丢失。脚本启动时调用 `SetProcessDpiAwarenessContext(-4)`（per-monitor v2）后才完整。这是脚本前置条件，不是可选项。
- **`PrintWindow` 动画首帧可能是空白帧**：实测开启动画后首帧只有标题栏可见、客户区全空，整帧平均亮度约 8.5（正常内容帧 24–30）。脚本用平均亮度 15 作为有效帧门限，跳过空白帧并在 `report.json` 的 `blankFrames` 记录编号，不把空白帧当作证据；本次复跑未出现空白帧，历史抓取曾出现 1 帧。替代方案：`capture-window.ps1 -Method screen` 走 `CopyFromScreen` 绕过 Direct3D 表面，或增大 `WarmupMs` 让后续帧稳定后再采。
- **按键注入方式（2026-11-09）**：`capture-window.ps1 -Keys` 用 `PostMessage(WM_KEYDOWN/WM_KEYUP)` 把按键送进窗口，走应用的 `on_key_down` 真实路径（stdout 有 `phase: key:*` 为证）。`SendKeys` 实测不进 GPUI 0.2.2 的键盘链（无任何 `key:` 标记），已弃用。注入前先 `SetForegroundWindow` 并在画布左上角点一下，让窗口激活且画布拿到键盘焦点。
- **搜索面板的隔离度量**：面板/标签与场景其余部分唯一的共同色是底色 `#161b22`，因此用「该色像素数 + 纵向范围」判断可见性。最初用「正文白像素相对基线的增量」，实测被面板自身遮挡底层节点标签抵消（无匹配面板增量 -997），不能隔离面板；改为底色口径后判据（三态可见）未变，只换度量方式。
- **读数会随 GPU / 字体 / 缩放漂移**：阈值仅用于本机人工证据，不设 CI 像素门禁。
- 本次未改动任何 `eval/` 期望值文件（`corpus.toml`、`baseline/current.json` 未变），因此无需独立的期望值变更提交。

### 2.2.2 Linux 窗口冒烟：xvfb + openbox + 软件渲染（2026-11-08 追加，已进 CI）

目标：把 ubuntu job 的窗口运行从「编译通过」升级为「运行时验证」。结论：**可跑通**，条件是「`xvfb` 虚拟显示 + 窗口管理器 `openbox` + Mesa lavapipe 软件 Vulkan」。

实测（CI run `42501294926`，ubuntu job，三次尝试同一次 job 内依次执行）：

| 尝试 | 条件 | 结果 |
| --- | --- | --- |
| default | xvfb，无 WM，默认图形 | 12 秒内无帧请求，`smoke: timeout waiting for first frame`，exit 2 |
| software | xvfb，无 WM，`LIBGL_ALWAYS_SOFTWARE=1` + `VK_ICD_FILENAMES=lvp_icd.json` | 30 秒内同样无帧请求，exit 2 |
| openbox | xvfb，WM=openbox，lavapipe | `smoke: frame rendered`，exit 0，约 2.1 秒 |

卡点定位（阶段标记 + `STRATA_SMOKE_LOG=1` 转发的 gpui 日志）：

- Vulkan/lavapipe 初始化成功：`Adapter: "llvmpipe (LLVM 21.1.8, 256 bits)"`，`VK_KHR_xcb_surface` 可用，surface present mode `MAILBOX`。
- 窗口创建成功，首次 draw 进入 `Render`（`smoke:render:enter`），随后 `smoke:window:opened`。
- 但 gpui X11 的帧请求循环要等窗口 `MapNotify` 后才启动；无 WM 的虚拟显示下该循环不启动，`on_next_frame` 永不回调。加 WM 后同一二进制立刻出帧。

定性：这是 **CI 环境限制（虚拟显示缺窗口管理器），不是本项目缺陷**；真实 Linux 桌面始终有 WM。若以后该路径失效，先看 `phase:` 标记：停在 `smoke:run:enter` 之前 = 客户端/GPU 初始化；停在 `smoke:window:opened` 之前 = 窗口/surface；`smoke:render:enter` 出现但无 `smoke:frame:callback` = 帧请求循环未启动（先查 WM/显示环境，而不是先改业务代码）。

边界：软件渲染不代表真实 GPU 表现；只证明「能开窗、能出帧、能干净退出」，不证明画面正确，不替代 2.2.1 的像素证据。CI 门禁为 exit 0 + `smoke: frame rendered`，未使用 `|| true` / `continue-on-error` / 放宽断言。

### 2.2.3 macOS 窗口冒烟：GitHub runner 实测通过（2026-11-09 追加，已进 CI）

目标：把 macOS 从「仅编译验证」升级为「运行验证」，且不新造判据——复用 `STRATA_PHASE_MARKERS`、`phase: content:presented` 与「exit 0 + 标记命中」。

步骤（`build (macos-latest)` job 内，`runner.os == 'macOS'`）：直接跑 `./target/debug/arch-view map tests/fixtures/layers --auto-close-ms 5000 --no-reveal`，输出落 `out/macos-window-smoke.log`；bash 看门狗 90s（macOS runner 无 GNU `timeout`），超时判 124。

**结果：PASS。** CI run `42535306096`，job `build (macos-latest)`，步骤耗时 **7s**（23:31:50 → 23:31:57）；该 job 总时长 3m33s（上一次 run `42534669345`）→ 3m50s，增量约 +17s（步骤本身 7s，其余为排队/波动）。

关键日志原文（`out/macos-window-smoke.log`，已去掉时间戳前缀）：

```
phase: main:start
phase: app:created
phase: app:entry
phase: window:callback
phase: frame:render
phase: window:created
phase: window:activated
phase: frame:presented
phase: map:prep:start
phase: map:prep:layers
phase: map:prep:callgraph
phase: map:prep:layout
phase: map:prep:done
phase: content:render
phase: content:presented
phase: quit:auto
== macos smoke: PASS (exit 0, content:presented) ==
```

环境：`macos-26-arm64`（macOS 26.7.2，runner image 20261102.0473.1）。

过程中唯一失败是 workflow YAML 解析错误（run `42535250496`：`run:` 块内嵌 Python heredoc 顶格，YAML 非法，job 数为 0），与本项目代码无关；改成纯 bash 看门狗后一次通过。

**边界**：只证明「能开窗、能出帧、干净退出」，不证明画面正确，也没有 macOS 像素证据（`pixel-evidence.ps1` 依赖 Windows `PrintWindow`）；不代表真实 Mac 的 GPU/显示器体验。ramp/颜色/DPI 差异仍未覆盖。

**判定**：这是 runner 上可复现的运行验证，不是环境限制；未出现「无 Aqua 会话 / 窗口创建失败 / `on_next_frame` 不回调」。

### 2.3 「本轮接受风险」的边界

只在以下范围默认放过：

1. 不改布局、边路由、标签排布、主题色；
2. 只跑结构断言，截图仅本机人工看一眼；
3. 不对外宣称「视觉无回归」。

以下任一情况**必须停下来人工确认**：

- 改动涉及布局算法、边路由、标签避让、DPI、主题；
- 要合并 UI 相关 PR，而结构断言覆盖不到该改动；
- 要发版或做对外演示；
- GPUI 版本升级（见第 6 项）。

### 2.4 成本、时机、决策

- **成本**：低（结构断言）；中（截图基线 + 图像 diff + 固定动画时钟）；高（CI GPU runner 或多平台基线）。
- **时机**：MVP 前先做结构断言；MVP 后补本机截图基线；发布前做平台探针。
- **需要你决策**：是（a）是否把结构断言作为 UI 合并门禁；（b）截图基线存仓库还是仅存 CI artifact；（c）像素差阈值（建议先只做「可见差」人工判，不做精确像素门禁）。

---

## 3. 动画流畅度

### 3.1 验证不了的后果（推断）

- **会溜过去的缺陷**：帧时间尖峰、掉帧、动画期间输入延迟；大图上路径播放卡成幻灯片，用户直接关掉动画。
- **暴露阶段**：真实用户硬件；开发机 GPU 快，测试机看不出来。
- **代价**：中。可能被迫降级为静态高亮，产品表达受损；若布局/绘制架构有性能问题，改动面更大。

### 3.2 可落地的替代口径

已验证事实：`Window::request_animation_frame` 与 `with_animation` 存在（源码/GPUI Book）。未验证：在 GPUI 渲染循环里埋点能否拿到可信的 CPU 帧时间；本轮没有运行。

| 口径 | 做法与命令 | 置信度 | 盲区 |
| --- | --- | --- | --- |
| **CPU 帧时间埋点** | 在 render 路径记录 `Instant`，固定 fixture/窗口/缩放，统计 p50/p95/p99；计划命令：`cargo test -p arch-view --test frame_budget -- --nocapture`（测试尚未编写） | 中高：能抓 CPU 侧卡顿 | 不含 GPU 提交/呈现时间；受机器负载影响 |
| **每帧工作量代理** | 统计每帧绘制的节点/边/文本数量、分配次数、离屏裁剪命中率；断言不超过预算 | 中：与硬件无关，能抓复杂度爆炸 | 不直接等于帧时间；代理阈值需先标定 |
| **受限实验（关动画对照）** | 同一 fixture、同一视角，动画开/关各跑 3 次，比较 p95 与输入延迟；归档读数 | 中：能证明动画本身的增量成本 | 仍受硬件影响；不能进 CI 硬判 |
| **GPU 计时（若可用）** | 用 GPU timestamp query 采集呈现时间；GPUI 是否暴露该能力**未验证**，本轮不承诺 | 高（若可用） | API 可用性未知；跨平台驱动差异 |
| **人工录屏** | 60fps 录屏，数掉帧/卡顿段，归档视频与机器规格 | 低 | 主观、不可批量 |

### 3.3 「本轮接受风险」的边界

只在以下范围默认放过：

1. 样本图 ≤500 节点/≤1000 边；
2. 动画默认关闭或可一键关闭；
3. 不对外宣称「60fps」「流畅」。

以下任一情况**必须停下来人工确认**：

- 要宣称 60fps 或支持 5k 文件规模；
- 动画默认开启；
- 用户报告卡顿/输入延迟；
- 引入粒子、逐节点独立动画等每帧 O(边数) 的绘制。

### 3.4 成本、时机、决策

- **成本**：低（埋点 + 一个 fixture）；中（CI 阈值 + 多次对照）；高（GPU 计时）。
- **时机**：MVP 前埋点并定预算；MVP 后进 CI；发布前在目标硬件做人工对照。
- **需要你决策**：是（a）帧预算取 16.7ms 还是 33ms；（b）动画默认开还是关；（c）允许动画的最大图规模。

---

## 4. 「用户能理解」的主观目标

### 4.1 验证不了的后果

- **会溜过去的缺陷**：图能画出来，但不能回答「这个函数属于哪层、从入口怎么走到它」；所有自动测试通过，产品目标没达成。
- **暴露阶段**：用户试用/发布后；越晚发现，交互与信息架构返工越贵。
- **代价**：高。可能推翻主视图与引导流程。

### 4.2 可落地的替代口径

| 口径 | 做法 | 置信度 | 盲区 |
| --- | --- | --- | --- |
| **任务型可用性测试（推荐主判据）** | 选 5 名目标用户，给固定仓库 + 3 个任务（找入口、定位函数所属模块、口述调用路径），记录成功率与耗时；准备标准答案；归档记录 | 高（对这几个任务） | 样本小、有学习效应；不能证明长期使用 |
| **路径产物结构断言** | CLI 输出路径 JSON，断言包含入口、层、调用链、注解；计划命令：`cargo run -p arch-cli -- path --entry <symbol> --target <symbol> --format json`（CLI 尚未实现） | 中：证明信息齐全 | 不证明人看得懂 |
| **本地行为指标（可选、默认不上传）** | 记录 time-to-first-answer、点击/缩放次数、引导路径使用率；仅本机文件 | 中低：能发现卡点 | 需要埋点与隐私设计；指标不等于理解 |
| **人工启发式检查** | 固定 10 条：入口可见、层名有依据、路径可逐步解释、`unclassified` 可见、无悬空边等 | 中：能抓明显缺失 | 依赖检查者水平 |

### 4.3 「本轮接受风险」的边界

- 在没有任务型测试前，对外只能称「可视化原型」，不能称「帮助上手」。
- 以下任一情况**必须停下来人工确认**：
  - 要对外演示/发布；
  - 要宣称 onboarding 闭环；
  - 引导路径的信息结构发生改动；
  - 用户反馈「看不懂/找不到」。

### 4.4 成本、时机、决策

- **成本**：低（定义 3 个任务与答案）；中（招募与执行测试）；高（多轮迭代）。
- **时机**：MVP 前定义任务与答案；MVP 后第一轮测试；发布前复测。
- **需要你决策**：是（a）目标用户是谁、能否找到 5 人；（b）成功率/耗时阈值；（c）是否接受本机匿名指标。

---

## 5. 跨平台视觉一致性

### 5.1 验证不了的后果（推断）

- **会溜过去的缺陷**：DirectWrite / CoreText / fontconfig 字体度量不同，导致标签裁切、换行差异、节点尺寸变化；DPI 125%/150% 下布局错位；颜色管理差异。
- **暴露阶段**：非主开发平台用户；发布后。
- **代价**：中。需要逐平台调布局与字体，可能引入平台分支。

### 5.2 可落地的替代口径

| 口径 | 做法与命令 | 置信度 | 盲区 |
| --- | --- | --- | --- |
| **跨平台结构断言** | 每个 OS 跑 `cargo test -p arch-view --test layout_assertions`，比较节点/标签包围盒是否落在容差内 | 高（对布局） | 不含光栅/抗锯齿/颜色管理 |
| **捆绑字体** | 应用内置固定字体，避免 fontconfig 差异；仍需验证各平台加载 | 中高 | 字体授权、包体增大；系统缩放仍会影响 |
| **DPI 矩阵** | 100%/125%/150%/200% 各跑一次结构断言 + 截图 | 中高 | 需要能设置 DPI 的环境 |
| **每平台截图基线** | 各平台固定窗口/字体/DPI 出图，单独基线 | 中 | 需要各平台 runner；GPU 差异 |
| **发布声明兜底** | 只声明已验证平台，其它平台标注「未验证」 | 高（诚实） | 不解决缺陷，只控制预期 |

可执行的基础命令（当前无代码，执行会失败）：

```bash
cargo build --release -p arch-cli -p arch-view
cargo test -p arch-view --test layout_assertions
```

### 5.3 「本轮接受风险」的边界

- 只验证 Windows 时，发布说明必须写「Windows only / 其它平台未验证」，不得写「跨平台」。
- 以下任一情况**必须停下来人工确认**：
  - 要宣称支持 macOS/Linux；
  - 收到非 Windows 平台的布局/字体问题；
  - 引入自定义字体、DPI 适配、平台条件编译；
  - 发布前。

### 5.4 成本、时机、决策

- **成本**：低（声明边界）；中（捆绑字体 + DPI 测试）；高（多平台 CI 与截图基线）。
- **时机**：MVP 前定支持矩阵；MVP 后做编译冒烟；发布前做截图与 DPI。
- **需要你决策**：是（a）首发支持哪些平台；（b）是否捆绑字体；（c）布局容差。

---

## 6. GPUI 升级回归

### 6.1 验证不了的后果（推断）

- **会溜过去的缺陷**：`cargo update` 拉到新 GPUI 后编译失败、API 行为变化、绘制/动画/命中测试悄悄改变；或长期不升级，卡在旧版本的 bug 与平台问题上。
- **暴露阶段**：依赖更新时或需要某个 GPUI 修复时；大版本跳跃会变成一次性大改。
- **代价**：中高。pre-1.0 破坏性变更没有 semver 保护，升级可能连带布局与交互回归。

已验证事实：`gpui` 当前 0.2.2、Apache-2.0；README 明示 pre-1.0 且版本间常有破坏性变更。

### 6.2 可落地的替代口径

| 口径 | 做法与命令 | 置信度 | 盲区 |
| --- | --- | --- | --- |
| **精确锁版本 + 锁文件** | `gpui = "=0.2.2"`，提交 `Cargo.lock`；CI 用 `cargo build --locked` | 高：防意外漂移 | 不解决主动升级 |
| **升级专用 PR + 回归套件** | 升级单独一笔，跑 `cargo build --locked`、`cargo test --locked`、结构断言、截图基线；归档改前/改后读数 | 高（对已覆盖项） | 截图/帧时间仍受平台限制 |
| **适配层隔离** | 把 GPUI 绘制/动画/命中测试包在自己的薄适配层，升级只改适配层；对适配层做结构单测 | 中高 | 适配层无法完全隔离行为变化 |
| **升级 runbook** | 固定步骤：读 CHANGELOG → 锁版本 → 编译 → 结构断言 → 截图 → 人工 10 项 → 记录版本；失败即回滚 | 中 | 依赖执行纪律 |
| **人工冒烟** | 启动、打开样本仓、播放路径、缩放、关闭动画，逐项记录 | 低到中 | 不可批量 |

可执行命令（当前无代码，执行会失败）：

```bash
cargo build --locked
cargo test --locked
```

### 6.3 「本轮接受风险」的边界

- 不允许无 PR 的 `cargo update`；不允许升级与功能改动混在同一提交。
- 以下任一情况**必须停下来人工确认**：
  - 编译失败或需要改公开 API；
  - 结构断言/截图出现无法解释的差异；
  - 升级跨多个 minor；
  - 发布前未记录实际测试过的 GPUI 版本。

### 6.4 成本、时机、决策

- **成本**：低（锁版本 + `--locked`）；中（升级回归套件 + runbook）。
- **时机**：MVP 前锁版本；每次升级执行；发布前记录版本。
- **需要你决策**：是（a）升级节奏（仅按需 vs 定期）；（b）谁批准升级 PR；（c）是否允许自动依赖更新。

---

## 7. 汇总

| # | 项目 | 主替代口径 | 成本 | 时机 | 需用户决策 |
| --- | --- | --- | --- | --- | --- |
| 1 | 峰值内存 | 硬上限运行 + OS/dhat 读数 | 低–中 | MVP 前定预算，发布前跑三平台 | 内存预算、是否门禁 |
| 2 | GPU 截图 | 结构断言为主，截图基线为辅 | 低–高 | MVP 前结构断言，发布前截图 | 是否门禁、基线位置、阈值 |
| 3 | 动画流畅度 | CPU 帧时间埋点 + 关动画对照 | 低–中 | MVP 前埋点，发布前人工对照 | 帧预算、默认开关、图规模 |
| 4 | 用户能理解 | 5 人任务型可用性测试 | 中 | MVP 前定任务，MVP 后测 | 目标用户、阈值 |
| 5 | 跨平台视觉 | 平台结构断言 + 声明边界 | 低–高 | MVP 前定矩阵，发布前截图 | 支持平台、字体、容差 |
| 6 | GPUI 升级 | 锁版本 + 升级 PR 回归 | 低–中 | MVP 前锁版本，每次升级 | 升级节奏、审批人 |

## 8. 本轮未验证清单

1. GPUI 已在 2026-11-07 编译并起最小窗口；`render_to_image` 在 0.2.2 中不存在，离屏出图能力尚未建立（见 2.2 更正记录）。
2. 未实现 CLI/索引器，所有内存与耗时命令仅为计划形态。
3. 未加入 `dhat` feature，dhat 路径当前不可执行。
4. Windows Job Object 内存上限 harness 未编写。
5. 未建立截图基线、结构断言、帧时间测试。
6. 未做任何用户测试。
7. 未测 macOS/Linux 字体与 DPI 差异。
8. 未实际执行 GPUI 升级流程。
9. （2026-11-08）ticket 15 的 P2 短镜头转场：GPUI 的 `with_animation(200ms)` 已用本机探针实测（11 帧 / 216ms），但 MapView 场景自身的逐帧序列未单独打点，且 `gpui 0.2.2` 无公开离屏导出，帧数无法进 CI 自动断言。
10. （2026-11-08）ticket 16 的 P1 上下文淡出：节点/边的角色透明度与标签对比度已用模型断言自动验证（0.68 下限最低 5.419:1）。原缺口「未从真实帧缓冲采样像素」已由本地像素证据链补上（关闭态 vs 开启态帧差 changed=7.886%、hotCells=454，见 2.2.1）；仍**未验证**布局节点不重叠时跨节点遮挡的实际视觉 z-order。
11. （2026-11-08）ticket 17 的 P4 双 commit 结构 diff：三色块 tint、120ms 入场淡入的最终态恒定与关闭态逐位一致已用模型断言自动验证。原缺口「未从真实帧缓冲采样」已补（合成 repo `added=3 removed=2 modified=1`；三色像素 461903/306721/154087，见 2.2.1）；`with_animation` 的实际帧数/时长仍**未打点**。
12. （2026-11-08）`eval/pixel-evidence.ps1` 像素证据链本身是**人工项**：需要可见 Windows 桌面会话、DPI-aware 抓屏进程，无头 CI 不可用；阈值只作本机证据，不同 GPU/字体/缩放下读数会漂移，不设 CI 像素门禁（见 2.2.1）。
13. （2026-11-16）`paired-ab` 判据式 CI 自测（job `paired-selftest`、夹具 `eval/baseline/paired-ab-selftest.json`、性质断言 PC1–PC4，口径见 `eval-harness.md` §6.1 第 6 条）的**守护范围之外仍是本机人工项**，三处边界：
    - **frame-timing 真实测量**：需可见 Windows 桌面会话 + `PostMessage` 按键注入 + release 二进制，无头 CI 跑不了。输入期读数（票 23 阶段 0 的 p95 34.80 / 36.39 / 38.27ms）只能本机复现；且实测同一命令相隔 15 分钟读数可漂移 30%（median 均值 10.3→13.38ms，+29.9%）到翻倍（地板 1.9→3.87ms，+103.7%；段间 std 0.603→1.103ms，+82.9%），**同命令复跑读数不同不得当复现成功**，须分开登记「行为未漂移」与「读数漂移」。
    - **`paired-ab` 读取 `out/**/pair*.json` 的路径与守卫逻辑**（`exitCode≠0` 即抛、`mapLine` 一致性、缺单侧不补）不在自测覆盖内——自测吃的是入仓夹具，不是真实 run 目录；这部分改坏只有真实跑汇总时才会暴露。
    - **像素证据链**沿用第 12 条边界，未因本次新增自测而改变。

    另记（同一事实的两侧）：`paired-selftest` 自 `1d382f7`、`fmt` 自 `029059f` 起**均从未在 GitHub runner 上真实执行**，本机 pwsh 跑通不等于 ubuntu runner 行为一致；两者在首次 push 产生 CI 运行记录之前一律按未验证处理。
14. （2026-11-16）`ENGINEERING-DISCIPLINE.md` §5 第 7 条「登记类数字必须先脚本回读」目前是**人工判据，无自动化检查**，与 §10.6 第 1 项裁决 A「新增规则优先落成检查或工具，文档只留指针」尚未对齐。缺口内容：应做 `eval/cite-check.ps1`——输入一张「文档中的数字 ↔ 源数据字段」引用登记表，逐条断言①文档出现的值与源值相等（数字规范化后比较，不靠字符串巧合）、②引用的 SHA 可 `git rev-parse --verify`、③源路径存在；源在 `out/**` 时 CI 不可见，必须记为 skipped 并显式报告，**不得因 skipped 而让整个检查返回通过**。该工具自身还须按票 24 的 negative control 要求自证能变红：故意写错一个登记数字必须使检查失败。在工具落地之前，本条规则的遵守完全依赖执行者自觉——本会话已出现四类会骗过自觉的取证失效形态（批次归属误记两例、幅度写成区间估计一例、文档行号引用因插入而漂移一例、`rg -q`／`Measure-Object -Line`／ParserError 三类命令自身给出假读数），故登记为流程缺口而非小事。
15. （2026-11-16，追加更正；第 14 条原文保留不改）第 14 条所登记的「未工具化」状态**已失效**：`eval/cite-check.ps1` + 登记表 `eval/baseline/citations.json` 已落地（`ff9e31c`），CI job `cite-check` 已加入（`9f81f03`），与 §10.6 裁决 A 的对齐状态由「未对齐」变为「已对齐」。

    首批覆盖范围与实测（本机）：登记 36 条——`json-field` 14（票 23 更正表 7 组 × 两侧 `meanMs`，源为入仓夹具）、`derived` 3（§8 第 13 条三个百分比，只登记源字段与 `pct_change` 算式）、`json-field` 3（阶段 0 三次 p95，源在 `out/`）、`pattern-count` 8、`sha` 6、`file-exists` 2；其中 `ciVisible=true` 30 条。本机执行 `PASS=36 FAIL=0 SKIP=0`（退出码 0）；`-SelfTest` 三处预置坏登记全部被抓（漏抓集为空）；用临时副本构造 6 条 SKIP 验证语义：默认 `PASS=30 FAIL=0 SKIP=6` 退出码 0，加 `-RequireAllResolved` 则 `FAIL=1` 退出码 1（临时副本已删，未入库）。

    **仍属人工项 / 未验证的边界（本条的实质内容，不得因工具上线而视为已闭环）**：

    1. **未登记数字不受保护**：工具只声称「已登记项一致」。登记表自身的完整性（新增高风险数字是否同步加项）仍是**人工判据**，无检查——遵守依据为纪律 §5 第 7 条。历史文档既有数字未审计，属已知盲区。
    2. **CI 真实执行未验证**：`cite-check`（`9f81f03`）、`paired-selftest`（`1d382f7`）、`fmt`（`029059f`）三个 job 均未在 GitHub runner 上跑过；本文件第 13 条对后两者的表述同样适用于 `cite-check`。
    3. **CI 上有 6 条 SKIP**：`out/**` 源在 runner 不可见，只在本机参与判定；`-RequireAllResolved` 未启用，启用条件是这些源入仓或改由入仓夹具覆盖。
    4. **`-SelfTest` 刻意不进 CI**：其 `derived` 坏登记依赖 `out/` 源，在 runner 上只会 SKIP 而被抓不到，放进 CI 会把「环境限制」误报成「工具无效」（假红）。因此「工具能变红」这一证明目前只有本机读数。
    5. **字面耦合脆弱**：文档改写措辞（不改数值）也会使 `needle` 失配而 FAIL；合规处置只能是同步更新 `needle`，**禁止删除登记项或把 `expect` 改成刚好等于实得的值**。
    6. **`derived` 精度依赖舍入约定**：`pct_change` 取 1 位小数、`signed3` 取 3 位并转 U+2212；若文档改用不同位数，FAIL 是正确行为而非可豁免噪声。
16. （2026-11-16）`cite-check` 的**保护强度边界**（由注入实验实证，不是推演）：`pattern-count` 类登记项只守护「关键语句存在且命中数不变」，**不守护语义等价**。实证做法：在不删除、不改写任何既有语句的前提下，向 `eval/paired-ab-selftest.ps1` 的副本叠加一行 `$invariantOk = $true` 短路掉强制失败——结果 `cite-check` 对该副本仍 `条目=54 PASS=54 FAIL=0`、退出码 0，且被削弱的脚本自身退出码 0、照样打印「不变量 实际检查项=138 计划=138 => 成立」。结论与处置：本套登记项的有效性**必须**与 `-SelfTest` 三处坏登记、`PASS+FAIL+SKIP==条目数` 不变量、以及行为级注入共同使用；单独依赖登记项会形成「CI 绿 = 实现可信」的误读，属已知缺口而非缺陷。反向补充证据（说明守护确实有效的一面）：删除式注入 `cite-check` 的不变量计算行会被 `cc-invariant-formula` 抓到（退出码 1）；把计划值降级为回报值会被 `pass-no-downgrade-to-reported` 抓到（退出码 1）。
    另记本轮新撞的一个取证坑（尚未写入纪律 §5 第 7 条，待裁决）：PCRE2 的 `$` 行尾锚对 `Set-Content` 写出的**CRLF 副本**行命中为空，而入仓文件不含 CRLF（已用 `ReadAllText` 验证）——用带尾锚的模式断言「某语句不存在」会产生假阴性，误读成语句被删。核查语句在位一律改用无尾锚模式或直接读原文逐字比对。
17. （2026-11-16）`eval/injection-selftest.ps1`（元测试，`c21d4c5` 落盘、`ba411b7` 并入 CI）的覆盖边界与已知脆弱点：
    - **已覆盖的削弱手法（6 类）**：活路径漏计逃逸（C2）、不变量被叠加短路（C3/C3b 把「抓不到」固化为断言）、`Get-RgCount` 恒返 1（C4）、`-SelfTest` 的 `failIds` 被清空（C5）、`expect` 被抬高（C6）、基线绿前置校验（C1a/b/c）。
    - **未覆盖**：只测了 `$invariantOk = $true` **这一种**短路写法。改写 `if (-not $invariantOk)` 条件本身、篡改 `Get-PairedStats` 函数体、把断言整体注释掉等手法，元测试仍看不见；要覆盖需引入代码签名/哈希基线这类机制，本仓库尚未有该约定。
    - **锚点脆弱**：C2 依赖「PC1 循环之后的第一个 `$Checked.Value++`」定位（当前实测行号 81），`paired-ab-selftest.ps1` 结构性重构后锚点可能落空——脚本对此的处理是 `throw`（按失败计），不会静默通过，但会表现为元测试红而非「跳过」。
    - **元测试自身有效性**已用反证方式验证（`-PassScript` 喂入被短路基准时 C2 由 PASS 翻 FAIL、整体退出码 1），但该反证本身也只覆盖一种攻击形式，不构成「元测试可证明检查器健全」的一般性保证。
    - **落盘过程中撞到的新取证坑（尚未写入纪律 §5 第 7 条，待裁决入册）**：`Read-Text $p -split "`n"` 会被解析成**向函数传名为 `-split` 的参数**，而不是对返回值做数组运算；后果是锚点定位拿到整文件字符串、`drop=-1`，若再用 `$L[-1]` 就命中末元素（与 §5 第 7 条坑族叠加），产出「注入未生效但退出码符合预期」的假证据。合规写法是先括住调用 `@((Read-Text $p) -split "`n")`。
    - **CI 真实执行仍未验证**：`injection-selftest` 步骤自 `ba411b7` 起从未在 GitHub runner 上跑过；连同 `fmt`（`029059f`）、`paired-selftest`（`1d382f7`）、`cite-check`（`9f81f03`），本会话新增的 4 个门禁全部只有本机证据。

18. （2026-11-17）`eval/ps-param-lint.ps1`（第 10 道门禁，`bd6688d` 落盘并接入 `fmt` job）的覆盖边界。它是为拦纪律 §5 第 7 条第九类坑而建的 AST 静态检查——`Func $x -split $y` 会被解析成「把 `-split` 当作 `Func` 的命名参数」，语法合法、不报错、静默退化成单值，产出的是错读数而非异常。规则两条：D1 = 调用仓库内定义的函数时携带该函数自身 `param` 未声明的命名参数；D2 = 任何调用携带运算符关键字形状的命名参数。当前 HEAD 实测：扫描 18 个 `.ps1`，函数 seen 71 / 有自身声明 69 / 无声明 2，param 槽位形式 A 127、形式 B 17（两者均须非 0），重名参数槽位 4，`hits=0`、退出码 0。
    - **不检外部命令与未收录名字**：`rg`、`git`、原生 cmdlet 拿不到声明表，D1 不适用，只有 D2 生效。跨文件调用已覆盖（两遍法先全库建表，故 `lib-paired-stats` 那类定义在他处、调用在本处的函数可查），但经 `-Command` / `Invoke-Expression` 的动态调用完全在射程外。
    - **不检无声明的函数**：HEAD 实测 2 个函数自身没有 `param`，D1 依 `decl.Count -gt 0` 跳过它们——既不报警也不报错，属已知放过。
    - **重名函数取并集**：同名函数在多处定义时声明参数取并集（HEAD 实测重名参数槽位 4），因此「按另一处定义的签名传参」不会被拦。
    - **`-FileList` 子集模式不跑覆盖自检**：这是给元测试注入用例留的口子（单文件探针天然只含一种声明形式）；代价是该模式下退出码 0 不代表全库覆盖成立。CI 走 `git ls-files` 发现模式，两条覆盖自检（两种声明形式均已入索引、索引非空）仅在发现模式生效。
    - **只拦「运算符被吞成参数」这一族**：多余位置参数、变量名拼写错误、`$($a).Count` 一类其它形式的静默降级不在覆盖内。
    - **实证削弱结果（本轮重跑的新鲜读数，非引用旧值）**：把形式 B 分支改成 `if ($false)` 后，HEAD 实测 `有自身声明=56 / seen=71 / 无声明=15 / 形式 B 槽位=0`，D1 覆盖率降至 56/71；该副本被门禁自身的 `SELF-CHECK FAIL(coverage-form-index)` 以退出码 2 拦下，而元测试 C7 在同一破坏下由红转绿、令 `eval/injection-selftest.ps1` 整体退出码 1（其余 11 例不受影响）——两条防线各自独立生效。口径提示：本文件与提交信息中出现过的 56/66 是门禁脚本自身尚未入库时的读数，分母自 `bd6688d` 起变为 71。
    - **文本守护天花板（承接第 16 条）**：`eval/baseline/citations.json` 现有 82 条登记项（`pattern-count` 53，其中 `literal=true` 12；`ciVisible=true` 76），含 13 条 `pl-*`（2 条指向 `ci.yml` 的接线与步骤名、11 条指向门禁脚本文本）与 12 条 `meta-*`（全部指向 `eval/injection-selftest.ps1`，其中 `meta-landing-readback` 为 `expect=3` 的计数守护）。这些只守「文本在位」，抓不到「保留原文而另加早退/重排」的语义削弱。真正有牙的是 C7/C8/C9 的行为断言与上述破坏副本反证，而**反证目前仍是本机一次性动作，未进 CI**。
    - **PSScriptAnalyzer 路线未采用、其能力未验证**：本机 `Get-Module -ListAvailable PSScriptAnalyzer` 查无此模块；GitHub runner 是否预装经检索未确认；采用它需增加 `Install-Module` 并依赖 PSGallery 出网，取包失败即整条门禁不可用。**其内置规则能否拦此写法本轮未能验证**——不采用的理由是可得性与依赖面，不是「它做不到」。
    - **CI 真实执行仍未验证**：`ci.yml` 现有 7 个 job，其中 `fmt` 4 步、`paired-selftest` 2 步、`cite-check` 3 步；本会话累计向 CI 新增 5 个门禁步骤（`fmt check`、`powershell param lint`、`paired-ab selftest`、`cite-check`、`injection selftest`），全部从未在 GitHub runner 上执行，本机 pwsh 跑通不代表 runner 行为一致。第 17 条末句「本会话新增的 4 个门禁」是其落盘当时的读数，现累计为 5 个步骤，原文按规矩保留不改。

19. （2026-11-17）自指守护 `reg-count-selfguard`（`744d772`）与「登记面改动必须 0 deletions」硬约束的**冲突口径**，裁决取 A 方案。冲突事实：该登记项的 `expect` 绑定登记表条目数，本笔落盘前实测为 `rg 字面 "id": " 命中 92 = ConvertFrom-Json 条目 92 = 登记 expect 92`（三者一致）；因此**今后任何向 `eval/baseline/citations.json` 追加条目的动作，都必须同步修改该数字**，而这一行修改在 `git diff --numstat` 上必然表现为 1 deletion——纯插入地扩充登记表在机械上已不可能。这不是实现缺陷，是自指设计的固有代价，故在此登记处置口径而非留作口头默契。
    - **A 方案（采纳）**：保留自指项。今后追加登记项时，允许且**仅**允许那一处 `expect` 数字变动，并必须在提交信息中显式声明这是对「0 deletions」的偏离，声明内容包含三件套：变更前后 `expect` 数值、修改后 `rg` 回读的实际条目数、`cite-check` 全量读数（且 `条目=` 数必须与新 `expect` 相等，不等即说明漏改或错改）。
    - **不接受的规避方式**：为使 diff 保持 0 deletions 而少加守护项；把 `expect` 改成与实得不符的值后靠删条目凑数；改 `-SelfTest`/判据绕过计数。三者都属于「调整判据以让它通过」，与既有禁止项同源。
    - **B 方案（放弃）**：删除自指项以恢复纯插入。放弃理由：这会让「静默摘掉守护」重新变成无人可见，正是本项要堵的缺口，等于为便利放弃防线。
    - **C 方案（放弃）**：改为存在性守护（只钉首条/末条 id 在位）。放弃理由：不需随追加变动，但抓不到中间任意条目被删，防护强度显著低于计数式。
    - **本条不改变的残留极限**：删掉 `reg-count-selfguard` 自身仍无任何机制报警（检查者随被删对象消失，见第 18 条同名子项）；要再上一层需外部计数器或哈希基线，本仓库尚无该约定。
    - **适用范围**：自 `744d772` 起生效；下一次对 `citations.json` 的追加即须按本条声明。对其它文件（含本文件）的追加不受影响——本笔即为反例：向 `acceptance-gaps.md` 追加不触碰任何计数 needle，登记项 `gaps-8-13-present` 与 `gaps-8-14-present` 的 `expect` 仍各为 1。

20. （2026-11-17）`eval/drift-gate.ps1 -SelfTest` 的残余边界（`cb0e660` 落盘 S1-S4，`ef9ab13` 增 S5 并接入 CI，`870b894` 补 15 条 `dg-*` 登记项）。落盘实测：`verdict=PASS`、`cases=5`、不符合 0、整体退出码 0；S1 子进程 exit 0、S2 exit 1（`newViolations=2`，构成为 `reverseEdge` + `cycle` 伴生）、S3 脚本级 `drift gate: FAILED` 字样成立、S4 exit 0 且仓内基线 SHA256 前后一致、S5 内层 exit 1 且抓到 S2、深度守卫生效。登记表条目 107。
    - **只覆盖 self 目标**：负控跑的是 fixture 副本配仓内 `$selfBaseline`；corpus 那条 `Invoke-DriftGate` 分支（`eval/baseline/drift-*.json` 配 `tests/corpus/**`）**没有负控**，其判红能力仍未证。
    - **CI 里带条件，未触发即不跑**：该步骤条件是 `always() && steps.plan.outputs.required == 'true'`，与 `build arch-cli` 同条件（selftest 前置是那个二进制存在；若无条件运行，会在「本次改动不触及 drift 输入面」的推送上因缺 exe 直接 `exit 2`，成为假红）。故「负控在 GitHub runner 上成立」**只在 gate-required 的推送上有证据**，其它推送连跑都不跑。
    - **平台耦合**：`$Exe` 默认值带 `.exe`，现状 `drift` job 为 `windows-latest` 才成立；该 job 若迁到 Linux 必须显式传 `-Exe`，否则一律 `PRECHECK exit 2`。
    - **退出码语义不可跨模式混用**：`-SelfTest` 下 0 = 全例符合、1 = 报警或任一例不符、2 = 前置不满足；而返回 1 有两种成因（门禁真失效 / 自测自身失真），码本身不区分，必须读逐例输出。同文件另有两处不同语义：基线改动且语料不可用走 `coverage: blocked` 并 `exit 1`；`-RequireCorpus` 且语料缺失按配置错误 `exit 2`。
    - **归档目录可能混层**：内层反证运行若与外层同秒启动，会复用同一个 `out/drift-selftest-<runId>/`。正确性由「S1-S4 的结论在派生内层之前就已求值完毕」保证（脚本顺序），但**该目录的文件计数不可当独立证据**，引用读数一律以 stdout 的逐例行为准。
    - **S5 的牙齿有前置**：S5 只有在本脚本真被带 `-SelfTest` 调用时才生效，而该调用点在 CI 里带上面那个条件。「反证的反证」到此为止；再往上需要外部计数器或哈希基线，本仓库尚无该约定（同第 19 条自指极限）。
    - **文本守护天花板延续**：`dg-*` 15 条只守文本在位，「保留 needle 原文另加早退」的语义削弱仍不可见（同第 16、18 条）。
    - **第 19 条 A 口径的首次实际执行（记录形态，非新缺口）**：`870b894` 追加 15 条时把自指项 `expect` 由 92 改为 107，产生 **1 处 deletion**；提交信息按要求附三件套——变更前后 expect（92→107）、落盘后 `"id": "` 行数回读 107 且与 `ConvertFrom-Json` 条目数一致、`cite-check` 条目=107 等于新 expect；并用 hunk 头 `@@ -257 +257,31 @@` 证明删除仅该行一处、其配对新增行按 `"expect": <N>` 归一化后逐字相同。本笔只动本文件、不追加登记项，未触发该口径。
    - **未验证**：`drift` job 是否有 GitHub runner 的真实执行历史本轮仍未取证；本条全部读数均来自本机 Windows。票 24 阶段 1 第 3 条要求的「归档命令与退出码」目前只落在被 gitignore 的 `out/` 里，未随证据入仓，故该条仍未闭环。
    - （2026-11-17 追加）上一条中「未随证据入仓、该条仍未闭环」是其落盘当时的读数，已被 `ff734a2` 取代：逐例命令、退出码、环境 provenance 与原始 stdout 现已归档为 `docs/evidence/24-drift-selftest-20261117.json`（`docs/evidence/` 沿用既有 `<票号>-<标签>.json` 命名），票 24 阶段 1 第 3 条的归档动作就此闭环；该文件由登记项 `ev-drift-selftest-json` 守护（`pattern-count`，`expect=1`，取落盘后回读），删文件或清空成 stub 都会使 `cite-check` 变红。**仍未闭环的部分**：该登记项只证「文件在且非空」，不校验其中读数与真实运行一致；`out/drift-selftest-*` 运行目录本身仍不入仓，日后无法用目录复核，只能以该文件内嵌的 `rawStdout` 为准。附带效果：本条把档名写进 tracked 文档后，此前不可构造的 `file-exists` 守护（要求另一文件出现该档名字面）从此可用，属后续笔可选项。

21. （2026-11-17）票 24 阶段 2 前置能力（`eval/ab-skip-selftest.ps1`，`111e8e5`）与其读数的残余边界。已成立的部分：`RenderSkip` 参与同进程段轮换（`c2c4d46`）、未知 AB 段名不再静默回落（`dff5b38`）、配对规则与七例判据入仓可重放（`111e8e5`，判据复用 `eval/lib-paired-stats.ps1::Get-PairedAnalysis`，`K=2`、同号门槛 0.8334 沿用 §6.1 第 6 条，脚本内不自定阈值）。**未闭环之处**：
    - **样本规模未达票面口径**：票面阶段 2 第 2 条要求「≥12 段 × 60 帧」，本轮实为 12 对配对、每批 20 帧（19 个间隔）。本笔确立的是「判据可重放 + 方向稳定」，不是票面口径下的完整测量；按票面跑需 `-SegmentFrames 60` 且重复数使配对数 ≥12，可执行但**尚未执行**。
    - **效应量跨批未复现，禁止被引用为收益数字**：两独立批次 median `mean(gap)` 4.368ms → 6.578ms、p95 6.491ms → 13.551ms（约 2 倍），而显著性、方向、同号数、同配置负对照不显著、顺序偏置未报警全部一致。据 §6.1 第 8 条只可宣称方向与显著性稳定。
    - **凭证断言的绑定强度有限**：`frame-timing` 的 RED 判据是「`ab:skip` 总条数 ≥ 降格段边界数」加「cycle 声明的每个 spec 都出现」，**未逐条绑定第 i 个边界与第 i 条凭证的同序性**；且 `ab-skip-selftest` 的复用模式（`-PositiveJson/-NegativeJson`）不重跑子进程，故不重校验凭证，只提示依赖产生批次。
    - **配对留下的未配对实例是常态而非缺陷**：实例总数为奇数时必剩 1（本轮两批均为 1，段名 `BA`）。脚本已显式报告，但**任何把这 1 个实例静默丢弃后再宣称 N=12 以外口径的做法都属篡改样本量**。
    - **顺序偏置自检在负对照上必然空转**：同配置对照两侧段名相同，不存在 variantFirst 分组，其 `detected=False` 不含信息；非空转要求只适用于正对照（§6.1 第 8 条已立此规）。
    - **不可进 CI 且 runner 侧零证据**：需可见 Windows 桌面与 release 构建；连同 `fmt`、`paired-selftest`、`cite-check`（含元测试步骤）、`ps-param-lint`、`drift gate selftest`，本会话新增的 6 个门禁步骤全部从未在 GitHub runner 执行。
    - **该自测脚本自身无登记项守护**：删除它或改写其判据文本不会使 `cite-check` 变红；其运行摘要只落在被 gitignore 的 `out/ab-skip-selftest-*/summary.json`，未随证据入仓。
    - （2026-11-17 当日追加）本条首项「样本规模未达票面口径」已补齐，原文保留：按票面第 2 条以 `-SegmentFrames 60 -Cycles 6` 重跑（24 段实例、12 对、每批 60 帧），`ab-skip-selftest` 九例全 PASS、退出码 0。正对照 p95 `mean(gap)=9.053ms` 对门槛 3.264ms、同号 12/12、rel 49.02%；median `mean=6.226ms` 对门槛 1.704ms、同号 12/12（按 §6.1 第 8 条只记录不判方向）；同配置负对照 median `+0.012ms` 对门槛 0.515ms（同号 5/12）、p95 `0.657ms` 对 2.299ms（同号 6/12）两侧均不显著；顺序偏置 delta 0.872ms ≪ std 1.476ms 且两组各 6（非空转）；能力凭证 `want=13 got=26 specs=[none skip:edges]`。**由此得到的新证据**：批长 20→60 帧使正对照 median 的 `std(gap)` 从 2.122/2.364ms 降到 1.476ms、门槛从 2.451/2.729ms 降到 1.704ms，负对照门槛从约 1.2–1.4ms 收到 0.515ms——即 §6.1 第 7 条 (b)→(a) 类的分辨率转化，「不显著」自此对同配置对照具备排除该量级效应的含义。**仍未闭环**：效应量跨批波动仍达 2 倍（median mean 4.368/6.578/6.226ms；p95 6.491/13.551/9.053ms），故只可用于验证判据分辨力，**不得**作为产品收益结论；真实降格档（路线 C 常开）的收益与代价仍归票 23 阶段 C/C′，本条不涉及；该自测与其余 5 个新增门禁步骤一样，从未在 GitHub runner 执行（本需可见 Windows 桌面，不进 CI）；`ab-skip-selftest.ps1` 与其摘要仍无登记项守护、未随证据入仓。
