//! ticket 03 / 03b：基于目录、文件名语义的层推断与依赖方向校正。
//!
//! 规则优先级（先命中先返回）：
//! 1. `external-package` → `external`；
//! 2. 文件名后缀（`*.test.*` / `*.spec.*` → `tests`，`*.stories.*` → `components`）；
//! 3. 目录段（大小写不敏感，最深匹配优先）；
//! 4. 拓扑位置只在第 2 步命中多个候选时断平，不用于从零归层。
//!
//! 无法归类的模块进入 [`UNCLASSIFIED`]，并携带 `no-rule-match` 或 `ambiguous` 原因码；
//! 外部包进入独立层 [`EXTERNAL`]，携带 `external` 原因码，但不出现在未分类列表。

use crate::{build_index, IndexArtifact, RepoInfo, ScanError, Stats, UnresolvedItem};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// 固定层顺序。视图的层带按此顺序自上而下排列。
pub const LAYER_ORDER: [&str; 8] = [
    "cli",
    "commands",
    "components",
    "server",
    "utils",
    "tests",
    EXTERNAL,
    UNCLASSIFIED,
];

pub const EXTERNAL: &str = "external";
pub const UNCLASSIFIED: &str = "unclassified";

pub const REASON_NO_RULE_MATCH: &str = "no-rule-match";
pub const REASON_AMBIGUOUS: &str = "ambiguous";
pub const REASON_EXTERNAL: &str = "external";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LayersArtifact {
    pub schema_version: u32,
    pub tool_version: String,
    pub repo: RepoInfo,
    pub stats: Stats,
    pub layer_order: Vec<&'static str>,
    pub modules: Vec<LayeredModule>,
    pub unclassified: Vec<UnclassifiedModule>,
    pub edges: Vec<LayeredEdge>,
    pub unresolved: Vec<UnresolvedItem>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LayeredModule {
    pub id: String,
    pub path: String,
    pub kind: &'static str,
    pub language: &'static str,
    pub lines: usize,
    pub layer: &'static str,
    /// 命中的规则标识，如 `kind:external-package`、`dir:components`、`topology:tests:support=2`。
    /// 未归类或未断平时为 `None`。
    pub rule: Option<String>,
    /// 仅 `external` 与 `unclassified` 携带原因码。
    pub reason: Option<&'static str>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnclassifiedModule {
    pub id: String,
    pub path: String,
    pub reason: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LayeredEdge {
    pub source: String,
    pub target: String,
    pub kind: String,
    /// 方向规则判定为反向时置 `true`；原边保留，不静默删除。
    pub direction_adjusted: bool,
}

/// 构建层推断产物。只读取目标仓库，不写入。
pub fn build_layers(root: &Path, tool_version: &str) -> Result<LayersArtifact, ScanError> {
    let index = build_index(root, tool_version)?;
    Ok(layers_from_index(index))
}

/// 将模块级索引转换为层推断产物。
pub fn layers_from_index(index: IndexArtifact) -> LayersArtifact {
    let IndexArtifact {
        schema_version,
        tool_version,
        repo,
        stats,
        modules,
        edges,
        unresolved,
    } = index;

    let naming_by_id = modules
        .iter()
        .map(|module| (module.id.clone(), naming_match(&module.path, module.kind)))
        .collect::<BTreeMap<_, _>>();

    let support_by_id = neighbor_layer_support(&edges, &naming_by_id);

    let mut layered = Vec::with_capacity(modules.len());
    let mut unclassified = Vec::new();
    for module in modules {
        let decision = classify(
            naming_by_id.get(&module.id).expect("naming entry"),
            support_by_id.get(&module.id),
        );
        if decision.layer == UNCLASSIFIED {
            unclassified.push(UnclassifiedModule {
                id: module.id.clone(),
                path: module.path.clone(),
                reason: decision.reason.expect("unclassified reason"),
            });
        }
        layered.push(LayeredModule {
            id: module.id,
            path: module.path,
            kind: module.kind,
            language: module.language,
            lines: module.lines,
            layer: decision.layer,
            rule: decision.rule,
            reason: decision.reason,
        });
    }

    let layer_by_id = layered
        .iter()
        .map(|module| (module.id.as_str(), module.layer))
        .collect::<BTreeMap<_, _>>();

    let edges = edges
        .into_iter()
        .map(|edge| {
            let source_layer = layer_by_id.get(edge.source.as_str()).copied();
            let target_layer = layer_by_id.get(edge.target.as_str()).copied();
            let direction_adjusted = match (source_layer, target_layer) {
                (Some(source), Some(target)) => direction_is_adjusted(source, target),
                _ => false,
            };
            LayeredEdge {
                source: edge.source,
                target: edge.target,
                kind: edge.kind,
                direction_adjusted,
            }
        })
        .collect();

    LayersArtifact {
        schema_version,
        tool_version,
        repo,
        stats,
        layer_order: LAYER_ORDER.to_vec(),
        modules: layered,
        unclassified,
        edges,
        unresolved,
    }
}

/// 仅按命名规则推断层，供单元测试与规则解释使用。
/// 命名冲突或没有匹配时返回 [`UNCLASSIFIED`]；外部包返回 [`EXTERNAL`]。
pub fn layer_for_module(path: &str, kind: &str) -> &'static str {
    match naming_match(path, kind) {
        NamingMatch::External => EXTERNAL,
        NamingMatch::Unique { layer, .. } => layer,
        NamingMatch::Conflict { .. } | NamingMatch::None => UNCLASSIFIED,
    }
}

#[derive(Debug, Clone)]
enum NamingMatch {
    External,
    Unique { layer: &'static str, rule: String },
    Conflict { candidates: Vec<&'static str> },
    None,
}

struct LayerDecision {
    layer: &'static str,
    rule: Option<String>,
    reason: Option<&'static str>,
}

impl LayerDecision {
    fn assigned(layer: &'static str, rule: String) -> Self {
        Self {
            layer,
            rule: Some(rule),
            reason: None,
        }
    }

    fn unclassified(reason: &'static str) -> Self {
        Self {
            layer: UNCLASSIFIED,
            rule: None,
            reason: Some(reason),
        }
    }
}

fn naming_match(path: &str, kind: &str) -> NamingMatch {
    if kind == "external-package" {
        return NamingMatch::External;
    }

    let file_name = path.rsplit('/').next().unwrap_or(path);
    let lower_file = file_name.to_ascii_lowercase();
    let is_test = lower_file.contains(".test.") || lower_file.contains(".spec.");
    let is_story = lower_file.contains(".stories.");

    if is_test && is_story {
        return NamingMatch::Conflict {
            candidates: vec!["tests", "components"],
        };
    }
    if is_test {
        return NamingMatch::Unique {
            layer: "tests",
            rule: "file:test-suffix".to_string(),
        };
    }
    if is_story {
        return NamingMatch::Unique {
            layer: "components",
            rule: "file:stories-suffix".to_string(),
        };
    }

    let mut directories = path.split('/');
    let _file = directories.next_back();
    let mut matched = None;
    for directory in directories {
        if let Some(layer) = layer_for_segment(directory) {
            matched = Some((layer, directory.to_ascii_lowercase()));
        }
    }

    match matched {
        Some((layer, segment)) => NamingMatch::Unique {
            layer,
            rule: format!("dir:{segment}"),
        },
        None => NamingMatch::None,
    }
}

fn layer_for_segment(segment: &str) -> Option<&'static str> {
    let segment = segment.to_ascii_lowercase();
    match segment.as_str() {
        "cli" => Some("cli"),
        "command" | "commands" | "action" | "actions" => Some("commands"),
        "component" | "components" | "ui" | "hooks" | "renderer" => Some("components"),
        "server" => Some("server"),
        "util" | "utils" | "lib" | "common" | "shared" => Some("utils"),
        "test" | "tests" | "__tests__" | "spec" | "specs" => Some("tests"),
        "app" | "apps" => Some("components"),
        _ if segment.ends_with("-app") => Some("components"),
        _ => None,
    }
}

/// 统计每个模块的直接邻居中、已由命名规则归层的模块层分布。
///
/// 只采纳命名结果（不采纳拓扑结果），且排除 `external` 与 `unclassified`，
/// 避免拓扑结果自我强化或把外部依赖当作层证据。
fn neighbor_layer_support(
    edges: &[crate::Edge],
    naming_by_id: &BTreeMap<String, NamingMatch>,
) -> BTreeMap<String, BTreeMap<&'static str, usize>> {
    let mut neighbors = BTreeMap::<String, BTreeSet<String>>::new();
    for edge in edges {
        neighbors
            .entry(edge.source.clone())
            .or_default()
            .insert(edge.target.clone());
        neighbors
            .entry(edge.target.clone())
            .or_default()
            .insert(edge.source.clone());
    }

    let mut support = BTreeMap::new();
    for (module, adjacent) in neighbors {
        let mut counts = BTreeMap::<&'static str, usize>::new();
        for neighbor in adjacent {
            if let Some(NamingMatch::Unique { layer, .. }) = naming_by_id.get(&neighbor) {
                if *layer != EXTERNAL && *layer != UNCLASSIFIED {
                    *counts.entry(layer).or_insert(0) += 1;
                }
            }
        }
        support.insert(module, counts);
    }
    support
}

fn classify(
    naming: &NamingMatch,
    support: Option<&BTreeMap<&'static str, usize>>,
) -> LayerDecision {
    match naming {
        NamingMatch::External => LayerDecision {
            layer: EXTERNAL,
            rule: Some("kind:external-package".to_string()),
            reason: Some(REASON_EXTERNAL),
        },
        NamingMatch::Unique { layer, rule } => LayerDecision::assigned(layer, rule.clone()),
        NamingMatch::None => LayerDecision::unclassified(REASON_NO_RULE_MATCH),
        NamingMatch::Conflict { candidates } => {
            let counts = support.cloned().unwrap_or_default();
            let max = candidates
                .iter()
                .map(|candidate| counts.get(candidate).copied().unwrap_or(0))
                .max()
                .unwrap_or(0);
            if max >= 2 {
                let winners = candidates
                    .iter()
                    .filter(|candidate| counts.get(**candidate).copied().unwrap_or(0) == max)
                    .copied()
                    .collect::<Vec<_>>();
                if winners.len() == 1 {
                    let winner = winners[0];
                    return LayerDecision::assigned(
                        winner,
                        format!("topology:{winner}:support={max}"),
                    );
                }
            }
            LayerDecision::unclassified(REASON_AMBIGUOUS)
        }
    }
}

/// 层的外向程度。数值越大越靠近入口；依赖应从大到小。
fn layer_rank(layer: &str) -> Option<u8> {
    match layer {
        "cli" => Some(4),
        "commands" => Some(3),
        "components" => Some(2),
        "server" => Some(1),
        "utils" => Some(0),
        "tests" => Some(5),
        _ => None,
    }
}

fn direction_is_adjusted(source: &str, target: &str) -> bool {
    match (layer_rank(source), layer_rank(target)) {
        (Some(source), Some(target)) => source < target,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_semantics_cover_fixed_layers() {
        assert_eq!(layer_for_module("cli/main.ts", "source"), "cli");
        assert_eq!(
            layer_for_module("src/commands/run.ts", "source"),
            "commands"
        );
        assert_eq!(
            layer_for_module("src/components/Button.tsx", "source"),
            "components"
        );
        assert_eq!(layer_for_module("src/server/api.ts", "source"), "server");
        assert_eq!(layer_for_module("src/utils/format.ts", "source"), "utils");
        assert_eq!(
            layer_for_module("src/tests/format.test.ts", "source"),
            "tests"
        );
    }

    #[test]
    fn directory_semantics_cover_common_repository_shapes() {
        assert_eq!(layer_for_module("lib/helper.ts", "source"), "utils");
        assert_eq!(layer_for_module("app/page.tsx", "source"), "components");
        assert_eq!(
            layer_for_module("packages/cli/src/main.ts", "source"),
            "cli"
        );
        assert_eq!(
            layer_for_module("packages/web/src/components/Badge.tsx", "source"),
            "components"
        );
        assert_eq!(
            layer_for_module("packages/web/src/utils/pkg-format.ts", "source"),
            "utils"
        );
    }

    #[test]
    fn filename_conventions_apply_before_directory_rules() {
        assert_eq!(
            layer_for_module("src/utils/math.test.ts", "source"),
            "tests"
        );
        assert_eq!(
            layer_for_module("src/utils/story.stories.tsx", "source"),
            "components"
        );
        assert_eq!(
            layer_for_module("src/utils/both.test.stories.tsx", "source"),
            UNCLASSIFIED
        );
    }

    #[test]
    fn deepest_semantic_directory_wins() {
        assert_eq!(
            layer_for_module("src/commands/utils/helper.ts", "source"),
            "utils"
        );
    }

    #[test]
    fn unknown_paths_stay_unclassified_but_external_packages_do_not() {
        assert_eq!(layer_for_module("misc/notes.ts", "source"), UNCLASSIFIED);
        assert_eq!(
            layer_for_module("src/features/random.ts", "source"),
            UNCLASSIFIED
        );
        assert_eq!(
            layer_for_module("packages/web/src/index.ts", "source"),
            UNCLASSIFIED
        );
        assert_eq!(layer_for_module("react", "external-package"), EXTERNAL);
    }

    #[test]
    fn topology_breaks_only_naming_conflicts() {
        let mut support = BTreeMap::new();
        support.insert("tests", 2);
        let decision = classify(
            &NamingMatch::Conflict {
                candidates: vec!["tests", "components"],
            },
            Some(&support),
        );
        assert_eq!(decision.layer, "tests");
        assert_eq!(decision.rule.as_deref(), Some("topology:tests:support=2"));

        let mut tied = BTreeMap::new();
        tied.insert("tests", 2);
        tied.insert("components", 2);
        let decision = classify(
            &NamingMatch::Conflict {
                candidates: vec!["tests", "components"],
            },
            Some(&tied),
        );
        assert_eq!(decision.layer, UNCLASSIFIED);
        assert_eq!(decision.reason, Some(REASON_AMBIGUOUS));

        let decision = classify(&NamingMatch::None, None);
        assert_eq!(decision.layer, UNCLASSIFIED);
        assert_eq!(decision.reason, Some(REASON_NO_RULE_MATCH));
    }

    #[test]
    fn direction_violations_are_marked_without_ranking_unknown_layers() {
        assert!(direction_is_adjusted("utils", "commands"));
        assert!(!direction_is_adjusted("cli", "commands"));
        assert!(!direction_is_adjusted("commands", "utils"));
        assert!(!direction_is_adjusted(UNCLASSIFIED, "cli"));
        assert!(!direction_is_adjusted(EXTERNAL, "cli"));
    }
}
