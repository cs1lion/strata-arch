# Strata

面向「第一次读陌生 TS/JS 仓库」的本地优先架构解析与可视化工具。Rust workspace：
# Strata

> Local-first TypeScript/JavaScript architecture & codebase evolution explorer built in Rust and GPUI.

面向大型 TS/JS 代码库的本地优先架构解析与可视化工具。基于 Rust + GPUI 构建，提供高效静态分析与高性能原生桌面视图。

## 核心能力 (Core Capabilities)

- **AST 与符号级静态分析**：基于 AST 的精确符号提取、跨文件引用解析与推断。
- **模块层级与调用/依赖图构建**：自动推断模块多层级结构，呈现清晰的模块间依赖与方法调用关系图谱。
- **Git 提交演化时间线追踪**：结合历史 Commit，可视化演化路径与架构漂移情况。
- **引导式源码阅读路径生成 (Guided Reading Tour)**：智能计算主干阅读链路，自动推荐从核心入口到关键分支的源码阅读步骤。

## Workspace 架构 (Crate Architecture)

本项目采用 Rust Workspace 模块化设计：

- `arch-core`: 静态分析与演化图谱引擎，负责源码解析、依赖图构建与演化路径计算。
- `arch-cli`: Headless 结构化 JSON 数据契约 CLI，便于集成 CI/CD 工具链或第三方自动化流程。
- `arch-view`: 基于 GPUI 的高性能原生桌面交互视图，提供分层地图、路径播放与演化时间线展示。

工程纪律与 TDD 规范见 [ENGINEERING-DISCIPLINE.md](docs/process/ENGINEERING-DISCIPLINE.md)，合作规范见 [AGENTS.md](AGENTS.md)。

## 快速开始 (Quick Start)

### 构建与测试

```powershell
cargo build --locked --workspace
cargo test --workspace
```

### 运行示例

```powershell
# CLI 分析
cargo run -p arch-cli -- index tests/fixtures/tiny

# GUI 交互视图
cargo run -p arch-view -- smoke tests/fixtures/tiny
cargo test -p arch-view --test scene_snapshot
```
- `arch-core`：索引、模块/符号解析、层推断、调用图、git 历史与引导式路径。
- `arch-cli`：headless CLI，唯一的 JSON 契约出口。
- `arch-view`：GPUI 桌面视图（分层地图、路径播放、演化时间线）。

工程纪律、TDD 档位与验收口径见 [docs/process/ENGINEERING-DISCIPLINE.md](docs/process/ENGINEERING-DISCIPLINE.md)；合作规范见 [AGENTS.md](AGENTS.md)。

## 平台矩阵（如实声明）

| 平台 | 构建 | 启动冒烟 | IR 快照 | 真实窗口截图 | 结论 |
| --- | --- | --- | --- | --- | --- |
| Windows | 已验证 | 已验证 | 已验证 | 已验证（一次人工证据） | **唯一已验证平台** |
| macOS | 仅编译配置，未实际编译 | 未验证 | 未验证 | **不做** | 仅编译未验证 |
| Linux | 仅编译配置，未实际编译 | 未验证 | 未验证 | **不做** | 仅编译未验证 |

**本项目不声称跨平台可用。** Windows 已验证；macOS/Linux 仅编译未验证——仓库里有对应 CI 配置，但没有任何远端运行记录，也没有在真实 runner 上编译过。

## 已验证命令（Windows）

```powershell
cargo build --locked --workspace
cargo test --workspace
cargo run -p arch-cli -- index tests/fixtures/tiny
cargo run -p arch-view -- smoke tests/fixtures/tiny
cargo test -p arch-view --test scene_snapshot
pwsh -File eval/perf-baseline.ps1        # 含 512MB 峰值内存门禁
pwsh -File eval/capture-window.ps1       # 手动 OS 窗口捕获，不进 CI
```

## 未验证项（如实列出）

### ticket 09（演化时间线与热力）

- 截图：未实现，已移交 ticket 11；当前只有「窗口能开且退出码 0」的 smoke，没有像素证据。
- excalidraw 时间线：未测。语料缓存是无 `.git` 的归档副本，无法生成历史时间线。
- UI 交互自动化：刻度点击与节点选择只做过本机 smoke，没有 `gpui::test` 事件断言。
- 成本：每个刻度重跑一次前缀采样（fixture 6 刻度约 4.4s），大仓不可用；增量/缓存未实现。

### ticket 11（跨平台构建与 GPUI 启动冒烟）

- macOS/Linux：未编译、未启动、未截图；gpui 在 Linux 的系统图形依赖能否在 CI 构建未知。
- CI：`.github/workflows/ci.yml` 仅本地落盘，从未在远端运行。
- 截图能力：gpui 0.2.2 无公开离屏导出入口。已裁决为「IR 快照进提交级门禁 + Windows OS 捕获人工证据」；IR 快照只覆盖布局模型（矩形、边端点、层带、z-order、命中框），不覆盖光栅化、抗锯齿、字体度量与 GPU 驱动差异；无像素差基线。
- OS 窗口捕获是手动脚本，依赖可见桌面会话，不作为 CI 门禁。
