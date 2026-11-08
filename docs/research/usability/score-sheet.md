# 记分与归因表（票 12）

**版本：** `UK-2026-11-08-01`
**配套：** `README.md`、`facilitator-script.md`、`answer-key.md`
**用途：** 每题记录通过/失败、耗时、置信度、归因；场次结束后汇总成可复核报告。

---

## 1. 通过判定（二值）

| 任务 | 通过条件 | 超时 |
| --- | --- | --- |
| T1 | 符号、模块、层序列与 key 完全一致，顺序正确 | 否 |
| T2 | 3 个模块集合与 key 完全一致，无多报/错报 | 否 |
| T3 | 2 个模块集合与 key 完全一致，无多报/错报 | 否 |
| T4 | 新增/删除/修改三类的路径集合与类别全部正确，无多报/错报 | 否 |
| T5 | 边=静态调用关系，步进≠运行时执行顺序，两问均正确 | 否 |

规则：

1. **通过必须是“正确且未超时”**。超时后即使答对，记 `success=false`、`timeout=true`；部分分照算，用于诊断。
2. **集合题（T2/T3/T4）** 多报一个 = 失败（precision 失败）；少报一个 = 失败（recall 失败）。不要因为“接近”改成通过。
3. **T1 顺序题** 顺序错误 = 失败；只报模块不报符号 = 失败（任务要求两者）。
4. **路径简写**：如果简写在同一类别/仓库中唯一可识别，接受；若产生歧义（例如两个同名 `index.ts`），要求补全。评分时在 `normalized_answer` 写清实际接受的字符串。
5. **空答案/“不知道”** 记失败；归因不能写“用户不会用”，要落到 `V/I/F/S/E`，或 `U`（仅脚本歧义）。

每题通过人数记 `k/5`；阈值 `≥4/5`。**不写百分比**；如果报告里必须写比率，同时写 `k/5` 和“5 人小样本，不做统计推断”。

---

## 2. 部分分（仅诊断，不改变通过）

| 任务 | 公式 | 说明 |
| --- | --- | --- |
| T1 | `0.7 * (按位置匹配步数/总步数) + 0.3 * layer_correct` | 位置匹配要求符号+模块都对；总步数见 key |
| T2 | `F1 = 2*|A∩K| / (|A|+|K|)` | `A` 为参与者集合，`K` 为 key 集合；≤3 个 |
| T3 | 同 T2 | `K` 为 2 个 |
| T4 | `mean(F1_added, F1_removed, F1_modified)` | 每类单独算；类别错放也扣 |
| T5 | 0/1/2 rubric | 2=通过，1=部分，0=失败 |

部分分用于：

- 区分“完全不会”和“只差一个”；
- 发现同名模块/近名模块的混淆；
- 生成缺陷清单时排优先级。

**不得**用部分分把 `success=false` 改成通过。

---

## 3. 耗时与中位数

记录字段：

- `started_at` / `ended_at`：主控念完任务文字到参与者说“答完”。
- `elapsed_s`：秒，保留 1 位小数。
- `timeout`：是否达到单题上限。
- `session_timeout`：是否触发 30 分钟硬上限。

超时处理：

```
if elapsed >= cap:
    timeout = true
    elapsed_recorded = cap          # 截尾，不把“继续等”的时间算进去
else:
    timeout = false
    elapsed_recorded = elapsed
```

中位数报告两个值：

| 指标 | 定义 | 用途 |
| --- | --- | --- |
| `median_all_s` | 5 人全部 `elapsed_recorded` 的中位数（超时按 cap 计） | 反映“在预算内能不能做完” |
| `median_success_s` | 仅 `success=true` 的 `elapsed_s` 中位数 | 反映“做成的人花了多久” |
| `timeout_count` | 超时人数 | 单独报，不要藏进中位数 |

若某题成功人数 <3，`median_success_s` 写“样本不足（n=<k>）”，不强行算中位数。

**总时长：** 单场 30 分钟硬上限。超过则停止后续任务，未做任务记 `not-run`；已做任务照常评分；在报告中写“超时且已记录”。

---

