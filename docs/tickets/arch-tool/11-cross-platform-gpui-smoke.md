# 11: 跨平台构建与 GPUI 启动冒烟

**What to build:** 三个平台都能以固定 GPUI 版本、`--locked` 构建；Windows 上应用能启动窗口、渲染一帧并离屏截图非空；macOS/Linux 至少编译通过。把 GPUI 的 pre-1.0 破坏性变更挡在合并前。

**Blocked by:** 03（分层地图，提供可渲染的最静态视图）

**Status:** ready-for-agent

**固定样本：** `tests/fixtures/tiny`；截图命令使用 `--smoke` 模式。

- [ ] 依赖固定：`gpui = "=0.2.2"`（推荐值，**待用户确认升级节奏**）；`Cargo.lock` 已提交；CI 所有构建使用 `cargo build --locked --workspace`，锁文件不一致即失败。
- [ ] Windows：`cargo run --locked -p arch-view -- smoke tests/fixtures/tiny --screenshot out/smoke.png` 在 3 秒内退出码 `0`；进程无 panic；输出图像宽高 >0 且不同颜色数 >10。
- [ ] macOS 与 Linux：`cargo build --locked --workspace` 通过，启动冒烟为推荐项（**平台矩阵待用户确认**：推荐 Windows 主验收，mac/Linux 编译 + 启动冒烟）。
- [ ] 依赖更新受控：CI 配置中不存在自动依赖更新任务（dependabot/renovate）；`Cargo.lock` 仅在单独 PR 中变更（断言变更 PR 只含依赖相关文件）。
- [ ] 失败判据：`--locked` 构建失败、Windows 启动崩溃或超 3s、截图空/纯色、锁文件被隐式改写。
- [ ] 自动化状态：构建与 Windows 冒烟自动化；mac/Linux 运行依赖可用 runner（**待用户确认**）；GPUI 升级后的视觉回归无基线，仅以编译 + 冒烟兜底（见 `docs/acceptance-gaps.md`）。
