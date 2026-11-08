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

## 平台支持（实测）

| 平台 | 构建 | 运行 | 状态 |
| --- | --- | --- | --- |
| Windows | CI 通过 | 窗口冒烟通过（CI） | 已验证 |
| macOS | CI 通过 | 未运行 | 仅编译验证 |
| Linux | CI 通过（需 xcb / xkbcommon 系统库） | 未运行 | 仅编译验证 |

macOS/Linux 目前只承诺可编译，不承诺可运行；依据见 ticket 11 与 CI run 42475740482。

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

### 时间线启动行为

`arch-view timeline <repo>` 采用「先开窗、后加载」：窗口先出现并显示加载态（当前阶段，例如「准备历史：共 617 个刻度」「构建分层」），数据就绪后自动切换到时间线并打印 `timeline: N ticks, M modules, K bands`。

- 窗口先出现、暂时只有加载文案是正常行为，不是卡死；计数行出现或时间线视图切换完成才算就绪。
- 若 `--auto-close-ms` 小于数据准备时间，进程会在计数行打印之前干净退出（退出码 0，stderr 无 panic），此时 stdout 可能没有 `timeline:` 行。
- 数据准备失败时，错误显示在窗口内，不只写 stderr。
