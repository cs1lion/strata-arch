# 11: 跨平台构建与 GPUI 启动冒烟

**What to build:** 三个平台都能以固定 GPUI 版本、`--locked` 构建；Windows 上应用能启动窗口、渲染一帧并干净退出；macOS/Linux 至少编译通过。把 GPUI 的 pre-1.0 破坏性变更挡在合并前。截图能力为开放决策项，见「截图能力重新定义」。

**Blocked by:** 03（分层地图，提供可渲染的最静态视图）

**Status:** partial（Windows 冒烟实测通过；mac/Linux 与 CI 执行未验证；截图开放项未选定）

**固定样本：** `tests/fixtures/tiny`；启动冒烟使用 `--smoke` 模式。

> **事实更正（2026-11-07）**：本票原文写「Windows 上应用能启动窗口、渲染一帧并离屏截图非空」，并把 `Window::render_to_image` 当作已存在原语。实测推翻该结论：对 `gpui-0.2.2` 整个 crate 源码检索 `render_to_image` 为 0 命中；`Scene` 为 `pub(crate)`；`TestWindow::draw(&Scene)` 直接丢弃场景；crates.io 最新版本仍为 0.2.2。**gpui 0.2.2 不存在公开离屏导出入口。** 该结论来自后续源码检索与本机验证，详见 `docs/acceptance-gaps.md` 第 2.2 节。
>
> 本机已验证（Windows）：`gpui = "=0.2.2"` 编译通过；最小窗口 `open_window` 成功，运行 2 秒后由 `cx.quit()` 干净退出。**Windows 后端可用，截图能力不可用。**

## 验收项

- [x] 依赖固定：`gpui = "=0.2.2"`（推荐值，**待用户确认升级节奏**）；`Cargo.lock` 已提交；CI 所有构建使用 `cargo build --locked --workspace`，锁文件不一致即失败。本地 `cargo build --locked --workspace` exit 0；CI 配置已加，**CI 实际执行未验证**。
- [x] Windows：`cargo run --locked -p arch-view -- smoke tests/fixtures/tiny` 在 3 秒内退出码 `0`；进程无 panic；窗口创建成功并可渲染一帧（由启动日志或冒烟标记断言）。实测二进制墙钟 828ms，stdout `smoke: layout 3 modules, 2 edges, 1 bands` 与 `smoke: frame rendered`。
- [ ] macOS 与 Linux：`cargo build --locked --workspace` 通过，启动冒烟为推荐项（**平台矩阵待用户确认**：推荐 Windows 主验收，mac/Linux 编译 + 启动冒烟）。**未验证**：无 mac/Linux runner；CI 已配置两平台但未实际运行。
- [x] 依赖更新受控：CI 配置中不存在自动依赖更新任务（dependabot/renovate）；`Cargo.lock` 仅在单独 PR 中变更（断言变更 PR 只含依赖相关文件）。仓库无 `.github/dependabot.yml` / `renovate.json`；`Cargo.lock` 已跟踪。
- [x] 失败判据：`--locked` 构建失败、Windows 启动崩溃或超 3s、窗口未能创建、锁文件被隐式改写。
- [ ] 截图能力：**当前未验证，不得记为通过**；路径选定后另加验收项（见下节）。
- [x] 自动化状态：构建与 Windows 启动冒烟自动化；mac/Linux 运行依赖可用 runner（**待用户确认**）；截图与视觉回归能力当前不可用（见 `docs/acceptance-gaps.md`）。

## 验收记录（2026-11-08，B 档）

### 断言先于实现

- 先写 `crates/arch-view/tests/smoke.rs`（4 条清单 + 2 个测试），再实现 `smoke` 子命令。
- Red（实现前实测）：`cargo test -p arch-view --test smoke` → 1 failed / 1 passed；失败原因为退出码 `2` 与 stderr 只有 `map` / `timeline` 用法，缺少 `smoke: frame rendered`。
- Green：实现 `run_smoke` 与 `SmokeView` 后 2 passed / 0 failed。

### 实现要点

- `arch-view smoke <repo>`：构建层产物与布局，打开 1200×800 窗口；`Render` 首次调用时用 `Window::on_next_frame` 注册「本帧渲染完成后」回调，回调打印 `smoke: frame rendered` 并 `cx.quit()`。
- 2.5s 兜底定时器：未渲染则打印 `smoke: timeout waiting for first frame` 并退出；`Application::run` 返回后检查 `AtomicBool`，未渲染则进程 exit 2。
- 缺仓库参数或多余参数退出码 `2`，stderr 给用法。

### 读数（实测）

