# 许可证：MIT OR Apache-2.0（双许可）

v0.1.0 之前，许可证一直留作「待定」：`Cargo.toml` 与三个 crate 清单里只有「勿填占位值」注释，仓库根没有 LICENSE 文件。2026-11-09，用户从 `MIT` / `Apache-2.0` / `MIT OR Apache-2.0` 三个选项中裁决为**双许可**。本 ADR 只记录裁决事实与落地方式；用户未给出选择理由，此处不代填、不推测。

**Status**: accepted (2026-11-09)

## Considered Options

- MIT：单许可，文本简短，无明确专利授权条款。
- Apache-2.0：单许可，含明确专利授权与贡献条款，文本较长。
- **MIT OR Apache-2.0（选中）**：使用者可任选其一，不是同时遵守两份。

## Consequences

- 仓库根放两份完整许可证文本 `LICENSE-MIT` 与 `LICENSE-APACHE`，不设单个 `LICENSE` 文件，避免把 `OR` 误读成 `AND` 或单一许可。文本分别取自 SPDX license-list-data 与 apache.org，非凭记忆手写；`LICENSE-MIT` 的版权行为中性写法 `Copyright (c) 2026 Strata contributors`（仓库提交来自 guoyu 与 cs1lion 两名贡献者），如需指定具体权利人应单独更正并说明。
- `[workspace.package]` 写 `license = "MIT OR Apache-2.0"`，三个 crate 用 `license.workspace = true` 继承，与 `version` / `edition` 的继承方式一致；原「license 待定」注释一并移除。
- 双许可对外发布后改为单许可需要全部贡献者同意，视为难逆转决定：后续变更必须新开 ADR，不能只改 `Cargo.toml` 字段。
- **已知缺口**：两份许可证文本在 workspace 根，不在各 crate 目录内；`cargo package --list` 显示 `.crate` 包当前不含许可证文本。本仓库按 ADR 0002 为纯桌面端、当前无 crates.io 发布流程，故本轮不处理；若将来要发布 crate，需补 `include` / `license-file` 或在各 crate 内放副本。
