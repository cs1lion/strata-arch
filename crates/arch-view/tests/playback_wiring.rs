//! B 档（断言先行）：两处「代码写了但没接上」的结构断言。
//!
//! 1. 符号场景的当前路径边必须被强调（`strong`），且只有当前边被强调。
//! 2. fly-to 打开后，视口中心的世界坐标必须移动到当前步骤节点中心。
//!
//! 反向用例：fly-to 关闭时不产生目标、也不移动视口；单步路径没有当前边，不得误报强调。
//!
//! 断言在修复前应给出非零失败读数（0 条强调边、视口位移 0px）。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use arch_core::callgraph::{build_call_graph, CallGraphArtifact};
use arch_core::guided_path::{build_path, GuidedPathArtifact};
use arch_core::layers::build_layers;
use arch_view::layout::{layout_map, LayoutConfig, Point, Rect};
use arch_view::playback::PlaybackState;
use arch_view::view::{paint_symbol_edges, CanvasState, SYMBOL_CANVAS_HEIGHT, SYMBOL_CANVAS_WIDTH};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .join("tests/fixtures/callgraph")
}

fn callgraph() -> CallGraphArtifact {
    build_call_graph(&fixture(), "test").expect("callgraph fixture")
}

fn beta_to_alpha() -> GuidedPathArtifact {
    build_path(
        &fixture(),
        "test",
        "src/a.ts#function:beta",
        "src/a.ts#function:alpha",
    )
    .expect("fixture path beta -> alpha")
}

fn drilled_state() -> (CanvasState, GuidedPathArtifact) {
    let layers = build_layers(&fixture(), "test").expect("layers fixture");
    let map = layout_map(&layers, LayoutConfig::default());
    let mut state = CanvasState::new(map);
    let graph = callgraph();
    assert!(
        state.drill_down("src/a.ts", &graph),
        "fixture must drill into src/a.ts"
    );
    (state, beta_to_alpha())
}

fn path_pairs(path: &GuidedPathArtifact) -> BTreeSet<(String, String)> {
    path.steps
        .windows(2)
        .map(|pair| (pair[0].symbol.clone(), pair[1].symbol.clone()))
        .collect()
}

#[test]
fn current_path_edge_is_emphasized_in_symbol_scene() {
    let (state, path) = drilled_state();
    let layout = state.symbol_layout().expect("drilled symbol layout");
    let pairs = path_pairs(&path);
    let mut playback = PlaybackState::new(&path);

    playback.play();
    assert!(playback.advance(), "enter step 1");

    let painted = paint_symbol_edges(layout, &state.viewport(), playback.current_edge().as_ref());
    let total = painted.len();
    let present = painted
        .iter()
        .filter(|edge| pairs.contains(&(edge.source.clone(), edge.target.clone())))
        .count();
    let strong = painted.iter().filter(|edge| edge.strong).count();
    assert!(
        present >= 1,
        "fixture must draw the path edge inside src/a.ts; drew {present}/{total}"
    );
    assert_eq!(
        strong, 0,
        "step 1 has no incoming edge, but strong {strong}/{total} (path edges present {present})"
    );

    assert!(playback.advance(), "enter step 2");
    let painted = paint_symbol_edges(layout, &state.viewport(), playback.current_edge().as_ref());
    let total = painted.len();
    let strong = painted.iter().filter(|edge| edge.strong).count();
    let strong_edges = painted
        .iter()
        .filter(|edge| edge.strong)
        .map(|edge| (edge.source.as_str(), edge.target.as_str()))
        .collect::<Vec<_>>();

    assert_eq!(
        strong, 1,
        "current edge must be the only strong edge: strong {strong}/{total}, path edges present {present}"
    );
    assert_eq!(
        strong_edges,
        vec![("src/a.ts#function:beta", "src/a.ts#function:alpha")],
        "strong edge must be exactly the current one: strong {strong}/{total}"
    );
    eprintln!("symbol scene: strong {strong}/{total}, path edges drawn {present}");
}

#[test]
fn fly_to_moves_viewport_center_to_current_node() {
    let (mut state, path) = drilled_state();
    let mut playback = PlaybackState::new(&path);
    playback.play();
    assert!(playback.advance(), "enter step 1");
    assert!(playback.advance(), "enter step 2");

    let rects = state
        .symbol_layout()
        .expect("drilled symbol layout")
        .nodes
        .iter()
        .map(|node| (node.id.clone(), node.rect))
        .collect::<BTreeMap<String, Rect>>();
    playback.set_fly_to(true);
    let target = playback
        .camera_center(&rects)
        .expect("current step must have a node rect");

    let size = (SYMBOL_CANVAS_WIDTH, SYMBOL_CANVAS_HEIGHT);
    let screen_center = Point {
        x: size.0 / 2.0,
        y: size.1 / 2.0,
    };
    let before = state.viewport().world_point(screen_center);
    let moved = state.apply_fly_to(Some(target), size);
    let after = state.viewport().world_point(screen_center);
    let delta = ((after.x - before.x).powi(2) + (after.y - before.y).powi(2)).sqrt();

    assert!(
        moved,
        "fly-to must move the viewport: delta {delta}px, before {before:?}, after {after:?}, target {target:?}"
    );
    assert!(
        (after.x - target.x).abs() < 0.01 && (after.y - target.y).abs() < 0.01,
        "viewport center must land on the current node: delta {delta}px, before {before:?}, after {after:?}, target {target:?}"
    );
    assert!(
        delta > 10.0,
        "viewport center movement must be measurable: delta {delta}px"
    );
    eprintln!("fly-to: delta {delta}px, before {before:?}, after {after:?}, target {target:?}");
}

#[test]
fn fly_to_off_and_single_step_path_do_not_trigger_wiring() {
    let (mut state, path) = drilled_state();
    let mut playback = PlaybackState::new(&path);
    playback.play();
    assert!(playback.advance());
    assert!(playback.advance());

    let rects = state
        .symbol_layout()
        .expect("drilled symbol layout")
        .nodes
        .iter()
        .map(|node| (node.id.clone(), node.rect))
        .collect::<BTreeMap<String, Rect>>();
    let target = playback.camera_center(&rects);
    assert!(
        target.is_none(),
        "fly-to off must not produce a camera target"
    );
    let before = state.viewport();
    let moved = state.apply_fly_to(target, (SYMBOL_CANVAS_WIDTH, SYMBOL_CANVAS_HEIGHT));
    assert!(!moved, "fly-to off must not move the viewport");
    assert_eq!(state.viewport(), before, "viewport must stay unchanged");

    let single = build_path(
        &fixture(),
        "test",
        "src/a.ts#function:alpha",
        "src/a.ts#function:alpha",
    )
    .expect("single step path");
    assert_eq!(single.steps.len(), 1, "fixture must stay a one-step path");
    let mut one = PlaybackState::new(&single);
    one.play();
    assert!(one.advance());
    let painted = paint_symbol_edges(
        state.symbol_layout().expect("drilled symbol layout"),
        &state.viewport(),
        one.current_edge().as_ref(),
    );
    let total = painted.len();
    let strong = painted.iter().filter(|edge| edge.strong).count();
    assert_eq!(
        strong, 0,
        "single-step path has no current edge, but strong {strong}/{total}"
    );
}