## 4. 置信度校准

每题答完后问：`你对这个答案有多确定？1=纯猜，5=非常确定。`

记录 `confidence_1_5`。汇总：

### 4.1 校准表

| 置信度 | 答对人数 | 答错人数 | 该档准确率 | 说明 |
| --- | --- | --- | --- | --- |
| 1 |  |  |  |  |
| 2 |  |  |  |  |
| 3 |  |  |  |  |
| 4 |  |  |  |  |
| 5 |  |  |  |  |

### 4.2 关键指标

```
high_conf_wrong_rate = count(confidence >= 4 and success == false) / count(confidence >= 4)
brier = mean((confidence/5 - success)^2)        # 可选；5×5=25 点，仅描述
```

- `high_conf_wrong_rate` 是重点：变化盲视/过度自信会让参与者“很确定但答错”。
- 如果 `count(confidence >= 4) == 0`，写“无高置信样本”，不要算 0/0。
- 置信度按任务也列一张表（T1–T5），看哪一题最容易“自信地错”。

### 4.3 解释边界

- 25 个点（5 人 × 5 题）只能做描述，不能做显著性/回归。
- T5 如果使用了 A/B/C/D 选项，标 `prompted=true`，与自由回答分开；prompted 的置信度更容易虚高，报告要注明。
- 不要把置信度当成“用户满意度”；它只反映自评确定程度。

---

## 5. 失败归因口径

每个失败必须有一个主归因；可选一个次归因。主归因只能是 `V/I/F/S/E`；`U` 仅用于复核确认的脚本歧义。

### 5.1 主类

| 代码 | 名称 | 定义 | 典型证据 |
| --- | --- | --- | --- |
| `V` | 视图问题 | 元素存在，但看不清/看不懂/被视觉编码误导 | 标签重叠、颜色/尺寸饱和、原始 epoch、高亮对比度低、行截断、相机丢失 |
| `I` | 信息缺失 | 数据/产物存在，界面没给出或没汇总 | 无全局排序/筛选、无格式化日期、无模块清单、无 diff 路径/原因、无“最近修改”视图 |
| `F` | 动线问题 | 找不到入口/控件/视图，或跳转太多/回退迷路 | 首动作乱点、找不到播放/前进、反复切视图、忘记刻度、需要记住上一屏 |
| `S` | 语义误读 | 图形/交互语义理解错误 | 边=数据流、步进=运行时顺序、冷色=很久没动 |
| `E` | 环境 | 崩溃/卡死/加载失败/窗口不在屏内 | 录屏、日志、退出码；单独统计，不混入可用性缺陷 |
| `U` | 任务歧义 | 脚本措辞本身歧义 | 需第二人复核；不计为参与者失败；修正脚本后另起一轮 |

### 5.2 视图标签

| 标签 | 范围 |
| --- | --- |
| `map` | 模块地图、层带、标签、命中框、缩放/平移 |
| `drill` | 模块下钻、符号视图、返回 |
| `path` | 路径播放、当前边高亮、前进/后退/播放、fly-to |
| `timeline` | 时间线刻度、热力、图例、底部模块信息 |
| `diff` | 双 commit diff、三类行、标题栏、路径/原因 |
| `shell` | 窗口启动、关闭、焦点、录屏/终端等应用外壳 |

### 5.3 常用代码组合（建议，不强制）

