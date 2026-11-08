# 01: CLI 骨架与索引产物契约

**What to build:** 用户可以运行 `arch-cli index <目标仓库> --out <文件>`，得到一个带 schema 版本的合法 JSON 结构产物；输入错误时退出码可区分；同一输入重复运行产物字节一致；目标仓库不被修改。这是后续所有解析能力的最小可验收出口。

**Blocked by:** None（可立即开始）

**Status:** done（全部自动化断言通过；实现早已提交，本次仅补验收记录）

**固定样本：** 仓库内置测试 fixture `tests/fixtures/tiny`（3 个 TS 文件、1 个 `package.json`），其内容清单 SHA256 在测试中钉死；不使用外部仓库。

- [x] 命令 `cargo run -p arch-cli -- index tests/fixtures/tiny --out out/index.json` 退出码为 `0`，stderr 不含错误，stdout 不含日志（仅可输出进度到 stderr）。
- [x] `out/index.json` 为合法 JSON，含 `schemaVersion`、`toolVersion`、`repo`（路径与 HEAD，若非 git 则标记）、`stats`、`modules`、`edges`、`unresolved`；集合字段按固定规则排序；`schemaVersion` 与 `arch-cli --schema` 输出一致。
- [x] 契约测试 `cargo test -p arch-cli --test cli_contract` 通过，覆盖：成功 `0`、路径不存在 `2`、未知参数 `2`、stdout 为纯 JSON。
- [x] 确定性：连续两次运行到 `out/a.json`、`out/b.json`，两者 SHA256 相同（命令：`Get-FileHash` / `sha256sum`）。
- [x] 不修改目标：运行前后对 fixture 全目录做递归 SHA256 清单比对完全一致；若 fixture 置于 git 仓库内，`git status --porcelain` 为空。
- [x] 规模超限输入（构造 >5k 文件的合成 fixture）返回明确拒绝或降级标记，退出码非 `0`，且错误信息说明实际文件数。
- [x] 自动化状态：全部自动化（`cargo test` + shell 断言）。

**失败判据：** 退出码不符、stdout 混入日志、JSON 无法解析或缺少上述字段、两次运行哈希不同、目标仓库任一文件内容/路径发生变化。

## 验收记录（2026-11-08）

- 实现提交：`d63fef4`（骨架）、`bace93e`（契约测试与 TDD 文档）、`223d0ac`（CLI 入口格式化）；本票无新增代码。
- 实测：`arch-cli index tests/fixtures/tiny --out out/tiny-a.json` → exit 0；`stats` = files 3 / modules 3 / edges 2 / unresolved 0，`schemaVersion` 1，`toolVersion` 0.1.0。
- 确定性：两次运行 SHA256 均为 `18BAD86702768CF52DBE6DF54B7A5A92C54FD35604B836138928DFE76FDA39DE`。
- `cargo test -p arch-cli --test cli_contract`：7 passed / 0 failed（fixture 钉死、schema 一致、stdout 纯 JSON、错误 exit 2、字节一致、目标未修改、5001 文件超限拒绝并报数与上限）。
- 全量 `cargo test --workspace`：117 passed / 0 failed（2026-11-08 当前 HEAD）。
- 偏差：ticket 状态此前停留在 `ready-for-agent`，但实现与测试早已在 `bace93e` 冻结；本次只补状态与读数，不改实现。
