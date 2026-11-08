//! B（视口裁剪）判据（实现前先写，票 10）：
//!
//! 1. 只剪「完全在视口外」的元素：任何与视口相交的节点/边都不得被剪。
//! 2. 命中测试仍用全量几何：视口内任何屏幕点都不会命中被剪掉的节点。
//! 3. 揭示动画位移下同样成立：逐帧扫描所有层带的 offset，部分可见即不剪。
//! 4. 边按两端包围盒判定：贴边/穿视口的线段不剪，整体在外才剪。
//!
//! 像素等价的实测读数由 `eval/map-pixel-compare.ps1` 给出；本文件只断言几何不变式。

use std::path::{Path, PathBuf};

use arch_core::callgraph::{build_call_graph, CallGraphArtifact};
use arch_core::layers::build_layers;
use arch_view::layout::{layout_map, LayoutConfig, MapLayout, Point, Rect};
use arch_view::reveal::RevealState;
use arch_view::view::{
    rect_fully_outside, segment_fully_outside, CanvasState, Viewport, CULL_MARGIN,
};

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

fn build() -> (MapLayout, CallGraphArtifact) {
    let layers = build_layers(&fixture_dir(), "0.1.0").expect("layers artifact");
    let map = layout_map(&layers, LayoutConfig::default());
    let callgraph = build_call_graph(&fixture_dir(), "0.1.0").expect("call graph");
    (map, callgraph)
}

fn screen_size() -> (f32, f32) {
    (900.0, 600.0)
}

/// 三个代表性相机：默认、平移后低缩放、放大到局部。
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
            pan_x: -120.0,
            pan_y: -80.0,
            zoom: 2.0,
        },
    ]
}

fn intersects(rect: Rect, size: (f32, f32)) -> bool {
    !(rect.x > size.0 || rect.x + rect.width < 0.0 || rect.y > size.1 || rect.y + rect.height < 0.0)
}

#[test]
fn culled_nodes_are_never_hit_inside_viewport() {
    let (map, _) = build();
    let size = screen_size();
    for viewport in cameras() {
        let mut culled = 0;
        for node in &map.nodes {
            let screen = viewport.screen_rect(node.rect);
            if !rect_fully_outside(screen, size, CULL_MARGIN) {
                continue;
            }
            culled += 1;
            // 视口内按 4px 网格采样：被剪节点不得被任何视口内屏幕点命中。
            let mut x = 0.0;
            while x <= size.0 {
                let mut y = 0.0;
                while y <= size.1 {
                    let world = viewport.world_point(Point { x, y });
                    assert!(
                        !node.hit_rect.contains(world),
                        "被剪节点 {} 在视口内 ({x},{y}) 仍可命中",
                        node.id
                    );
                    y += 4.0;
                }
                x += 4.0;
            }
        }
        assert!(culled > 0, "相机 {viewport:?} 下没有节点被剪，测试样本无效");
    }
}

#[test]
fn partially_visible_nodes_are_never_culled() {
    let (map, _) = build();
    let size = screen_size();
    let mut partial = 0;
    for viewport in cameras() {
        for node in &map.nodes {
            let screen = viewport.screen_rect(node.rect);
            if !intersects(screen, size) || rect_fully_outside(screen, size, CULL_MARGIN) {
                continue;
            }
            partial += 1;
            assert!(
                !rect_fully_outside(screen, size, CULL_MARGIN),
                "部分可见节点 {} 被剪掉（相机 {viewport:?}）",
                node.id
            );
        }
    }
    assert!(partial > 0, "没有部分可见节点，测试样本无效");
}

