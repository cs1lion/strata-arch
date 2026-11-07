# 采用 Rust core + GPUI 作为实现栈

项目需要在本地解析 TS/JS 仓库并把依赖、调用关系呈现为可交互图形，候选方案是 Web UI 栈、Python 后端 + TS UI、Rust core + Web UI、Rust core + GPUI。用户裁决采用 **Rust core（解析/索引/图算法）+ GPUI（原生桌面渲染）**：核心逻辑与 UI 共享 Rust 类型，文件不出本机；GPUI 提供 GPU 加速的 canvas、路径绘制、hitbox 与动画原语，足以自绘交互图。代价是接受 GPUI 的 pre-1.0 破坏性变更、没有 Web/HTML 目标、图布局与拾取需自行实现。

**Status**: accepted

## Considered Options

- Node/TS 全栈：上手最快，但大仓库索引与并发弱。
- Python 后端 + TS UI：解析生态好，分发与性能一般。
- Rust core + Web UI：原 Q2 推荐，但被 Q8 的 GPUI 选择取代；两者不能同时成立。
- **Rust core + GPUI**：用户选择。

## Consequences

- GPUI crate 当前为 0.2.2（Apache-2.0，pre-1.0，README 明示版本间常有破坏性变更），升级需要回归测试。
- GPUI 只提供桌面窗口后端（macOS Metal、Linux/FreeBSD Wayland/X11、Windows Win32/DirectWrite），没有 Web 目标；若保留 HTML 导出，必须另建一条渲染路径。
- 图布局、平移缩放、命中测试、标签避让都要在 GPUI 之上自行实现，没有现成图库可复用。
- CLI 需要作为独立二进制存在，避免无 GUI 环境被 GPUI 的桌面依赖拖住；核心逻辑放在共享 crate。

交付形态已由 ADR-0002 裁决：纯桌面端，取消 Web UI 与可交互 HTML 导出；本 ADR 的栈选择不变。
