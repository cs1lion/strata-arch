//! A（边批处理）的几何等价判据（票 10，与像素断言分开）：
//!
//! base 与 A 必须产生完全相同的节点中心/尺寸、标签文本与锚点、边端点序列；
//! 允许的差异只存在于栅格化（绘制顺序、path 子路径合并）层面。
//!
//! 这里不读截图、不看像素；任何几何被批处理改变都会单独失败。

use std::path::{Path, PathBuf};

use arch_core::layers::build_layers;
use arch_view::layout::{layout_map, LayoutConfig};
use arch_view::view::{frame_geometry, group_edges, EdgeVisual, FrameGeometry, Viewport};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn fixture_dir() -> PathBuf {
    workspace_root().join("tests/fixtures/callgraph")
}

fn build() -> arch_view::layout::MapLayout {
    let layers = build_layers(&fixture_dir(), "0.1.0").expect("layers artifact");
    layout_map(&layers, LayoutConfig::default())
}

fn cameras() -> Vec<Viewport> {
    vec![
        Viewport {
            pan_x: 0.0,
            pan_y: 0.0,
            zoom: 1.0,
        },
        Viewport {
            pan_x: -240.0,
            pan_y: -160.0,
            zoom: 0.4,
        },
        Viewport {
            pan_x: 37.0,
            pan_y: -53.0,
            zoom: 2.0,
        },
    ]
}

fn assert_same_geometry(base: &FrameGeometry, batched: &FrameGeometry) {
    assert_eq!(
        base.nodes.len(),
        batched.nodes.len(),
        "base 与 A 的节点数量必须一致"
    );
    for (index, (left, right)) in base.nodes.iter().zip(batched.nodes.iter()).enumerate() {
        assert_eq!(
            left.label, right.label,
            "第 {index} 个节点的标签文本被批处理改变"
        );
        assert_eq!(
            left.center, right.center,
            "第 {index} 个节点（{}）的中心被批处理改变",
            left.label
        );
        assert_eq!(
            left.size, right.size,
            "第 {index} 个节点（{}）的尺寸被批处理改变",
            left.label
        );
        assert_eq!(
            left.rect, right.rect,
            "第 {index} 个节点（{}）的屏幕矩形被批处理改变",
            left.label
        );
        assert_eq!(
            left.label_anchor, right.label_anchor,
            "第 {index} 个节点（{}）的标签位置被批处理改变",
            left.label
        );
    }

    assert_eq!(
        base.edges.len(),
        batched.edges.len(),
        "base 与 A 的边数量必须一致（批处理不得丢边或增边）"
    );
    for (index, (left, right)) in base.edges.iter().zip(batched.edges.iter()).enumerate() {
        assert_eq!(left.from, right.from, "第 {index} 条边的起点被批处理改变");
        assert_eq!(left.to, right.to, "第 {index} 条边的终点被批处理改变");
        assert_eq!(
            left.adjusted, right.adjusted,
            "第 {index} 条边的方向样式被批处理改变"
        );
        assert_eq!(
            left.strong, right.strong,
            "第 {index} 条边的强调状态被批处理改变"
        );
        assert_eq!(
            left.opacity, right.opacity,
            "第 {index} 条边的透明度被批处理改变"
        );
    }
}

#[test]
fn base_and_a_have_identical_geometry_at_three_cameras() {
    let map = build();
    for viewport in cameras() {
        let base = frame_geometry(&map, &viewport, false);
        let batched = frame_geometry(&map, &viewport, true);
        assert_same_geometry(&base, &batched);
    }
}

#[test]
fn batching_actually_groups_edges_but_flattens_back() {
    let map = build();
    let viewport = Viewport::default();
    let edges = map
        .edges
        .iter()
        .map(|edge| EdgeVisual {
            from: viewport.screen_point(edge.from),
            to: viewport.screen_point(edge.to),
            adjusted: edge.direction_adjusted,
            strong: false,
            opacity: 1.0,
        })
        .collect::<Vec<_>>();
    let per_edge = group_edges(&edges, false);
    let batched = group_edges(&edges, true);
    assert_eq!(per_edge.len(), edges.len(), "关闭 A 时每条边必须自成一组");
    assert!(
        batched.len() < edges.len(),
        "开启 A 后必须发生合并，否则该样本无法证明批处理生效"
    );
    let flatten = |runs: &[arch_view::view::EdgeRun]| {
        runs.iter()
            .flat_map(|run| run.edges.iter().copied())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        flatten(&per_edge),
        (0..edges.len()).collect::<Vec<_>>(),
        "逐边分组的展开顺序必须与输入一致"
    );
    assert_eq!(
        flatten(&batched),
        (0..edges.len()).collect::<Vec<_>>(),
        "批处理分组的展开顺序必须与输入一致"
    );
}
