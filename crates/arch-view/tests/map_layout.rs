//! ticket 03 B 档结构断言清单（实现前先写）：
//!
//! 1. 节点矩形：节点数等于 JSON 模块数，ID 集合一致，矩形为正且落在画布内。
//! 2. 边端点：边数等于 JSON 边数，`from`/`to` 分别等于源/目标节点矩形中心，方向标记保留。
//! 3. 标签包围盒：每个节点都有正面积的标签盒，且完全落在节点矩形内。
//! 4. z-order：层带 0、边 1、节点 ≥1000 且严格递增；节点永远压在边和层带之上。
//! 5. 命中框：命中框与节点矩形一致，中心点命中。
//! 6. 层带顺序：层带按固定 `LAYER_ORDER` 自上而下；`external` 与 `unclassified` 各自独立层带，`unclassified` 最后。
//! 7. 同层重叠：同层节点包围盒重叠面积总和不超过基线阈值（默认 0）。
//! 8. 确定性：同一输入两次布局的序列化结果字节一致。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use arch_core::layers::{build_layers, LayersArtifact, LAYER_ORDER};
use arch_view::layout::{layout_map, LayoutConfig, MapLayout};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn fixture_dir() -> PathBuf {
    workspace_root().join("tests/fixtures/layers")
}

fn build() -> (LayersArtifact, MapLayout) {
    let artifact = build_layers(&fixture_dir(), "0.1.0").expect("layers artifact");
    let layout = layout_map(&artifact, LayoutConfig::default());
    (artifact, layout)
}

#[test]
fn node_rectangles_match_json_modules_and_stay_inside_canvas() {
    let (artifact, layout) = build();

    assert_eq!(
        layout.nodes.len(),
        artifact.modules.len(),
        "node count must equal JSON module count"
    );

    let module_ids = artifact
        .modules
        .iter()
        .map(|module| module.id.as_str())
        .collect::<BTreeSet<_>>();
    let node_ids = layout
        .nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(module_ids, node_ids);

    for node in &layout.nodes {
        assert!(
            node.rect.width > 0.0 && node.rect.height > 0.0,
            "node {} must have a positive rect",
            node.id
        );
        assert!(
            node.rect.x >= 0.0
                && node.rect.y >= 0.0
                && node.rect.x + node.rect.width <= layout.width
                && node.rect.y + node.rect.height <= layout.height,
            "node {} must stay inside the canvas",
            node.id
        );
    }
}

#[test]
fn bands_follow_fixed_layer_order_top_to_bottom() {
    let (artifact, layout) = build();

    let expected = LAYER_ORDER
        .iter()
        .filter(|layer| {
            artifact
                .modules
                .iter()
                .any(|module| module.layer == **layer)
        })
        .copied()
        .collect::<Vec<_>>();
    let actual = layout
        .bands
        .iter()
        .map(|band| band.layer)
        .collect::<Vec<_>>();
    assert_eq!(actual, expected, "bands must follow LAYER_ORDER");

    let mut previous_bottom = f32::NEG_INFINITY;
    for band in &layout.bands {
        assert!(
            band.rect.y >= previous_bottom,
            "band {} must start below the previous band",
            band.layer
        );
        previous_bottom = band.rect.y + band.rect.height;
    }
}

#[test]
fn unclassified_has_its_own_last_band_and_distinct_visual_role() {
    let (artifact, layout) = build();

    let unclassified_band = layout
        .bands
        .iter()
        .find(|band| band.is_unclassified)
        .expect("unclassified band must exist");
    assert_eq!(unclassified_band.layer, "unclassified");
    assert_eq!(
        layout.bands.last().map(|band| band.layer),
        Some("unclassified"),
        "unclassified band must be last"
    );
    assert_eq!(
        unclassified_band.node_count,
        artifact.unclassified.len(),
        "unclassified band node count must equal the JSON unclassified list"
    );

    let unclassified_ids = artifact
        .unclassified
        .iter()
        .map(|module| module.id.as_str())
        .collect::<BTreeSet<_>>();
    let mut seen_unclassified = BTreeSet::new();
    for node in &layout.nodes {
        if node.layer == "unclassified" {
            assert!(
                node.is_unclassified && node.visual_role == "unclassified",
                "unclassified node {} must carry a distinct visual role",
                node.id
            );
            assert!(!node.is_external);
            assert!(unclassified_ids.contains(node.id.as_str()));
            seen_unclassified.insert(node.id.as_str());
        } else if node.layer == "external" {
            assert!(
                node.is_external && node.visual_role == "external",
                "external node {} must carry its own visual role",
                node.id
            );
            assert!(!node.is_unclassified);
            assert!(!unclassified_ids.contains(node.id.as_str()));
        } else {
            assert!(!node.is_unclassified && node.visual_role == "layer");
            assert!(!node.is_external);
            assert!(!unclassified_ids.contains(node.id.as_str()));
        }
    }
    assert_eq!(seen_unclassified, unclassified_ids);
}

