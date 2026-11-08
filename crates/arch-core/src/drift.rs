//! N1：架构漂移检测（B+C 组合）。
//!
//! 两张产物：
//! - **基线** [`DriftBaseline`]：仓库内显式提交的 `drift-baseline.json`。包含层聚合
//!   （门禁判据）与逐模块 `id → layer` 明细（解释材料）；不依赖历史 tree 索引。
//! - **报告** [`DriftReport`]：与基线比较后的门禁结论、逐条违规与模块级 `added` /
//!   `removed` / `moved` / `newViolations` / `resolvedViolations`。
//!
//! # 四类门禁判据（只拦「新增违规」，默认容差 0）
//!
//! 1. **层人口突变**：每层模块计数相对基线变化。容差由基线字段 `populationTolerance`
//!    显式记录（本项目取 `0`：任何计数变化都要求重新录制基线，而不是静默通过）。
//!    违规带 `layer` / `baseline` / `current` / `delta`。层人口按「层」聚合，不看模块身份，
//!    因此合法新增模块也会触发——这是有意的：重新录制基线必须是一次独立的、写明理由的提交。
//! 2. **新增反向边**：`directionAdjusted == true` 的 `(source, target, kind)` 出现在当前产物、
//!    但不在基线中。违规带两端所在层，解释「哪条边、从哪层到哪层违反了外向程度」。
//!    已存在的反向边记录在基线里，不重复计数；消失的反向边进入 `resolvedViolations`。
//! 3. **新增循环**：模块级图的强连通分量（SCC，成员数 ≥2 或自环）的**成员集合**不在基线中。
//!    一个 SCC 只产生一条记录，不按环上的边重复计数；基线里已存在的循环不重复上报。
//!    只按 SCC 计数意味着「同一组模块里新增第二条环边」不会被当成新循环。
//! 4. **external 泄漏**：`kind == "external-package"` 的模块出现在 `external` 之外的层。
//!    违规带 `module` 与 `layer`，即「哪一层被外部包污染」。基线同样记录该集合，
//!    因此新增与已解决分开计数。
//!
//! # 事实边界（external 泄漏）
//!
//! 实测（`arch-cli layers tests/fixtures/layers`）：所有 `external-package` 模块都落在
//! `external` 层（1/1），因为 [`crate::layers`] 的分类规则对 `kind == "external-package"`
//! 无条件返回 `external`。也就是说，这条判据在当前管线下**不会由真实仓库触发**，
//! 保留它是为了防「分类规则退化导致外部包漏进业务层」的回归；正向基线恒为空。
//! 该判据可用合成 [`LayersArtifact`] 触发（见 `crates/arch-core/tests/drift.rs`）。
//!
//! # 模块明细
//!
//! 不参与门禁判定，只用于定位：`added` / `removed` / `moved` 由基线中显式的模块集合做差。
//! **重命名不做归并**：同一路径改名会同时出现在 `added` 与 `removed`，不会静默变成
//! 「无变化」。这与提案原判据 3 / 5（用 `git diff -M` 归并 rename）相反，理由见
//! `docs/proposals/n1-architecture-drift.md`：显式基线文件没有历史 tree，且用户要求
//! rename 必须显式可见。
//!
//! 所有集合按字典序输出，同一输入两次运行字节一致。

use crate::layers::{build_layers, LayeredModule, LayersArtifact, EXTERNAL, LAYER_ORDER};
use crate::ScanError;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// 漂移产物的独立 schema 版本，不复用 index / layers / callgraph / path / history 的版本号。
pub const DRIFT_SCHEMA_VERSION: u32 = 1;

