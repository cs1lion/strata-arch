//! ticket 03 分层地图的纯布局模型。
//!
//! 结构断言先于实现：节点矩形、边端点、标签包围盒、z-order、命中框都在此模型上断言，
//! GPUI 只负责把该模型画出来，不参与布局计算。

use arch_core::layers::{LayersArtifact, EXTERNAL, LAYER_ORDER, UNCLASSIFIED};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn center(&self) -> Point {
        Point {
            x: self.x + self.width / 2.0,
            y: self.y + self.height / 2.0,
        }
    }

    pub fn contains(&self, point: Point) -> bool {
        point.x >= self.x
            && point.x <= self.x + self.width
            && point.y >= self.y
            && point.y <= self.y + self.height
    }

    pub fn intersection_area(&self, other: Rect) -> f32 {
        let left = self.x.max(other.x);
        let top = self.y.max(other.y);
        let right = (self.x + self.width).min(other.x + other.width);
        let bottom = (self.y + self.height).min(other.y + other.height);
        if right <= left || bottom <= top {
            0.0
        } else {
            (right - left) * (bottom - top)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BandLayout {
    pub layer: &'static str,
    pub rect: Rect,
    pub node_count: usize,
    pub z_index: i32,
    pub is_unclassified: bool,
    pub is_external: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeLayout {
    pub id: String,
    pub layer: &'static str,
    pub rect: Rect,
    pub label_rect: Rect,
    pub hit_rect: Rect,
    pub z_index: i32,
    /// `layer`、`external` 或 `unclassified`；视图据此给不同来源独立视觉标记。
    pub visual_role: &'static str,
    pub is_unclassified: bool,
    pub is_external: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EdgeLayout {
    pub source: String,
    pub target: String,
    pub direction_adjusted: bool,
    pub from: Point,
    pub to: Point,
    pub z_index: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MapLayout {
    pub width: f32,
    pub height: f32,
    pub bands: Vec<BandLayout>,
    pub nodes: Vec<NodeLayout>,
    pub edges: Vec<EdgeLayout>,
    /// 同层节点包围盒重叠面积总和；用于基线断言。
    pub overlap_area: f32,
    pub overlap_area_threshold: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct LayoutConfig {
    pub width: f32,
    pub height: f32,
    pub band_gap: f32,
    pub band_header: f32,
    pub band_padding: f32,
    pub node_width: f32,
    pub node_height: f32,
    pub node_gap_x: f32,
    pub node_gap_y: f32,
    pub overlap_area_threshold: f32,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            width: 1200.0,
            height: 800.0,
            band_gap: 24.0,
            band_header: 28.0,
            band_padding: 14.0,
            node_width: 168.0,
            node_height: 44.0,
            node_gap_x: 16.0,
            node_gap_y: 12.0,
            overlap_area_threshold: 0.0,
        }
    }
}

pub fn layout_map(artifact: &LayersArtifact, config: LayoutConfig) -> MapLayout {
    let columns = (((config.width - 2.0 * config.band_padding + config.node_gap_x)
        / (config.node_width + config.node_gap_x))
        .floor()
        .max(1.0)) as usize;

    let mut bands = Vec::new();
    let mut nodes = Vec::new();
    let mut cursor_y = 0.0;

    let mut layer_groups: Vec<(&'static str, Vec<String>)> = LAYER_ORDER
        .iter()
        .filter_map(|&layer| {
            let ids = artifact
                .modules
                .iter()
                .filter(|m| m.layer == layer)
                .map(|m| m.id.clone())
                .collect::<Vec<_>>();
            if ids.is_empty() {
                None
            } else {
                Some((layer, ids))
            }
        })
        .collect();
    let edge_pairs = artifact
        .edges
        .iter()
        .map(|e| (e.source.clone(), e.target.clone()))
        .collect::<Vec<_>>();
    order_layers_for_readability(&mut layer_groups, &edge_pairs);

    for (layer, layer_modules) in layer_groups {

        let rows = layer_modules.len().div_ceil(columns);
        let band_height = config.band_header
            + 2.0 * config.band_padding
            + rows as f32 * config.node_height
            + rows.saturating_sub(1) as f32 * config.node_gap_y;
        let is_unclassified = layer == UNCLASSIFIED;
        let is_external = layer == EXTERNAL;
        bands.push(BandLayout {
            layer,
            rect: Rect::new(0.0, cursor_y, config.width, band_height),
            node_count: layer_modules.len(),
            z_index: 0,
            is_unclassified,
            is_external,
        });

        for (index, module_id) in layer_modules.iter().enumerate() {
            let row = index / columns;
            let column = index % columns;
            let x = config.band_padding + column as f32 * (config.node_width + config.node_gap_x);
            let y = cursor_y
                + config.band_header
                + config.band_padding
                + row as f32 * (config.node_height + config.node_gap_y);
            let rect = Rect::new(x, y, config.node_width, config.node_height);
            let label_rect = Rect::new(
                x + 6.0,
                y + 6.0,
                (config.node_width - 12.0).max(0.0),
                (config.node_height - 12.0).max(0.0),
            );
            let z_index = 1000 + nodes.len() as i32;
            nodes.push(NodeLayout {
                id: module_id.clone(),
                layer,
                rect,
                label_rect,
                hit_rect: rect,
                z_index,
                visual_role: if is_unclassified {
                    "unclassified"
                } else if is_external {
                    "external"
                } else {
                    "layer"
                },
                is_unclassified,
                is_external,
            });
        }

        cursor_y += band_height + config.band_gap;
    }

    let height = config.height.max(
        cursor_y
            - if bands.is_empty() {
                0.0
            } else {
                config.band_gap
            },
    );

    let mut overlap_area = 0.0;
    for (index, node) in nodes.iter().enumerate() {
        for other in nodes.iter().skip(index + 1) {
            if other.layer == node.layer {
                overlap_area += node.rect.intersection_area(other.rect);
            }
        }
    }

    let node_by_id = nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<BTreeMap<_, _>>();
    let edges = artifact
        .edges
        .iter()
        .filter_map(|edge| {
            let source = node_by_id.get(edge.source.as_str())?;
            let target = node_by_id.get(edge.target.as_str())?;
            Some(EdgeLayout {
                source: edge.source.clone(),
                target: edge.target.clone(),
                direction_adjusted: edge.direction_adjusted,
                from: source.rect.center(),
                to: target.rect.center(),
                z_index: 1,
            })
        })
        .collect::<Vec<_>>();

    MapLayout {
        width: config.width,
        height,
        bands,
        nodes,
        edges,
        overlap_area,
        overlap_area_threshold: config.overlap_area_threshold,
    }
}
fn dir_prefix(id: &str) -> &str {
    match id.rsplit_once('/') {
        Some((dir, _)) => dir,
        None => "",
    }
}

fn normalized_positions(layer_ids: &[String]) -> BTreeMap<String, f32> {
    let denom = layer_ids.len().saturating_sub(1).max(1) as f32;
    layer_ids
        .iter()
        .enumerate()
        .map(|(idx, id)| (id.clone(), idx as f32 / denom))
        .collect()
}

fn reorder_layer_by_barycenter(
    layer_ids: &mut [String],
    ref_positions: &BTreeMap<String, f32>,
    adj: &BTreeMap<String, Vec<String>>,
) {
    let current = normalized_positions(layer_ids);
    let mut scored = layer_ids
        .iter()
        .map(|id| {
            let mut sum = 0.0_f32;
            let mut count = 0usize;
            if let Some(neighbors) = adj.get(id) {
                for neighbor in neighbors {
                    if let Some(&pos) = ref_positions.get(neighbor) {
                        sum += pos;
                        count += 1;
                    }
                }
            }
            let score = if count > 0 {
                sum / count as f32
            } else {
                current.get(id).copied().unwrap_or(0.5)
            };
            (id.clone(), score)
        })
        .collect::<Vec<_>>();

    scored.sort_by(|(id_a, score_a), (id_b, score_b)| {
        score_a
            .total_cmp(score_b)
            .then_with(|| dir_prefix(id_a).cmp(dir_prefix(id_b)))
            .then_with(|| id_a.cmp(id_b))
    });

    for (slot, (id, _)) in layer_ids.iter_mut().zip(scored) {
        *slot = id;
    }
}

/// 按目录前缀聚类并执行 2 轮跨层重心扫描，减少相邻层连线交叉（完全确定性）。
fn order_layers_for_readability(
    layers: &mut [(&'static str, Vec<String>)],
    edges: &[(String, String)],
) {
    for (_, ids) in layers.iter_mut() {
        ids.sort_by(|a, b| dir_prefix(a).cmp(dir_prefix(b)).then_with(|| a.cmp(b)));
    }
    if layers.len() < 2 || edges.is_empty() {
        return;
    }

    let mut adj: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (src, dst) in edges {
        if src == dst {
            continue;
        }
        adj.entry(src.clone()).or_default().push(dst.clone());
        adj.entry(dst.clone()).or_default().push(src.clone());
    }

    for _ in 0..2 {
        for idx in 1..layers.len() {
            let ref_pos = normalized_positions(&layers[idx - 1].1);
            reorder_layer_by_barycenter(&mut layers[idx].1, &ref_pos, &adj);
        }
        for idx in (0..layers.len() - 1).rev() {
            let ref_pos = normalized_positions(&layers[idx + 1].1);
            reorder_layer_by_barycenter(&mut layers[idx].1, &ref_pos, &adj);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count_two_layer_crossings(
        top: &[String],
        bottom: &[String],
        edges: &[(String, String)],
    ) -> usize {
        let top_idx = top
            .iter()
            .enumerate()
            .map(|(i, id)| (id.as_str(), i))
            .collect::<BTreeMap<_, _>>();
        let bottom_idx = bottom
            .iter()
            .enumerate()
            .map(|(i, id)| (id.as_str(), i))
            .collect::<BTreeMap<_, _>>();
        let mapped = edges
            .iter()
            .filter_map(|(s, t)| {
                let a = *top_idx.get(s.as_str())?;
                let b = *bottom_idx.get(t.as_str())?;
                Some((a, b))
            })
            .collect::<Vec<_>>();
        let mut crossings = 0;
        for (i, &(a1, b1)) in mapped.iter().enumerate() {
            for &(a2, b2) in mapped.iter().skip(i + 1) {
                if (a1 < a2 && b1 > b2) || (a1 > a2 && b1 < b2) {
                    crossings += 1;
                }
            }
        }
        crossings
    }

    #[test]
    fn barycenter_ordering_reduces_cross_layer_crossings() {
        let mut layers = vec![
            ("ui", vec!["src/ui/a.ts".to_string(), "src/ui/b.ts".to_string()]),
            (
                "domain",
                vec!["src/domain/x.ts".to_string(), "src/domain/y.ts".to_string()],
            ),
        ];
        // 初始字典序存在 1 处交叉：a -> y, b -> x
        let edges = vec![
            ("src/ui/a.ts".to_string(), "src/domain/y.ts".to_string()),
            ("src/ui/b.ts".to_string(), "src/domain/x.ts".to_string()),
        ];
        let before = count_two_layer_crossings(&layers[0].1, &layers[1].1, &edges);
        assert_eq!(before, 1);

        order_layers_for_readability(&mut layers, &edges);
        let after = count_two_layer_crossings(&layers[0].1, &layers[1].1, &edges);
        assert_eq!(after, 0);
    }

    #[test]
    fn directory_clustering_groups_same_folder_modules() {
        let mut layers = vec![(
            "domain",
            vec![
                "src/z/one.ts".to_string(),
                "src/a/beta.ts".to_string(),
                "src/z/two.ts".to_string(),
                "src/a/alpha.ts".to_string(),
            ],
        )];
        order_layers_for_readability(&mut layers, &[]);
        assert_eq!(
            layers[0].1,
            vec![
                "src/a/alpha.ts".to_string(),
                "src/a/beta.ts".to_string(),
                "src/z/one.ts".to_string(),
                "src/z/two.ts".to_string(),
            ]
        );
    }
}
