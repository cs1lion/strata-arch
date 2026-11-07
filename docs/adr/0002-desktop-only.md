# 交付形态：纯桌面端，取消 Web UI

Round 1 的 Q2 选了「本地 CLI + 本地 Web UI」，Q8 又选了 GPUI 原生桌面框架，两者不能同时成立。用户确认：交付形态为 **Rust core + CLI + GPUI 桌面端**，取消本地 Web UI，不再保留 Web 界面这条线，也不做可交互 HTML 导出。CLI 仍可输出机器可读的 JSON 供其它工具消费，但那不是浏览器界面。

**Status**: accepted

## Considered Options

- 桌面端 + 可交互 HTML 导出：保留分享能力，但需要第二条渲染路径和 JS 依赖。
- 放弃 GPUI、保留 Web UI：与 Q8 裁决冲突。
- GPUI 内嵌 WebView 渲染图：技术上是混合方案，但引入平台不一致与额外运行时。
- **纯桌面端（选中）**：单一渲染路径，代码不出本机，交互与 GPU 绘制都在 GPUI 内完成。

## Consequences

- 只有一条 UI 渲染路径，所有图形、动画、交互都在 GPUI 内实现；没有浏览器端可访问性、分享链接或远程查看。
- 分享只能通过截图/录屏或 CLI 的 JSON 产物，不能给非本机用户一个可交互页面。
- 以后若要做 Web 界面，必须新开 ADR，并重新评估图渲染、交互与数据契约，不能假设现有 GPUI 代码可复用。
- 测试策略需要围绕 GPUI 的 `render_to_image` 与本机基线建立，不能依赖浏览器截图工具。
