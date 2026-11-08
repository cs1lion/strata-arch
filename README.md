# Strata

面向「第一次读陌生 TS/JS 仓库」的本地优先架构解析与可视化工具。Rust workspace：

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
