# 评估体系（Eval Harness）

本文定义 Strata 解析质量的固定评估方式：固定脚本 + 具体案例 + 机器可读读数。
目标是让“解析得对不对”可复现、可追溯、可门禁，而不是靠人工抽查或事后描述。

> 状态（2026-11-07）：ticket 02 已接入最小可用版本（不变量 + 确定性 + 两个固定样本仓的 golden 读数）。
> 标注集、合成极端用例、独立 eval 脚本、pin 清单与归档流程目前是**设计**，尚未实现，见第 9 节。

## 1. 原则

1. **分母可见**：每个指标必须给出命中数 / 总数，并逐条列出失败项。禁止只给百分比。
2. **失败逐条可读**：缺失边、多余边、错归属、未解析项、不变量违规都必须带源文件、说明符、目标与原因。
3. **只提交已验证读数**：文档与基线中的数字必须来自实际执行输出；未跑通的项标注为未验证，不写预期值。
4. **期望值独立变更**：修改 `eval/` 下的期望值、golden 或标注必须单独提交并说明理由，禁止与实现改动混在同一提交。
5. **读数差不等于可以放宽**：指标不好看时先修实现或显式记录已知限制，不得直接下调阈值。

## 2. 案例分层

| 层级 | 位置 | 规模 | 用途 | 是否标注 |
| --- | --- | --- | --- | --- |
| L1 标注集 `labeled` | 仓内 `eval/cases/labeled/` | 小，边数几十到几百 | 算精确率 / 召回率，逐条比对 | 是，人工 ground truth |
| L2 真实仓库快照 `pinned` | 仓外缓存，仓内只存 pin 清单 | 中到大型真实仓库 | 只做回归，不做“正确性”判定 | 否，锁定读数 |
| L3 合成极端构造 `synthetic` | 仓内 `eval/cases/synthetic/` | 极小、每个用例只测一个语义 | 约束边界行为，失败即红 | 是，边集合即期望 |

### L1 标注集

- 人工标注每个源文件的期望导入边：`(source, specifier, target | unresolved(reason))`。
- 期望文件为 `eval/cases/labeled/<case>/expected.json`，同时记录源文件 SHA256。
- 指标必须同时给出：
  - 召回：命中的期望边 / 期望边总数，逐条列出漏掉的边；
  - 精确：命中的期望边 / 产物边总数，逐条列出多余或错归属的边。
- 标注集按 ticket 逐步扩充：02 模块边，03/04 符号与调用边，06 图层，08 演化指标。

### L2 真实仓库快照

- 样本仓库本体**不入库**。仓内只存 pin 清单：仓库 URL、commit、许可证、关键文件 SHA256、期望读数。
- 运行时由脚本拉取到缓存目录，默认 `~/.cache/strata/corpus/<name>-<commit>/`。
- 真实仓库只做回归：读数漂移必须解释，不做人工逐边标注。
- 当前仓内测试直接读取 `tests/corpus/`，属于 ticket 02 的过渡实现；后续迁移到 pin 清单 + 缓存目录。

### L3 合成极端构造

每个目录一个场景，至少覆盖：

- 循环依赖：`a -> b -> a`，验证不丢边、不递归爆栈。
- Barrel / re-export：`index.ts` 只做 `export * from`，验证边归属到 barrel 而非最终使用方。
- `export *` 与 `export { x } from`。
- `tsconfig` / `jsconfig` 路径别名：精确匹配、通配符、最长前缀、别名未命中。
- 动态 `import()`：字面量与表达式；表达式必须进 `unresolved`，不得猜测。
- `require()`：字面量与表达式。
- 大小写冲突：仅大小写不同的文件名（在大小写不敏感文件系统上标注为平台限制，不静默合并）。
- `node_modules` 深包：`a/node_modules/b/node_modules/c` 只出现 `external:b` 与 `external:c`，不展开文件。
- workspace 包：包名入口、子路径、`package.json` 入口字段。
- 非源码扩展：`.json` / `.css` / `.woff2` 等落入 `unsupported-extension`。
- 注释、字符串、模板字面量、正则中的伪导入不得产生边。

## 3. 存储与归属

### 仓内（随代码演化、进 CI、可追溯）

- `eval/run*`：评估脚本入口。
- `eval/cases/labeled/`：标注用例与 `expected.json`。
- `eval/cases/synthetic/`：合成用例与其期望边集合。
- `eval/golden/`：固定样本仓的期望读数、pin 清单、SHA256 清单。
- `eval/baseline/current.json`：当前基线摘要（只放摘要，不放完整产物）。
- `eval/README.md`：运行方式与读数解读。