| 代码 | 含义 | 例子 |
| --- | --- | --- |
| `V-MAP` | 地图看不清 | 标签重叠、层带归属看不清 |
| `V-PATH` | 路径呈现问题 | 当前边对比度低、看不到中步、状态栏误导 |
| `V-TL` | 时间线呈现问题 | 热力饱和、epoch 不可读、刻度太密 |
| `V-DIFF` | diff 呈现问题 | 行截断、同名模块不可区分、总数误导 |
| `I-RANK` | 无排序/筛选 | T2 无法比较所有模块 |
| `I-RECENCY` | 无最近修改视图/日期格式化 | T3 只能逐点看 epoch |
| `I-DIFF` | 缺 diff 口径说明/原因 | T4 不知道“内容变化≠结构变化” |
| `F-ENTRY` | 找不到入口/视图 | 不知道时间线怎么回看、diff 从哪开 |
| `F-CONTROL` | 找不到控件 | 不会前进/播放/缩放 |
| `F-FLOW` | 动线太长/回退迷路 | T2 逐个点 136 个节点，忘记已看过哪些 |
| `S-DATAFLOW` | 边读成数据流 | T5 |
| `S-EXEC` | 步进读成运行时顺序 | T5 |
| `S-SCOPE` | 模块范围误读 | 排除 tests/.d.ts、把 package.json 算模块 |
| `E-ENV` | 环境 | 时间线加载失败、窗口崩溃 |
| `U-TASK` | 任务歧义 | 脚本没说“顺序不重要”等（需复核） |

### 5.4 归因证据要求

每个失败至少要有：

1. 参与者原话（逐字，匿名）。
2. 发生时的视图/动作（首动作、是否回退、是否切视图）。
3. 录屏时间戳或观察者时间戳。
4. 归因理由：为什么是 `V` 而不是 `I`/`F`/`S`。
5. 若归 `U`：第二人复核记录。

没有证据的失败条目不得进入缺陷清单；宁可标 `unresolved`，不要猜。

### 5.5 反作弊/反事后解释

- 不因为“通过率没到 4/5”就回头改 key、放宽判据、把失败改归 `U`。
- 不把“用户没听懂”当主类；必须落到 `V/I/F/S/E`。
- 不用部分分把失败洗成通过。
- 不删掉与预期相反的数据；`not-run`/`timeout`/`unresolved` 都要保留。
- 报告分“做到的 / 没做到的 / 未验证的”三档；读数来自实测输出。

---

## 6. 汇总表模板

### 6.1 每题汇总

| 任务 | 通过人数 `k/5` | 是否 ≥4/5 | `median_all_s` | `median_success_s`（n） | `timeout_count` | 高置信错误率 | 主要归因 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| T1 |  |  |  |  |  |  |  |
| T2 |  |  |  |  |  |  |  |
| T3 |  |  |  |  |  |  |  |
| T4 |  |  |  |  |  |  |  |
| T5 |  |  |  |  |  |  |  |

### 6.2 参与者明细

| participant | task | success | partial | elapsed_s | timeout | confidence | 主归因 | 次归因 | 视图 | 证据 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| P01 | T1 |  |  |  |  |  |  |  |  |  |
| P01 | T2 |  |  |  |  |  |  |  |  |  |
| … | … |  |  |  |  |  |  |  |  |  |

### 6.3 缺陷清单

| id | 任务 | 视图 | 主类/代码 | 观察（匿名原话摘要） | 影响人数 | 证据 | 严重度 | 下一步探针 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| D01 |  |  |  |  |  |  |  |  |

严重度建议：

- `blocker`：任务无法完成且归因明确为 `I`/`V`/`F`；多人重复。
- `major`：任务可完成但耗时显著/易错；或高置信错误。
- `minor`：局部可读性/措辞问题，不影响完成。

### 6.4 按视图汇总

| 视图 | 失败次数 | 主要代码 | 备注 |
| --- | --- | --- | --- |
| map |  |  |  |
| drill |  |  |  |
| path |  |  |  |
| timeline |  |  |  |
| diff |  |  |  |
| shell |  |  |  |

---

## 7. `out/usability.json` 契约

字段沿用票 12，并补充本包需要的归因/校准字段。**只放 `out/`，不提交。**

