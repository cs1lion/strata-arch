# Strata

本地优先的 TypeScript/JavaScript 架构解析与可视化工具：静态分析得到模块分层、依赖与调用图，再把它们放进原生桌面窗口里看。Rust workspace，`arch-core` 负责解析与图模型，`arch-cli` 输出 JSON，`arch-view` 提供 GPUI 桌面视图。

## 前置条件

- Rust stable（workspace 使用 edition 2021，CI 用 `dtolnay/rust-toolchain@stable`）。
- 目标仓库需含 TS/JS 源文件；`timeline` 与 `diff` 还要求它是**非浅克隆**的 git 仓库。
- Windows：安装 Rust MSVC toolchain 即可，无额外系统库。
- Linux：需要 xcb / xkbcommon 与 Vulkan/GL 运行库，参考 CI 安装命令：
  ```bash
  sudo apt-get install -y libxcb1-dev libxkbcommon-dev libxkbcommon-x11-dev \
    mesa-vulkan-drivers libgl1-mesa-dri vulkan-tools
  ```
- macOS：可编译；运行尚未验证（见下方「平台支持」）。

## 三分钟跑起来

```bash
# 1) 构建（release，首次编译 GPUI 较慢，只需一次）
cargo build --release --locked

# 2) 最小仓库的架构地图（分层 + 依赖边）
cargo run --release -p arch-view -- map tests/fixtures/tiny

# 3) 提交演化时间线（在本仓库内运行，读取 first-parent 提交历史）
cargo run --release -p arch-view -- timeline .

# 4) 两个提交的结构 diff（本仓库内的一对固定 commit，示例输出 added=25）
cargo run --release -p arch-view -- diff . --from 2b66deb --to 29f8e27

# 5) 引导式阅读路径（入口 → 目标符号，逐步播放）
cargo run --release -p arch-view -- map tests/fixtures/path \
  --from src/cli.ts#module-toplevel --to src/utils/helper.ts#function:helper
```

四个视图各自看什么：

| 模式 | 命令入口 | 看的是什么 |
| --- | --- | --- |
| `map` | `arch-view map <repo>` | 模块分层地图与模块间依赖边；先开窗、显示加载态，数据就绪后展示内容 |
| `timeline` | `arch-view timeline <repo>` | 按提交采样的演化时间线（模块数、边、分层带随时间的漂移） |
| `diff` | `arch-view diff <repo> --from <commit> --to <commit>` | 两个 commit 之间的结构差异：added / removed / modified 与疑似重命名提示 |
| `path` | `arch-view map <repo> --from <入口ID> --to <符号ID>` | 从入口到目标的一条最短调用路径，可在底部面板逐步播放 |

需要 JSON 而不是窗口时，用 `arch-cli`：

```bash
cargo run --release -p arch-cli -- index tests/fixtures/tiny --out out/index.json
cargo run --release -p arch-cli -- layers tests/fixtures/tiny --out out/layers.json
cargo run --release -p arch-cli -- path tests/fixtures/path --entries
```

运行期退出：窗口关闭即退出；自动化场景用 `--auto-close-ms <毫秒>` 定时干净退出（退出码 0）。

## 已知限制

- **diff 在大仓上约 10 秒**：excalidraw（802 模块）热 diff 实测 median 约 9.8s、p95 约 10.2s。窗口先显示加载态与当前阶段，可随时关闭窗口或点「取消」立即退出；读数与预算见 [ticket 10](docs/tickets/arch-tool/10-performance-memory-baseline.md)。
- **Linux 只在软件渲染下验证过**：CI 用 `xvfb` + `openbox` + Mesa lavapipe 跑窗口冒烟，只证明能开窗、出帧、干净退出，不代表真实 GPU 表现，也不代表画面正确；视觉判据见 [acceptance-gaps 2.2.1/2.2.2](docs/acceptance-gaps.md)。
- **macOS 仅编译验证**：CI 构建通过，未在真机运行；依据见 ticket 11 与 CI run 42475740482。
- **首屏揭示动画默认开启**：`--no-reveal` 关闭（与 `--reveal` 同时出现时以 `--no-reveal` 为准）。固定 **900ms** 时长驱动（进度取自 GPUI 动画时钟，帧率只决定中间帧疏密），各层带按层序均分时长。实测 `reveal:start → reveal:done`：本机 0.93s、lavapipe 多核 0.99s、lavapipe 单核 1.16s（最慢与最快相差 24%）。任意交互可立即跳到最终态。需要精确的静态截图或中途帧时用 `--no-reveal`。
- **timeline / diff 需要真实历史**：无 `.git` 的归档副本（如 `tests/corpus/excalidraw`）不能用于 `timeline`；浅克隆会失败或找不到 `--from/--to` 指定的提交。
- **符号与路径的已知缺口**：顶层 JSX 位于 `root.render(<...>)` 参数内时不产生 `module-toplevel` 的 jsx 边，这类入口无法给出路径（见 [ticket 06](docs/tickets/arch-tool/06-entry-target-guided-path-query.md)）。

## 平台支持（实测）

| 平台 | 构建 | 运行 | 状态 |
| --- | --- | --- | --- |
| Windows | CI 通过 | 窗口冒烟通过（CI） | 已验证 |
| macOS | CI 通过 | 未运行 | 仅编译验证 |
| Linux | CI 通过（需 xcb / xkbcommon 系统库） | 窗口冒烟通过（CI：xvfb + openbox + 软件渲染） | 运行验证（软件渲染） |

### 时间线启动行为

`arch-view timeline <repo>` 采用「先开窗、后加载」：窗口先出现并显示加载态（当前阶段，例如「准备历史：共 617 个刻度」「构建分层」），数据就绪后自动切换到时间线并打印 `timeline: N ticks, M modules, K bands`。

- 窗口先出现、暂时只有加载文案是正常行为，不是卡死；计数行出现或时间线视图切换完成才算就绪。
- 若 `--auto-close-ms` 小于数据准备时间，进程会在计数行打印之前干净退出（退出码 0，stderr 无 panic），此时 stdout 可能没有 `timeline:` 行。
- 数据准备失败时，错误显示在窗口内，不只写 stderr。

## Workspace 结构

- `arch-core`：源码解析、模块解析、分层推断、调用图、历史采样与 commit 结构 diff。
- `arch-cli`：headless JSON 产物（`index` / `layers` / `callgraph` / `history` / `path` / `drift`）。
- `arch-view`：GPUI 桌面视图（map / timeline / diff / path 播放 / scene / smoke）。

工程纪律与 TDD 规范见 [ENGINEERING-DISCIPLINE.md](docs/process/ENGINEERING-DISCIPLINE.md)，协作约定见 [AGENTS.md](AGENTS.md)。

## 验证与回归

```bash
cargo test --workspace
cargo fmt --check
pwsh eval/drift-gate.ps1 -PlanOnly   # 改动是否触发漂移门禁
pwsh eval/release-check.ps1          # 本地发布前检查：五条路径 + 测试 + fmt + drift 计划
```

`eval/release-check.ps1` 只用于本地发布前核对，不进入 CI（CI 已覆盖构建、测试与漂移门禁）。四条用户可见路径的耗时口径见 [eval/path-timing.ps1](eval/path-timing.ps1)。