#[test]
fn external_has_its_own_band_before_unclassified() {
    let (artifact, layout) = build();

    let external_band = layout
        .bands
        .iter()
        .find(|band| band.layer == "external")
        .expect("external band must exist");
    assert!(external_band.is_external);
    assert!(!external_band.is_unclassified);
    assert_eq!(
        external_band.node_count,
        artifact
            .modules
            .iter()
            .filter(|module| module.layer == "external")
            .count(),
        "external band node count must match external modules"
    );

    let unclassified_band = layout
        .bands
        .iter()
        .find(|band| band.layer == "unclassified")
        .expect("unclassified band must exist");
    assert!(
        external_band.rect.y + external_band.rect.height <= unclassified_band.rect.y,
        "external band must sit above the unclassified band"
    );
}

#[test]
fn same_layer_node_overlap_stays_below_threshold() {
    let (_, layout) = build();

    let mut totals: BTreeMap<&str, f32> = BTreeMap::new();
    for (index, node) in layout.nodes.iter().enumerate() {
        for other in layout.nodes.iter().skip(index + 1) {
            if other.layer == node.layer {
                *totals.entry(node.layer).or_insert(0.0) += node.rect.intersection_area(other.rect);
            }
        }
    }
    let measured = totals.values().sum::<f32>();
    assert!(
        (measured - layout.overlap_area).abs() <= 0.01,
        "reported overlap must match measured overlap: {measured} vs {}",
        layout.overlap_area
    );
    assert!(
        measured <= layout.overlap_area_threshold + 0.01,
        "same-layer overlap {measured} exceeds threshold {}",
        layout.overlap_area_threshold
    );
}

#[test]
fn edge_endpoints_are_source_and_target_node_centers() {
    let (artifact, layout) = build();

    assert_eq!(
        layout.edges.len(),
        artifact.edges.len(),
        "layout must retain every JSON edge"
    );

    let nodes = layout
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<BTreeMap<_, _>>();
    let expected_edges = artifact
        .edges
        .iter()
        .map(|edge| {
            (
                edge.source.as_str(),
                edge.target.as_str(),
                edge.direction_adjusted,
            )
        })
        .collect::<BTreeSet<_>>();

    for edge in &layout.edges {
        let source = nodes
            .get(edge.source.as_str())
            .unwrap_or_else(|| panic!("missing source node {}", edge.source));
        let target = nodes
            .get(edge.target.as_str())
            .unwrap_or_else(|| panic!("missing target node {}", edge.target));
        assert_eq!(
            edge.from,
            source.rect.center(),
            "edge from must be the source node center"
        );
        assert_eq!(
            edge.to,
            target.rect.center(),
            "edge to must be the target node center"
        );
    }

    let actual_edges = layout
        .edges
        .iter()
        .map(|edge| {
            (
                edge.source.as_str(),
                edge.target.as_str(),
                edge.direction_adjusted,
            )
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(actual_edges, expected_edges);
}

#[test]
fn label_boxes_and_hit_boxes_are_contained() {
    let (_, layout) = build();

    for node in &layout.nodes {
        assert!(
            node.label_rect.width > 0.0 && node.label_rect.height > 0.0,
            "node {} must have a positive label box",
            node.id
        );
        assert!(
            node.label_rect.x >= node.rect.x
                && node.label_rect.y >= node.rect.y
                && node.label_rect.x + node.label_rect.width
                    <= node.rect.x + node.rect.width + 0.01
                && node.label_rect.y + node.label_rect.height
                    <= node.rect.y + node.rect.height + 0.01,
            "node {} label box must stay inside its node rect",
            node.id
        );
        assert_eq!(
            node.hit_rect, node.rect,
            "node {} hit box must match its rect",
            node.id
        );
        assert!(
            node.hit_rect.contains(node.rect.center()),
            "node {} center must be hittable",
            node.id
        );
    }
}

#[test]
fn z_order_puts_nodes_above_edges_and_bands() {
    let (_, layout) = build();

    assert!(
        layout.bands.iter().all(|band| band.z_index == 0),
        "bands must be the bottom layer"
    );
    assert!(
        layout.edges.iter().all(|edge| edge.z_index == 1),
        "edges must sit above bands"
    );

    let mut previous = 0;
    for node in &layout.nodes {
        assert!(node.z_index >= 1000, "nodes must sit above edges and bands");
        assert!(
            node.z_index > previous,
            "node z-order must strictly increase in layout order"
        );
        previous = node.z_index;
    }
}

#[test]
fn layout_is_deterministic() {
    let (artifact, first) = build();
    let second = layout_map(&artifact, LayoutConfig::default());
    assert_eq!(
        serde_json::to_string(&first).expect("serialize first"),
        serde_json::to_string(&second).expect("serialize second"),
        "layout must be byte-identical for the same input"
    );
}
