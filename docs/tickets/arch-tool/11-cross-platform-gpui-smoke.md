# 11: 跨平台构建与 GPUI 启动冒烟

**What to build:** 三个平台都能以固定 GPUI 版本、`--locked` 构建；Windows 上应用能启动窗口、渲染一帧并干净退出；macOS/Linux 至少编译通过。把 GPUI 的 pre-1.0 破坏性变更挡在合并前。截图路径已裁决：IR 快照进提交级门禁，Windows OS 窗口捕获只作人工证据（见「截图路径裁决」）。

**Blocked by:** 03（分层地图，提供可渲染的最静态视图）

**Status:** partial（2026-11-08：Windows 构建 + 窗口冒烟在 CI 通过；macOS/Linux 仅 CI 编译通过，未运行窗口；CI 已实际执行，见 run 42475740482。同日追加本地像素证据链，人工项 14/14 通过，不进 CI）

**固定样本：** `tests/fixtures/tiny`；启动冒烟使用 `--smoke` 模式。

> **事实更正（2026-11-07）**：本票原文写「Windows 上应用能启动窗口、渲染一帧并离屏截图非空」，并把 `Window::render_to_image` 当作已存在原语。实测推翻该结论：对 `gpui-0.2.2` 整个 crate 源码检索 `render_to_image` 为 0 命中；`Scene` 为 `pub(crate)`；`TestWindow::draw(&Scene)` 直接丢弃场景；crates.io 最新版本仍为 0.2.2。**gpui 0.2.2 不存在公开离屏导出入口。** 该结论来自后续源码检索与本机验证，详见 `docs/acceptance-gaps.md` 第 2.2 节。
>
> 本机已验证（Windows）：`gpui = "=0.2.2"` 编译通过；最小窗口 `open_window` 成功，运行 2 秒后由 `cx.quit()` 干净退出。**Windows 后端可用，截图能力不可用。**

## 验收项

- [x] 依赖固定：`gpui = "=0.2.2"`（推荐值，**待用户确认升级节奏**）；`Cargo.lock` 已提交；CI 所有构建使用 `cargo build --locked --workspace`。本地 `cargo build --locked --workspace` exit 0；CI 已实际执行（2026-11-08 run 42475740482 三平台全绿）。
- [x] Windows：`cargo run --locked -p arch-view -- smoke tests/fixtures/tiny` 在 3 秒内退出码 `0`；进程无 panic；窗口创建成功并可渲染一帧（由启动日志或冒烟标记断言）。实测二进制墙钟 828ms（debug 构建），stdout `smoke: layout 3 modules, 2 edges, 1 bands` 与 `smoke: frame rendered`。（**判据修正 2026-11-08 / ticket 14**：3 秒预算经 20+20 次压测判定不成立，改为 15 秒；原读数保留，理由见 `14-flaky-smoke-timeout.md`。）
- [x] macOS 与 Linux：`cargo build --locked --workspace` 在 CI 通过（2026-11-08 run 42475740482，`ubuntu-latest` 与 `macos-latest`）；启动冒烟未执行。**平台矩阵已裁决 2026-11-08：Windows 主验收；macOS/Linux 仅编译验证，运行未验证。**
- [x] 依赖更新受控：CI 配置中不存在自动依赖更新任务（dependabot/renovate）；`Cargo.lock` 仅在单独 PR 中变更（断言变更 PR 只含依赖相关文件）。仓库无 `.github/dependabot.yml` / `renovate.json`；`Cargo.lock` 已跟踪。
- [x] 失败判据：`--locked` 构建失败、Windows 启动崩溃或超预算（原 3s，ticket 14 修正为 15s）、窗口未能创建、锁文件被隐式改写。
- [x] 截图能力：按 2026-11-08 裁决执行为「IR 快照进 CI + Windows OS 捕获人工证据」。IR 快照 `arch-view scene` 输出节点/边/层带/z-order/命中框，同输入两次字节一致；`eval/capture-window.ps1` 为手动脚本，不参与 CI。**mac/Linux 截图不做，未验证。**
- [x] 自动化状态：构建与 Windows 启动冒烟自动化；IR 快照确定性断言自动化（`cargo test -p arch-view --test scene_snapshot`）；OS 窗口捕获为手动证据，不进 CI；mac/Linux 运行与截图未验证。

## 验收记录（2026-11-08，B 档）

### 断言先于实现

