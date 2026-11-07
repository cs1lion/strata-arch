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
            band_gap: 16.0,
            band_header: 24.0,
            band_padding: 10.0,
            node_width: 150.0,
            node_height: 40.0,
            node_gap_x: 12.0,
            node_gap_y: 10.0,
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

    for layer in LAYER_ORDER {
        let layer_modules = artifact
            .modules
            .iter()
            .filter(|module| module.layer == layer)
            .collect::<Vec<_>>();
        if layer_modules.is_empty() {
            continue;
        }

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

        for (index, module) in layer_modules.iter().enumerate() {
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
                id: module.id.clone(),
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