/// 层人口容差默认值：0 表示任何计数变化都算漂移。
pub const DEFAULT_POPULATION_TOLERANCE: u32 = 0;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriftBaseline {
    pub schema_version: u32,
    pub tool_version: String,
    /// 录制基线时的仓库 HEAD；非 git 仓库为 `None`。
    pub recorded_head: Option<String>,
    /// 层人口容差（模块计数变化超过该值才算漂移）；本项目显式取 0。
    pub population_tolerance: u32,
    pub layers: Vec<LayerPopulation>,
    pub reverse_edges: Vec<ReverseEdgeKey>,
    /// 每个循环一条记录，成员按字典序排序。
    pub cycles: Vec<Vec<String>>,
    pub external_leaks: Vec<ExternalLeakRecord>,
    /// 模块明细：`id → layer`，按 `id` 排序。
    pub modules: Vec<BaselineModule>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub struct LayerPopulation {
    pub layer: String,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub struct ReverseEdgeKey {
    pub source: String,
    pub target: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub struct ExternalLeakRecord {
    pub module: String,
    pub layer: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BaselineModule {
    pub id: String,
    pub layer: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriftReport {
    pub schema_version: u32,
    pub tool_version: String,
    pub baseline: BaselineSummary,
    pub gate: DriftGate,
    pub modules: ModuleDrift,
    pub new_violations: Vec<Violation>,
    pub resolved_violations: Vec<Violation>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BaselineSummary {
    pub schema_version: u32,
    pub tool_version: String,
    pub recorded_head: Option<String>,
    pub population_tolerance: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriftGate {
    pub passed: bool,
    pub checks: GateChecks,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GateChecks {
    pub layer_population: CriterionCheck,
    pub reverse_edges: CriterionCheck,
    pub cycles: CriterionCheck,
    pub external_leaks: CriterionCheck,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CriterionCheck {
    pub new_count: usize,
    pub resolved_count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleDrift {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub moved: Vec<MovedModule>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MovedModule {
    pub id: String,
    pub from_layer: String,
    pub to_layer: String,
}

/// 一条可解释的漂移记录；`kind` 决定其余字段。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Violation {
    /// 层人口变化：`current - baseline`，正负都是漂移。
    LayerPopulation {
        layer: String,
        baseline: usize,
        current: usize,
        delta: i64,
    },
    /// 新增/消失的反向边：两端所在层用于解释方向规则。
    ReverseEdge {
        source: String,
        target: String,
        edge_kind: String,
        source_layer: String,
        target_layer: String,
    },
    /// 一个强连通分量（SCC）；成员按字典序排序。
    Cycle { modules: Vec<String> },
    /// 外部包出现在 `external` 之外的层。
    ExternalLeak {
        module: String,
        layer: String,
        expected_layer: String,
    },
}

/// 录制基线。纯函数：只读 `layers`，不写入。
pub fn record_baseline(layers: &LayersArtifact, tool_version: &str) -> DriftBaseline {
    let current_counts = population_by_layer(&layers.modules);

    let mut layer_names = LAYER_ORDER
        .iter()
        .map(|layer| layer.to_string())
        .collect::<Vec<_>>();
    for layer in current_counts.keys() {
        if !layer_names.iter().any(|known| known == layer) {
            layer_names.push(layer.clone());
        }
    }
    layer_names.sort();
    let layers_population = layer_names
        .into_iter()
        .map(|layer| LayerPopulation {
            count: current_counts.get(&layer).copied().unwrap_or(0),
            layer,
        })
        .collect();

    let mut reverse_edges = reverse_edges_of(layers);
    reverse_edges.sort();

    let mut external_leaks = external_leaks_of(layers);
    external_leaks.sort();

    let mut modules = layers
        .modules
        .iter()
        .map(|module| BaselineModule {
            id: module.id.clone(),
            layer: module.layer.to_string(),
        })
        .collect::<Vec<_>>();
    modules.sort_by(|left, right| left.id.cmp(&right.id));

    DriftBaseline {
        schema_version: DRIFT_SCHEMA_VERSION,
        tool_version: tool_version.to_string(),
        recorded_head: layers.repo.head.clone(),
        population_tolerance: DEFAULT_POPULATION_TOLERANCE,
        layers: layers_population,
        reverse_edges,
        cycles: cycles_of(layers),
        external_leaks,
        modules,
    }
}

/// 与基线比较并生成门禁报告。
pub fn evaluate(
    baseline: &DriftBaseline,
    layers: &LayersArtifact,
    tool_version: &str,
) -> DriftReport {
    let current_counts = population_by_layer(&layers.modules);
    let baseline_counts = baseline
        .layers
        .iter()
        .map(|entry| (entry.layer.clone(), entry.count))
        .collect::<BTreeMap<String, usize>>();

    let mut population_violations = Vec::new();
    let mut checked_layers = baseline_counts.keys().cloned().collect::<BTreeSet<_>>();
    checked_layers.extend(current_counts.keys().cloned());
    for layer in checked_layers {
        let old = baseline_counts.get(&layer).copied().unwrap_or(0);
        let new = current_counts.get(&layer).copied().unwrap_or(0);
        let delta = new as i64 - old as i64;
        if delta.abs() > i64::from(baseline.population_tolerance) {
            population_violations.push(Violation::LayerPopulation {
                layer,
                baseline: old,
                current: new,
                delta,
            });
        }
    }

    let layer_by_id = layers
        .modules
        .iter()
        .map(|module| (module.id.as_str(), module.layer))
        .collect::<BTreeMap<_, _>>();

    let current_reverse = reverse_edges_of(layers)
        .into_iter()
        .collect::<BTreeSet<_>>();
    let baseline_reverse = baseline
        .reverse_edges
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let new_reverse = current_reverse
        .difference(&baseline_reverse)
        .cloned()
        .collect::<Vec<_>>();
    let resolved_reverse = baseline_reverse
        .difference(&current_reverse)
        .cloned()
        .collect::<Vec<_>>();

    let current_cycles = cycles_of(layers).into_iter().collect::<BTreeSet<_>>();
    let baseline_cycles = baseline.cycles.iter().cloned().collect::<BTreeSet<_>>();
    let new_cycles = current_cycles
        .difference(&baseline_cycles)
        .cloned()
        .collect::<Vec<_>>();
    let resolved_cycles = baseline_cycles
        .difference(&current_cycles)
        .cloned()
        .collect::<Vec<_>>();

    let current_leaks = external_leaks_of(layers)
        .into_iter()
        .collect::<BTreeSet<_>>();
    let baseline_leaks = baseline
        .external_leaks
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let new_leaks = current_leaks
        .difference(&baseline_leaks)
        .cloned()
        .collect::<Vec<_>>();
    let resolved_leaks = baseline_leaks
        .difference(&current_leaks)
        .cloned()
        .collect::<Vec<_>>();

    let mut new_violations = population_violations;
    new_violations.extend(new_reverse.into_iter().map(|edge| {
        Violation::ReverseEdge {
            source_layer: layer_by_id
                .get(edge.source.as_str())
                .copied()
                .unwrap_or("<unknown>")
                .to_string(),
            target_layer: layer_by_id
                .get(edge.target.as_str())
                .copied()
                .unwrap_or("<unknown>")
                .to_string(),
            source: edge.source,
            target: edge.target,
            edge_kind: edge.kind,
        }
    }));
    new_violations.extend(
        new_cycles
            .into_iter()
            .map(|modules| Violation::Cycle { modules }),
    );
    new_violations.extend(new_leaks.into_iter().map(|leak| Violation::ExternalLeak {
        module: leak.module,
        layer: leak.layer,
        expected_layer: EXTERNAL.to_string(),
    }));

    let mut resolved_violations = resolved_reverse
        .into_iter()
        .map(|edge| Violation::ReverseEdge {
            source_layer: layer_by_id
                .get(edge.source.as_str())
                .copied()
                .unwrap_or("<unknown>")
                .to_string(),
            target_layer: layer_by_id
                .get(edge.target.as_str())
                .copied()
                .unwrap_or("<unknown>")
                .to_string(),
            source: edge.source,
            target: edge.target,
            edge_kind: edge.kind,
        })
        .collect::<Vec<_>>();
    resolved_violations.extend(
        resolved_cycles
            .into_iter()
            .map(|modules| Violation::Cycle { modules }),
    );
    resolved_violations.extend(
        resolved_leaks
            .into_iter()
            .map(|leak| Violation::ExternalLeak {
                module: leak.module,
                layer: leak.layer,
                expected_layer: EXTERNAL.to_string(),
            }),
    );

    let checks = GateChecks {
        layer_population: CriterionCheck {
            new_count: population_count(&new_violations, Violation::is_layer_population),
            resolved_count: 0,
        },
        reverse_edges: CriterionCheck {
            new_count: new_violations
                .iter()
                .filter(|violation| matches!(violation, Violation::ReverseEdge { .. }))
                .count(),
            resolved_count: resolved_violations
                .iter()
                .filter(|violation| matches!(violation, Violation::ReverseEdge { .. }))
                .count(),
        },
        cycles: CriterionCheck {
            new_count: new_violations
                .iter()
                .filter(|violation| matches!(violation, Violation::Cycle { .. }))
                .count(),
            resolved_count: resolved_violations
                .iter()
                .filter(|violation| matches!(violation, Violation::Cycle { .. }))
                .count(),
        },
        external_leaks: CriterionCheck {
            new_count: new_violations
                .iter()
                .filter(|violation| matches!(violation, Violation::ExternalLeak { .. }))
                .count(),
            resolved_count: resolved_violations
                .iter()
                .filter(|violation| matches!(violation, Violation::ExternalLeak { .. }))
                .count(),
        },
    };

    DriftReport {
        schema_version: DRIFT_SCHEMA_VERSION,
        tool_version: tool_version.to_string(),
        baseline: BaselineSummary {
            schema_version: baseline.schema_version,
            tool_version: baseline.tool_version.clone(),
            recorded_head: baseline.recorded_head.clone(),
            population_tolerance: baseline.population_tolerance,
        },
        gate: DriftGate {
            passed: new_violations.is_empty(),
            checks,
        },
        modules: module_drift(&baseline.modules, &layers.modules),
        new_violations,
        resolved_violations,
    }
}

/// 便捷入口：从磁盘仓库构建层产物再与基线比较。
pub fn build_drift(
    root: &Path,
    tool_version: &str,
    baseline: &DriftBaseline,
) -> Result<DriftReport, ScanError> {
    let layers = build_layers(root, tool_version)?;
    Ok(evaluate(baseline, &layers, tool_version))
}

impl Violation {
    fn is_layer_population(&self) -> bool {
        matches!(self, Violation::LayerPopulation { .. })
    }
}

fn population_count(violations: &[Violation], predicate: fn(&Violation) -> bool) -> usize {
    violations
        .iter()
        .filter(|violation| predicate(violation))
        .count()
}

fn population_by_layer(modules: &[LayeredModule]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for module in modules {
        *counts.entry(module.layer.to_string()).or_insert(0) += 1;
    }
    counts
}

fn reverse_edges_of(layers: &LayersArtifact) -> Vec<ReverseEdgeKey> {
    layers
        .edges
        .iter()
        .filter(|edge| edge.direction_adjusted)
        .map(|edge| ReverseEdgeKey {
            source: edge.source.clone(),
            target: edge.target.clone(),
            kind: edge.kind.clone(),
        })
        .collect()
}

/// 只有「外部包落在非 external 层」算泄漏。
///
/// 反方向（源模块被放进 `external`）不在此判据内：它不是「某一层被外部包污染」，
/// 且当前分类规则也不会产生；若未来需要，应作为独立的不变量，而不是悄悄并进这一类。
fn external_leaks_of(layers: &LayersArtifact) -> Vec<ExternalLeakRecord> {
    layers
        .modules
        .iter()
        .filter(|module| module.kind == "external-package" && module.layer != EXTERNAL)
        .map(|module| ExternalLeakRecord {
            module: module.id.clone(),
            layer: module.layer.to_string(),
        })
        .collect()
}

fn module_drift(baseline: &[BaselineModule], current: &[LayeredModule]) -> ModuleDrift {
    let baseline_by_id = baseline
        .iter()
        .map(|module| (module.id.as_str(), module.layer.as_str()))
        .collect::<BTreeMap<_, _>>();
    let current_by_id = current
        .iter()
        .map(|module| (module.id.as_str(), module.layer))
        .collect::<BTreeMap<_, _>>();

    let added = current_by_id
        .keys()
        .filter(|id| !baseline_by_id.contains_key(**id))
        .map(|id| id.to_string())
        .collect::<Vec<_>>();
    let removed = baseline_by_id
        .keys()
        .filter(|id| !current_by_id.contains_key(**id))
        .map(|id| id.to_string())
        .collect::<Vec<_>>();

    let mut moved = current_by_id
        .iter()
        .filter_map(|(id, layer)| {
            baseline_by_id.get(*id).and_then(|old| {
                if *old == *layer {
                    None
                } else {
                    Some(MovedModule {
                        id: id.to_string(),
                        from_layer: old.to_string(),
                        to_layer: layer.to_string(),
                    })
                }
            })
        })
        .collect::<Vec<_>>();
    moved.sort_by(|left, right| left.id.cmp(&right.id));

    ModuleDrift {
        added,
        removed,
        moved,
    }
}

/// 模块级循环：强连通分量（成员数 ≥2，或单点自环）。
fn cycles_of(layers: &LayersArtifact) -> Vec<Vec<String>> {
    let ids = layers
        .modules
        .iter()
        .map(|module| module.id.clone())
        .collect::<Vec<_>>();
    let index = ids
        .iter()
        .enumerate()
        .map(|(position, id)| (id.as_str(), position))
        .collect::<BTreeMap<_, _>>();
    let node_count = ids.len();

    let mut adjacency = vec![Vec::new(); node_count];
    let mut reverse_adjacency = vec![Vec::new(); node_count];
    let mut self_loop = vec![false; node_count];
    for edge in &layers.edges {
        let (Some(&source), Some(&target)) = (
            index.get(edge.source.as_str()),
            index.get(edge.target.as_str()),
        ) else {
            continue;
        };
        if source == target {
            self_loop[source] = true;
            continue;
        }
        adjacency[source].push(target);
        reverse_adjacency[target].push(source);
    }
    for list in adjacency.iter_mut().chain(reverse_adjacency.iter_mut()) {
        list.sort_unstable();
        list.dedup();
    }

    let mut visited = vec![false; node_count];
    let mut order = Vec::with_capacity(node_count);
    for start in 0..node_count {
        if visited[start] {
            continue;
        }
        visited[start] = true;
        let mut stack = vec![(start, 0usize)];
        while let Some((node, position)) = stack.pop() {
            if position < adjacency[node].len() {
                stack.push((node, position + 1));
                let next = adjacency[node][position];
                if !visited[next] {
                    visited[next] = true;
                    stack.push((next, 0));
                }
            } else {
                order.push(node);
            }
        }
    }

    let mut component = vec![usize::MAX; node_count];
    let mut components = Vec::<Vec<usize>>::new();
    for &start in order.iter().rev() {
        if component[start] != usize::MAX {
            continue;
        }
        let component_id = components.len();
        component[start] = component_id;
        let mut members = Vec::new();
        let mut stack = vec![start];
        while let Some(node) = stack.pop() {
            members.push(node);
            for &next in &reverse_adjacency[node] {
                if component[next] == usize::MAX {
                    component[next] = component_id;
                    stack.push(next);
                }
            }
        }
        components.push(members);
    }

    let mut cycles = components
        .into_iter()
        .filter(|members| members.len() >= 2 || (members.len() == 1 && self_loop[members[0]]))
        .map(|members| {
            let mut ids = members
                .into_iter()
                .map(|position| ids[position].clone())
                .collect::<Vec<_>>();
            ids.sort();
            ids
        })
        .collect::<Vec<_>>();
    cycles.sort();
    cycles
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drift_schema_version_is_its_own_contract() {
        assert_eq!(DRIFT_SCHEMA_VERSION, 1);
        assert_eq!(DEFAULT_POPULATION_TOLERANCE, 0);
    }
}