- 先写 `crates/arch-view/tests/smoke.rs`（4 条清单 + 2 个测试），再实现 `smoke` 子命令。
- Red（实现前实测）：`cargo test -p arch-view --test smoke` → 1 failed / 1 passed；失败原因为退出码 `2` 与 stderr 只有 `map` / `timeline` 用法，缺少 `smoke: frame rendered`。
- Green：实现 `run_smoke` 与 `SmokeView` 后 2 passed / 0 failed。

### 实现要点

- `arch-view smoke <repo>`：构建层产物与布局，打开 1200×800 窗口；`Render` 首次调用时用 `Window::on_next_frame` 注册「本帧渲染完成后」回调，回调打印 `smoke: frame rendered` 并 `cx.quit()`。
- 2.5s 兜底定时器：未渲染则打印 `smoke: timeout waiting for first frame` 并退出；`Application::run` 返回后检查 `AtomicBool`，未渲染则进程 exit 2。（ticket 14 已替换为 12s 独立 OS 线程看门狗，原因见该票取证：GPUI 执行器阻塞时原定时器不触发。）
- 缺仓库参数或多余参数退出码 `2`，stderr 给用法。

### 读数（实测）

- `cargo test -p arch-view --test smoke`：2 passed / 0 failed（1.81s）。（旧读数；ticket 14 修后单跑 20 次 2.50–4.04s、0 失败。）
- 二进制：`target/debug/arch-view.exe smoke tests/fixtures/tiny` → exit 0，墙钟 828ms（debug 构建），stdout 两行（布局读数 + 帧标记），stderr 空。
- `cargo build --locked --workspace`：exit 0。
- `cargo test --workspace`：117 passed / 0 failed；`cargo fmt --all --check` 通过。
- 依赖控制：`gpui = "=0.2.2"`；`Cargo.lock` 已跟踪；无 dependabot/renovate 配置。

### 判据修正（2026-11-08，ticket 14）

- 原判据：Windows 冒烟在 3 秒内 exit 0。
- 旧读数：单次 828ms（ticket 11 首测）。
- 修前实测：单跑 20 次 3 失败、最慢成功 2.62s；并行负载 20 次 18 失败；负载下到帧 6.51–8.66s。
- 新判据：15 秒内 exit 0；二进制看门狗 12 秒。
- 理由：3 秒没有并行负载与分布依据，余量仅 0.38s；15 秒对实测最慢 8.66s 留 1.73x 余量，仍能抓住挂死。详见 `14-flaky-smoke-timeout.md`。

### 已知偏差与未验证