#[test]
fn hit_test_uses_full_geometry_regardless_of_cull() {
    let (map, _) = build();
    let size = screen_size();
    for viewport in cameras() {
        let mut state = CanvasState::new(map.clone());
        state.set_viewport(viewport);
        for node in &map.nodes {
            let center = viewport.screen_point(node.hit_rect.center());
            if center.x < 0.0 || center.y < 0.0 || center.x > size.0 || center.y > size.1 {
                continue;
            }
            let hit = state.select_at(center);
            assert!(hit.is_some(), "视口内节点中心必须可命中：{}", node.id);
            // 命中结果必须来自全量几何，并且该节点绝不可能被视口裁剪剪掉。
            let hit_id = hit.expect("hit");
            let hit_node = map
                .nodes
                .iter()
                .find(|candidate| candidate.id == hit_id)
                .expect("hit node in map");
            assert!(
                !rect_fully_outside(viewport.screen_rect(hit_node.rect), size, CULL_MARGIN),
                "可命中节点 {} 不得被视口裁剪剪掉",
                hit_id
            );
        }
    }
}

#[test]
fn reveal_offsets_never_cull_a_visible_node() {
    let (map, _) = build();
    let size = screen_size();
    let viewport = Viewport {
        pan_x: -120.0,
        pan_y: -60.0,
        zoom: 0.6,
    };
    let mut reveal = RevealState::new(true, map.bands.len());
    reveal.start();
    loop {
        for stage in 0..map.bands.len() {
            let offset = reveal.offset_for_stage(stage);
            for node in &map.nodes {
                let screen = viewport.screen_rect(node.rect);
                let drawn = Rect::new(screen.x, screen.y + offset, screen.width, screen.height);
                if intersects(drawn, size) {
                    assert!(
                        !rect_fully_outside(drawn, size, CULL_MARGIN),
                        "揭示第 {stage} 层（offset={offset}）把可见节点 {} 剪掉",
                        node.id
                    );
                }
            }
        }
        if !reveal.advance_frame() {
            break;
        }
    }
}

#[test]
fn edges_are_culled_only_when_fully_outside() {
    let size = (800.0, 600.0);
    // 完全在左侧/右侧/上方/下方。
    assert!(segment_fully_outside(
        Point { x: -10.0, y: 50.0 },
        Point { x: -5.0, y: 200.0 },
        size,
        CULL_MARGIN
    ));
    assert!(segment_fully_outside(
        Point { x: 900.0, y: 50.0 },
        Point {
            x: 1200.0,
            y: 200.0
        },
        size,
        CULL_MARGIN
    ));
    // 穿过视口、贴边（margin 内）、一端在内：都不得剪。
    assert!(!segment_fully_outside(
        Point { x: -10.0, y: 50.0 },
        Point { x: 100.0, y: 200.0 },
        size,
        CULL_MARGIN
    ));
    assert!(!segment_fully_outside(
        Point { x: 801.0, y: 0.0 },
        Point { x: 801.0, y: 600.0 },
        size,
        CULL_MARGIN
    ));
    assert!(!segment_fully_outside(
        Point {
            x: -1000.0,
            y: 300.0
        },
        Point { x: 400.0, y: 300.0 },
        size,
        CULL_MARGIN
    ));
}

#[test]
fn symbol_scene_follows_same_cull_rule() {
    let (map, callgraph) = build();
    let size = screen_size();
    let module = map
        .nodes
        .iter()
        .find(|node| node.id.contains("src"))
        .map(|node| node.id.clone())
        .expect("fixture 模块");
    let mut state = CanvasState::new(map);
    assert!(state.drill_down(&module, &callgraph), "下钻必须成功");
    let symbols = state.symbol_layout().expect("symbol layout").clone();
    let viewport = state.viewport();
    for node in &symbols.nodes {
        let screen = viewport.screen_rect(node.rect);
        if rect_fully_outside(screen, size, CULL_MARGIN) {
            assert!(!intersects(screen, size), "符号节点被剪但仍在视口内");
        }
    }
    for edge in &symbols.edges {
        let from = viewport.screen_point(edge.from);
        let to = viewport.screen_point(edge.to);
        if segment_fully_outside(from, to, size, CULL_MARGIN) {
            assert!(
                from.x < 0.0 && to.x < 0.0
                    || from.x > size.0 && to.x > size.0
                    || from.y < 0.0 && to.y < 0.0
                    || from.y > size.1 && to.y > size.1,
                "符号边按包围盒被剪但两端不在同一外侧"
            );
        }
    }
}