```json
{
  "meta": {
    "protocol_version": "UK-2026-11-08-01",
    "key_version": "UK-2026-11-08-01",
    "key_sha256": "<首位参与者前记录>",
    "date": "2026-11-08",
    "repo": "privatenumber/tsx",
    "commit": "4473971f4b3261de253209b148d8fdc43269fcc4",
    "history_window": {"maxCommits": 500, "firstParent": true},
    "build": "debug; origin/main 63ea209",
    "machine": "<OS/CPU/分辨率/缩放>",
    "facilitator": "F01",
    "observer": "O01",
    "recording": "out/usability-recordings/"
  },
  "sessions": [
    {
      "participant_id": "P01",
      "pre_exposed": false,
      "chinese_fluent": true,
      "assisted": false,
      "session_timeout": false,
      "tasks": [
        {
          "task": "T1",
          "success": false,
          "partial_score": 0.7,
          "elapsed_s": 300.0,
          "timeout": true,
          "confidence_1_5": 4,
          "prompted": false,
          "answer_verbatim": "……",
          "normalized_answer": "……",
          "views_used": ["map", "path"],
          "first_action": "点播放",
          "hesitation_notes": "……",
          "ui_leaks": ["t1_step_count"],
          "attribution": {
            "view": "path",
            "category": "V",
            "code": "V-PATH",
            "note": "……",
            "secondary": null
          },
          "evidence_ref": "P01_T1 00:03:10",
          "facilitator_note": "……"
        }
      ],
      "debrief": {
        "most_uncertain": "……",
        "one_change": "……"
      }
    }
  ]
}
```

字段规则：

- `participant_id`：`P01`–`P05`，不含姓名/公司/邮箱。
- `success`：布尔；超时且答对也写 `false`。
- `partial_score`：0–1，保留 2 位；仅诊断。
- `elapsed_s`：超时按 cap 截尾；另存 `timeout=true`。
- `confidence_1_5`：整数 1–5；拒答写 `null`。
- `prompted`：T5 使用 A/B/C/D 选项时为 `true`。
- `views_used`：`map|drill|path|timeline|diff|shell` 的子集。
- `ui_leaks`：`t1_step_count`、`t4_counts`、`timeline_legend` 等。
- `attribution.category`：`V|I|F|S|E|U`；`code` 见 §5.3。
- `evidence_ref`：录屏文件 + 时间戳；没有录屏则写观察者记录页码/时间。
- `not-run` 任务也要在 `tasks` 里出现，`success=false`、`timeout=false`、`not_run=true`（附加字段），并写原因。

**校验：**

- 5 个 participant，每人 5 个 task（或 `not-run`）。
- 每个 `success=false` 必须有 `attribution.category` 和 `evidence_ref`。
- 每个 `timeout=true` 必须有 `elapsed_s == cap`。
- `meta.key_sha256` 必须与 `out/usability/key.sha256` 一致。

---

## 8. 报告模板（三档分开）

```markdown
## 做到的
- 5 人 × 5 任务已完成 / 未完成，实际场次……
- T1–T5 的真实通过人数：……
- 可复现 key：命令与读数……
- 录屏/答卷归档位置……

## 没做到的
- 哪些任务未达 4/5；每题 k/5、耗时、超时……
- 按视图归因的缺陷清单（D01…），每条有证据……
- 哪些失败归因不明确（unresolved）……

## 未验证的
- 粒子沿边移动的误读率（R1 未实现）
- excalidraw / 其他仓库 / 更大规模
- 动画开/关的因果效果（无 A/B）
- reduced-motion / 键盘 / 无障碍
- 修复后的效果（需下一轮）
- 任何没有录屏/原话支撑的猜测

## 读数附录
- 原始 `out/usability.json` 位置
- key SHA256
- 每题的实测命令与 stdout（原样）
```

---

## 9. 复核清单（提交报告前）

- [ ] 5 人 × 5 任务的记录齐全；`not-run` 有原因。
- [ ] 每题 `k/5`、`median_all_s`、`median_success_s`、`timeout_count` 都填了。
- [ ] 置信度表与 `high_conf_wrong_rate` 已算；0 分母有说明。
- [ ] 每个失败有主归因 + 视图 + 证据；没有“用户不会用”主类。
- [ ] `U` 条目有第二人复核。
- [ ] key SHA256 未变；若变了，本轮作废并说明。
- [ ] 报告分“做到的 / 没做到的 / 未验证的”。
- [ ] 没有把 5 人结果写成总体百分比/统计显著性。
- [ ] 没有把 T5 结果外推成“粒子误读率”。
- [ ] 没有修改产品代码/测试；本包只产生文档与 `out/` 归档。