- **CI 已实际执行（2026-11-08）**：run [42475740482](https://github.com/cs1lion/strata-arch/actions/runs/42475740482) 三平台构建全绿；Windows 冒烟原文 `smoke: layout 3 modules, 2 edges, 1 bands` / `smoke: frame rendered`。此前 main 的 Ubuntu 构建长期红，原因与修复见下节。「锁文件不一致即失败」仍未单独实测。
- **mac/Linux 运行未验证**：CI 只做 `cargo build --locked --workspace`，不启动窗口；gpui 运行时还需要图形会话/驱动，未在 runner 上验证。
- **截图路径已裁决（(d)+(a)）**：IR 快照做提交级门禁，Windows OS 捕获仅作人工证据；mac/Linux 不截图。
- **`cargo run` 计时口径**：3 秒预算按已构建二进制的墙钟计；`cargo run` 的冷编译时间不计入。

## CI 平台构建修复（2026-11-08）

### 事实

- main 上 32 次 run 全部 failure，唯一失败 job 为 `build (ubuntu-latest)`；从第一次 run [42428089452](https://github.com/cs1lion/strata-arch/actions/runs/42428089452)（`f1f3c44`，即引入 CI 的同一次 push）起就红，不存在「曾绿过」的区间。
- 失败原文（run 42472801703）：

```
error[E0425]: cannot find value `ENOATTR` in crate `::libc`
 12 |               pub const ENOATTR: ::libc::c_int = ::libc::ENOATTR;
```

- 依赖链：`arch-view → gpui 0.2.2 → gpui_http_client 0.2.2 → zed-async-tar 0.5.0-zed → xattr 0.2.3 → libc`。`xattr 0.2.3` 在 Linux 上引用 `libc::ENOATTR`；`libc 0.2.189` 仍提供（deprecated，值等于 `ENODATA`），`libc 0.2.190/0.2.191` 已移除。

### 修复

1. `Cargo.lock` 将 `libc` pin 到 `0.2.189`（`xattr` 只能停留在 0.2.x，`zed-async-tar` 的 `xattr = "0.2"` 无法升到 1.x）。
2. Ubuntu build job 安装系统库：`libxcb1-dev libxkbcommon-dev libxkbcommon-x11-dev`；缺库时链接原文为 `rust-lld: error: unable to find library -lxcb` / `-lxkbcommon` / `-lxkbcommon-x11`。

### 验证

- 本地（Windows）：`cargo check --locked -p arch-view` exit 0；Linux 目标单独交叉 `cargo check --target x86_64-unknown-linux-gnu`（`xattr 0.2.3 + libc 0.2.189`）exit 0。
- CI：run [42475740482](https://github.com/cs1lion/strata-arch/actions/runs/42475740482) 全绿；Ubuntu 日志原文 `Downloaded libc v0.2.189` / `Compiling xattr v0.2.3` / `Finished dev profile ... in 2m 06s`；macOS `2m 13s`；Windows `4m 49s` + 冒烟两行。

### 平台口径（以此为准）

| 平台 | 构建 | 启动/运行 | 状态 |
| --- | --- | --- | --- |
| Windows | CI 通过 | 窗口冒烟通过（CI） | **已验证** |
| macOS | CI 通过 | 未运行 | **仅编译验证** |
| Linux | CI 通过（需系统库） | 未运行 | **仅编译验证** |

## 截图路径裁决（2026-11-08，已执行）

用户裁决：**仅 IR 快照 + Windows 下补一次 OS 窗口捕获作为人工证据**；mac/Linux 截图不做。

### IR 快照（提交级门禁）

- 入口：`arch-view scene <目标仓库> [--out <场景文件>]`；产物 `schemaVersion = 1`，含 `repo`、`layout.width/height`、`bands[]`（layer/rect/zIndex/nodeCount）、`nodes[]`（id/layer/rect/labelRect/hitRect/zIndex）、`edges[]`（source/target/kind/from/to/zIndex）。
- z-order 固定：层带 < 边 < 节点；命中框 `hitRect` 等于节点矩形（当前无额外热区）。
- 测试（A 档，先 Red 后实现）：`cargo test -p arch-view --test scene_snapshot` 3 passed / 0 failed；实现前实测 1 passed / 2 failed（`scene` 子命令不存在，退出码 2）。
- 确定性读数：`arch-view scene tests/fixtures/layers --out docs/evidence/11-scene-layers.json` 两次运行字节一致；文件 23,368 字节，SHA256 `4FE2B0EE68C3DF7ADD33EED3E0C610496575547FDB0DB2581D16372360AF4516`。
- 覆盖边界：只证明布局模型（矩形、端点、层带、z-order、命中框），不覆盖光栅化、抗锯齿、字体度量与 GPU 驱动差异。

### Windows OS 窗口捕获（人工证据，不进 CI）

- 脚本：`eval/capture-window.ps1`（手动执行；启动 `arch-view map`，取主窗口句柄，优先 `PrintWindow`，失败回退屏幕捕获）。
- 实测（Windows，2026-11-08，**DPI-unaware 运行**）：`pwsh -File eval/capture-window.ps1` → exit 0，窗口读数 1212×835，产物 `docs/evidence/11-window-capture.png`（1212×835），SHA256 `DA6733FE135332AB7D0DDE292A5DE240385876E86613BBDDCBF7B6598FBAC7AE`。该读数来自 `GetWindowRect` 的虚拟坐标；`PrintWindow` 实际按物理像素渲染，图被右下裁切，底部面板与居中状态文案丢失，**仅作历史记录，不再作为完整画面证据**。
- DPI 修正（2026-11-08 追加）：脚本启动时调用 `SetProcessDpiAwarenessContext(-4)`（per-monitor v2）。修正后实测 DPI 216 / 2.25x，窗口 2728×1879，加载/失败文案、地图播放面板、时间线刻度条完整可见。抓屏进程 DPI-aware 是硬前置条件。
- 像素证据链（2026-11-08 追加，人工项，不进 CI）：`pwsh -File eval/pixel-evidence.ps1` 复用本脚本抓取加载 / 失败 / diff / P1 / P3 五个状态并做像素断言，最近一次 **14/14 通过**；判定方式、阈值依据与实测读数见 `docs/acceptance-gaps.md` 第 2.2.1 节。`PrintWindow` 动画首帧可能是空白帧（整帧平均亮度约 8.5，正常帧 24–30），脚本以平均亮度 15 剔除并记录编号。
- 不进 CI：需要可见桌面会话，无头 runner 不可靠；每次运行像素受字体/DPI/GPU 影响，不作为门禁。

### 未验证

- macOS/Linux 启动冒烟：CI 未运行，未验证。
- 像素级差异基线：未建立 CI 门禁；本地五状态像素断言已建立（见 `docs/acceptance-gaps.md` 2.2.1），但读数随 GPU/字体/缩放漂移。

## 截图能力重新定义（2026-11-07 开放项，2026-11-08 已裁决）

目标：在不引入「第二套与 GPUI 分叉的渲染路径」的前提下，获得可自动断言、可复现的视觉产物。2026-11-08 用户裁决 **（d）IR 快照 + （a）Windows 人工捕获**；下表保留原评估与状态。

| 路径 | 做法 | 判据 | CI 可用性 | 与「不允许双渲染路径漂移」的冲突 | 状态 |
| --- | --- | --- | --- | --- | --- |
| **(a) OS 窗口捕获** | Windows 用 `PrintWindow`/DWM 抓取真实窗口像素；macOS/Linux 用各平台窗口捕获 | 固定窗口尺寸与布局下，PNG 非空、不同颜色数 >10；与基线做像素差 | 低：需要可见窗口与桌面会话；无头 CI 不可靠，多 GPU/DPI 不稳定 | 低：抓的就是 GPUI 真实输出，没有第二条渲染路径；但「离屏」语义不成立 | **已选：仅 Windows 人工证据，不进 CI** |
| **(b) 切换 gpui git 版本验证离屏 API** | 改用 Zed 仓库的 gpui git revision，检索并验证是否存在公开离屏导出入口 | 能产出 `RgbaImage`/PNG，结构断言与像素基线可复用 | 中：取决于上游 API 与 CI GPU 条件；需锁定 revision | 低：仍为 GPUI 单一渲染路径；但破坏 `=0.2.2` + `--locked` 固定策略，需改 spec 与升级策略 | 未选（否决：破坏版本固定） |
| **(c) 场景 IR + 确定性 CPU 光栅器** | GPUI 继续负责交互窗口；另建与布局模型同源的场景 IR，用 CPU 光栅器（如 tiny-skia）生成 PNG | 场景 IR 的节点/边/层带与布局模型逐项一致；PNG 结构断言通过；像素测试确定性可复现 | 高：纯 CPU，无头可用，跨平台一致 | **高**：形成两条渲染路径，存在漂移风险；必须用「同一场景 IR 驱动两条路径」或定期像素对照来约束 | 未选（否决：双渲染路径） |
| **(d) 仅 IR 级快照，不落像素** | 导出确定性场景 IR（JSON/二进制）作为快照产物，断言节点矩形、边端点、标签包围盒、z-order、命中框 | IR 快照字节级稳定；字段与布局模型一致 | 高：纯数据，无头可用 | 低：不新增渲染路径；但无法发现光栅化、抗锯齿、字体度量、GPU 驱动差异 | **已选：提交级门禁** |

**与「不允许双渲染路径漂移」的约束说明：**

- (a) 与 (b) 复用 GPUI 的真实输出，不产生第二条渲染路径；主要风险是 CI 稳定性与版本固定。
- (c) 明确会产生第二条渲染路径。若选 (c)，必须同时满足：两条路径共享同一场景 IR；IR 变更必须同时更新两侧；提交级跑 IR 结构断言，发布级跑 GPUI 与 CPU 光栅器产物的像素对照；否则不接受。
- (d) 不产生第二条渲染路径，但只覆盖布局/结构，不覆盖真实像素。

**原待用户决定的开放项（2026-11-08 已裁决）：**

1. 选 (d) 作为提交级门禁 + (a) 作为 Windows 本机人工证据；不选 (b)/(c)。
2. (c) 未采用，因此「共享场景 IR」约束不触发。
3. (b) 未采用，`gpui = "=0.2.2"` 固定策略不变。
4. 截图产物：IR 快照 `docs/evidence/11-scene-layers.json`（确定性，入库）；OS 捕获 `docs/evidence/11-window-capture.png`（人工证据，入库一次）；两者都不设像素差阈值。
5. 无头 CI 不作为截图硬门禁；IR 快照测试可在无头环境跑，OS 捕获仅 Windows 本机手动。