- `cargo test -p arch-view --test smoke`：2 passed / 0 failed（1.81s）。
- 二进制：`target/debug/arch-view.exe smoke tests/fixtures/tiny` → exit 0，墙钟 828ms，stdout 两行（布局读数 + 帧标记），stderr 空。
- `cargo build --locked --workspace`：exit 0。
- `cargo test --workspace`：117 passed / 0 failed；`cargo fmt --all --check` 通过。
- 依赖控制：`gpui = "=0.2.2"`；`Cargo.lock` 已跟踪；无 dependabot/renovate 配置。

### 已知偏差与未验证

- **CI 未实际执行**：`.github/workflows/ci.yml` 只在本地落盘，没有远端运行记录；「锁文件不一致即失败」依赖 CI 的 `--locked`，未经实测。
- **mac/Linux 未验证**：没有可用 runner；gpui 在 Linux 需要系统图形依赖，能否在 `ubuntu-latest` 直接构建未知。
- **截图开放项未选定**：下方 (a)/(b)/(c)/(d) 仍需用户决策；本票不以截图作为通过条件，「渲染一帧」只由帧后回调标记证明，不代表像素正确。
- **`cargo run` 计时口径**：3 秒预算按已构建二进制的墙钟计；`cargo run` 的冷编译时间不计入。

## 截图能力重新定义（2026-11-07，开放项）

目标：在不引入「第二套与 GPUI 分叉的渲染路径」的前提下，获得可自动断言、可复现的视觉产物。以下路径均**未选定**，需要用户决策；未选定前本票不以截图作为通过条件。

| 路径 | 做法 | 判据 | CI 可用性 | 与「不允许双渲染路径漂移」的冲突 | 状态 |
| --- | --- | --- | --- | --- | --- |
| **(a) OS 窗口捕获** | Windows 用 `PrintWindow`/DWM 抓取真实窗口像素；macOS/Linux 用各平台窗口捕获 | 固定窗口尺寸与布局下，PNG 非空、不同颜色数 >10；与基线做像素差 | 低：需要可见窗口与桌面会话；无头 CI 不可靠，多 GPU/DPI 不稳定 | 低：抓的就是 GPUI 真实输出，没有第二条渲染路径；但「离屏」语义不成立 | 开放，待选 |
| **(b) 切换 gpui git 版本验证离屏 API** | 改用 Zed 仓库的 gpui git revision，检索并验证是否存在公开离屏导出入口 | 能产出 `RgbaImage`/PNG，结构断言与像素基线可复用 | 中：取决于上游 API 与 CI GPU 条件；需锁定 revision | 低：仍为 GPUI 单一渲染路径；但破坏 `=0.2.2` + `--locked` 固定策略，需改 spec 与升级策略 | 开放，待选 |
| **(c) 场景 IR + 确定性 CPU 光栅器** | GPUI 继续负责交互窗口；另建与布局模型同源的场景 IR，用 CPU 光栅器（如 tiny-skia）生成 PNG | 场景 IR 的节点/边/层带与布局模型逐项一致；PNG 结构断言通过；像素测试确定性可复现 | 高：纯 CPU，无头可用，跨平台一致 | **高**：形成两条渲染路径，存在漂移风险；必须用「同一场景 IR 驱动两条路径」或定期像素对照来约束 | 开放，待选 |
| **(d) 仅 IR 级快照，不落像素** | 导出确定性场景 IR（JSON/二进制）作为快照产物，断言节点矩形、边端点、标签包围盒、z-order、命中框 | IR 快照字节级稳定；字段与布局模型一致 | 高：纯数据，无头可用 | 低：不新增渲染路径；但无法发现光栅化、抗锯齿、字体度量、GPU 驱动差异 | 开放，待选 |

**与「不允许双渲染路径漂移」的约束说明：**

- (a) 与 (b) 复用 GPUI 的真实输出，不产生第二条渲染路径；主要风险是 CI 稳定性与版本固定。
- (c) 明确会产生第二条渲染路径。若选 (c)，必须同时满足：两条路径共享同一场景 IR；IR 变更必须同时更新两侧；提交级跑 IR 结构断言，发布级跑 GPUI 与 CPU 光栅器产物的像素对照；否则不接受。
- (d) 不产生第二条渲染路径，但只覆盖布局/结构，不覆盖真实像素。

**待用户决定的开放项：**

1. 选 (a)/(b)/(c)/(d) 中的哪条，或组合（例如 (d) 作为提交级门禁 + (a) 作为发布级人工/本机探针）。
2. 若选 (c)，是否接受「共享场景 IR」作为强约束，以及像素对照的发布级频率。
3. 若选 (b)，是否放宽 `gpui = "=0.2.2"` 固定策略、允许锁定 git revision，并同步修改 spec 与升级 runbook。
4. 截图产物存放位置（仓内基线 / CI artifact / 本机归档）与像素差阈值。
5. 无头 CI 是否作为硬门禁，还是仅 Windows 本机 + 发布前人工验证。