选择仓内的理由：脚本需要随解析器 API 演化；必须能进 CI；读数变化要能对应到具体提交。

### 仓外（不随主仓增长）

- 样本仓库本体：`~/.cache/strata/corpus/<name>-<commit>/`。
- 完整读数、截图、基线产物、临时产物：`~/.cache/strata/eval/archive/<git-sha>/<timestamp>/`。
- 可通过环境变量 `STRATA_EVAL_ARCHIVE` 覆盖归档根目录；路径约定固定，便于审计。
- `.gitignore` 至少包含 `/eval/archive/`、`/eval/tmp/`、`/tests/corpus/`，防止误入库。
- 主仓只保留 `eval/baseline/current.json` 这一份“当前基线摘要”，其余生成物不得回仓。

### 防作弊约束

1. 标注文件、`expected.json`、golden 与 pin 清单全部钉 SHA256，清单本身入库并由 CI 校验。
2. 实现者不得单独修改标注或期望值让测试变绿；标注由产品负责人或指定评审人确认。
3. 修改 `eval/` 下任何期望值必须使用独立提交，提交信息以 `eval:` 开头并写明理由、旧读数、新读数。
4. 实现改动与期望值改动不得出现在同一提交；CI 拒绝同时修改 `crates/` 与 `eval/golden|labeled` 的提交，除非显式豁免标记。
5. CI 对比上一基线；未登记的读数漂移直接失败。

## 4. 评估脚本与机器可读输出

计划入口：`eval/run`（提交级快集）与 `eval/run --full`（发布级全集）。脚本只编排，实际索引调用 `arch-cli` 或 `arch-core`。

输出必须是一份 JSON，禁止只用百分比。示例结构：

```json
{
  "mode": "commit",
  "generatedAt": "2026-11-07T12:00:00Z",
  "gitSha": "…",
  "cases": [
    {
      "name": "synthetic/barrel-export-star",
      "status": "failed",
      "failures": [
        {"kind": "missing-edge", "source": "index.ts", "specifier": "./a", "expectedTarget": "a.ts"},
        {"kind": "extra-edge", "source": "index.ts", "target": "wrong.ts"}
      ]
    }
  ],
  "metrics": {
    "resolution": {
      "recall": {"matched": 19, "total": 20, "missedEdges": ["…"]},
      "precision": {"matched": 19, "total": 22, "extraEdges": ["…"]}
    },
    "unresolved": {
      "total": 7,
      "byReason": {"relative-missing": 0, "alias-missing": 0, "dynamic-import": 5, "unsupported-extension": 2},
      "items": ["…"]
    },
    "invariants": {"violations": 0, "items": []},
    "determinism": {"byteDiff": 0, "firstSha256": "…", "secondSha256": "…"},
    "resources": {"wallMs": 812, "peakRssMb": 143}
  },
  "failedItems": []
}
```

关键要求：

- 每个 `matched/total` 的分母必须出现；
- `missedEdges`、`extraEdges`、`unresolved.items`、`invariants.items` 必须逐条列出，不能折叠成计数；
- 失败用例退出码非 0，但 JSON 仍完整写出；
- 生成物写入归档目录，仓内只更新 `eval/baseline/current.json`（如策略允许）。

## 5. 指标（固定 5 个）

| # | 指标 | 定义 | 分母 / 读数 | 适用层级 |
| --- | --- | --- | --- | --- |
| 1 | 解析召回 `resolution_recall` | 命中期望边 / 期望边总数 | 逐条列出漏边 | L1 标注集 |
| 2 | 解析精确 `resolution_precision` | 命中期望边 / 产物边总数 | 逐条列出多余或错归属边 | L1 标注集 |
| 3 | 未解析边数 `unresolved_edges` | `unresolved` 总数 + 按原因分布 | 逐条列出未解析项 | L1/L2/L3 |
| 4 | 不变量违规数 `invariant_violations` | 违反不变量检查的条数 | 逐条列出违规 | 全部 |
| 5 | 确定性与资源 `determinism_resources` | 两次运行字节差异、墙钟耗时、峰值内存 | diff 字节数、ms、MB | L2/L3 + 发布级 |

不变量检查（无需标注）：

- `modules` 按 ID 有序且 ID 唯一；
- `edges` 按 `(source, target, kind)` 有序，且两端节点都存在；
- `unresolved` 按 `(file, specifier, reason)` 有序，`file` 存在于模块集合；
- 外部依赖只出现包级节点，`node_modules` 不展开；
- 两次运行序列化字节完全一致；
- 所有无法解析的导入显式出现在 `unresolved`，不静默丢弃。

## 6. 门禁分层

| 层级 | 触发 | 内容 | 预算 |
| --- | --- | --- | --- |
| 提交级快集 `commit` | 每次提交 / PR | 合成用例 + 小型标注集 + fixture 不变量 + 确定性 | ≤ 30 秒 |
| 发布级全集 `release` | 手动或发布标签 | 全部合成 + 全部标注 + 两个固定真实仓库 + 资源读数 | 不设硬上限，记录读数 |

- 阈值与基线存放在 `eval/baseline/current.json`，修改必须独立提交。
- 任何“放宽阈值”的提交都必须写明旧读数、新读数、原因与风险；禁止因为读数不好看而直接放宽。
- ticket 02 阶段只实现不变量与确定性；完整门禁在后续 ticket 逐步接入。

## 7. 当前已实现（ticket 02 最小版本）

位置：`crates/arch-core/tests/resolution_golden.rs`，以及 `crates/arch-cli/tests/resolution.rs`。

已覆盖：

- 不变量：模块 ID 唯一、模块有序、边有序、边两端存在、`unresolved` 有序且文件存在、外部包不展开、路径使用正斜杠、`stats` 与集合长度一致。
- 确定性：固定样本仓连续两次 `build_index`，序列化结果字节一致。
- 固定读数：`tsx` 与 `excalidraw` 的 `files / modules / edges / unresolved` 及边类型、原因类别计数。
- 另有一个小型 fixture 的完整解析断言（相对导入、别名、workspace、外部包、动态导入、`require`、未解析原因）。

尚未实现：

- 独立 `eval` 脚本与 JSON 输出；
- L1 标注集；
- L3 合成极端用例目录；
- pin 清单 / SHA256 清单 / 归档目录；
- 提交级与发布级门禁编排。

## 8. 当前基线摘要（已实际运行）

执行日期：2026-11-07。命令：

```text
cargo test --workspace
cargo run -p arch-cli -- index tests/corpus/tsx --out …/tsx.json
cargo run -p arch-cli -- index tests/corpus/excalidraw --out …/excalidraw.json
```

| 样本 | files | modules（source + external） | edges（import / dynamic / require） | unresolved（原因分布） |
| --- | --- | --- | --- | --- |
| `privatenumber/tsx` @ `4473971` | 95 | 136（95 + 41） | 379（376 / 2 / 1） | 10（dynamic 5，unsupported 5，relative-missing 0） |
| `excalidraw/excalidraw` @ `701b901` | 718 | 802（718 + 84） | 4363（4292 / 26 / 45） | 334（unsupported 323，dynamic 8，alias-missing 3，relative-missing 0） |

- `excalidraw` 的 3 条 `alias-missing` 均位于 `examples/` 与 `dev-docs/`，引用构建产物 CSS；根 `tsconfig` 的 include 范围是 `packages` + `excalidraw-app`，因此不在本票判定范围内。
- 两次运行 SHA256：`tsx` = `D4029E828F2AD6C30D20678B6A2634A04ADC1EC1F5A9C6923939ACAA221D6FD4`，`excalidraw` = `C64F00E8DC77DDD640078A71833BA6CA4FA43819FC5083608E746D206062F239`，同一样本两次一致。

## 9. 后续落地顺序

1. 抽出独立 `arch-eval` 入口与 JSON 输出，接入 `assert_invariants` 的等价检查。
2. 建 `eval/cases/synthetic/`，先覆盖循环、barrel、`export *`、别名、动态导入、`node_modules` 深包。
3. 建首批 L1 标注集（以 `tests/fixtures/resolution` 为起点扩充）。
4. 把 `tests/corpus/` 迁移为 pin 清单 + `~/.cache/strata` 缓存拉取。
5. 接 CI 快集（≤30 秒），发布级全集手动触发并归档读数。
6. 每次结果写入固定归档路径；主仓只更新基线摘要。

## 10. Ticket 02 验收接入

ticket 02 的验收以以下命令为准：

```text
cargo test -p arch-core --test resolution_golden
```

该测试同时覆盖不变量与确定性，并锁定第 8 节读数。完整评估体系（标注集、合成用例、脚本输出、门禁）按第 9 节在后续 ticket 落地。
