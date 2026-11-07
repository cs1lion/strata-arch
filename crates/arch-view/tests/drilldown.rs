//! ticket 05 B 档结构断言清单（实现前先写）：
//!
//! 1. 下钻后符号视图的节点数等于该模块在调用图 JSON 中的符号数，ID 集合一致。
//! 2. 符号视图内只保留两端都在本模块的调用边；跨模块边不出现在画布，但邻居高亮仍包含它们。
//! 3. 选中某个符号后，高亮邻居集合（入边 + 出边、去掉自身）等于从调用图边集合独立计算的结果。
//! 4. 下钻前的平移与缩放被保存；返回上一层后逐值恢复（`pan` 与 `zoom`）。
//! 5. 命中测试：节点中心坐标选中该节点；空白坐标清空选择；视图变换后按屏幕坐标命中同一节点。
//! 6. 平移与缩放状态可读；缩放钳制在 `[MIN_ZOOM, MAX_ZOOM]`；缩放锚点下的世界坐标保持不动。
//! 7. LOD：缩放低于 `LABEL_ZOOM_THRESHOLD` 时标签隐藏，恢复阈值以上后重新可见。
//! 8. 合成 5k 模块 fixture：启动（布局 + 状态初始化）、下钻、返回各 ≤2s，3 次运行记录中位数与 p95。
//!
//! 像素门禁默认关闭；本票不依赖离屏截图，结构变化全部在布局/状态模型上断言。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

use arch_core::callgraph::{
    build_call_graph, CallEdge, CallGraphArtifact, CallGraphStats, SymbolNode,
    CALLGRAPH_SCHEMA_VERSION,
};
use arch_core::layers::{build_layers, LayeredEdge, LayeredModule, LayersArtifact, LAYER_ORDER};
use arch_core::{RepoInfo, Stats};
use arch_view::layout::{layout_map, LayoutConfig, MapLayout, Point};
use arch_view::view::{CanvasState, Level, LABEL_ZOOM_THRESHOLD, MAX_ZOOM, MIN_ZOOM};

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

fn build() -> (LayersArtifact, MapLayout, CallGraphArtifact) {
    let layers = build_layers(&fixture_dir(), "0.1.0").expect("layers artifact");
    let map = layout_map(&layers, LayoutConfig::default());
    let callgraph = build_call_graph(&fixture_dir(), "0.1.0").expect("call graph");
    (layers, map, callgraph)
}

fn screen_of(state: &CanvasState, world: Point) -> Point {
    state.viewport().screen_point(world)
}

#[test]
fn drilldown_symbol_count_matches_module_symbols() {
    let (_, map, callgraph) = build();
    let mut state = CanvasState::new(map);

    let module = "src/a.ts";
    assert!(
        state.drill_down(module, &callgraph),
        "drilldown must succeed"
    );

    let expected = callgraph
        .symbols
        .iter()
        .filter(|symbol| symbol.module == module)
        .map(|symbol| symbol.id.as_str())
        .collect::<BTreeSet<_>>();
    let actual = state
        .symbol_layout()
        .expect("symbol layout")
        .nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<BTreeSet<_>>();

    assert_eq!(
        actual, expected,
        "symbol view must contain exactly the module's JSON symbols"
    );
    assert_eq!(state.symbol_layout().expect("symbol layout").nodes.len(), 5);
    assert!(matches!(
        state.level(),
        Level::Symbols { module: drilled } if drilled == module
    ));
}

#[test]
fn symbol_edges_stay_inside_module_and_highlight_matches_callgraph() {
    let (_, map, callgraph) = build();
    let mut state = CanvasState::new(map);
    assert!(state.drill_down("src/a.ts", &callgraph));

    let layout = state.symbol_layout().expect("symbol layout");
    let ids = layout
        .nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<BTreeSet<_>>();

    for edge in &layout.edges {
        assert!(
            ids.contains(edge.source.as_str()) && ids.contains(edge.target.as_str()),
            "symbol edge {} -> {} must stay inside the module",
            edge.source,
            edge.target
        );
    }
    let internal = callgraph
        .edges
        .iter()
        .filter(|edge| ids.contains(edge.source.as_str()) && ids.contains(edge.target.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        layout.edges.len(),
        internal.len(),
        "only intra-module call edges are drawn"
    );

    let selected = "src/a.ts#function:alpha";
    let node = layout
        .nodes
        .iter()
        .find(|node| node.id == selected)
        .expect("alpha node");
    assert_eq!(
        state.select_at(node.rect.center()),
        Some(selected.to_string())
    );

    let mut expected = BTreeSet::new();
    for edge in &callgraph.edges {
        if edge.source == selected {
            expected.insert(edge.target.clone());
        }
        if edge.target == selected {
            expected.insert(edge.source.clone());
        }
    }
    expected.remove(selected);
    assert_eq!(
        state.highlighted(),
        expected,
        "highlighted neighbors must match the call graph in both directions"
    );
    assert!(
        state
            .highlighted()
            .contains(&"src/util.ts#function:helper".to_string()),
        "cross-module neighbors stay highlighted even when not drawn"
    );
}

#[test]
fn returning_restores_pan_and_zoom() {
    let (_, map, callgraph) = build();
    let mut state = CanvasState::new(map);

    state.pan_by(120.0, -30.0);
    state.zoom_at(Point { x: 400.0, y: 300.0 }, 1.5);
    let saved = state.viewport();

    assert!(state.drill_down("src/a.ts", &callgraph));
    assert_eq!(
        state.viewport().zoom,
        1.0,
        "drilldown resets to identity view"
    );
    assert!(state.can_go_back());

    assert!(state.go_back());
    let restored = state.viewport();
    assert_eq!(restored.pan_x, saved.pan_x);
    assert_eq!(restored.pan_y, saved.pan_y);
    assert_eq!(restored.zoom, saved.zoom);
    assert_eq!(state.level(), Level::Modules);
    assert!(!state.can_go_back());
}

#[test]
fn hit_test_selects_center_and_clears_on_blank() {
    let (_, map, callgraph) = build();
    let mut state = CanvasState::new(map);
    assert!(state.drill_down("src/a.ts", &callgraph));

    let target = "src/a.ts#function:beta";
    let center = state
        .symbol_layout()
        .expect("symbol layout")
        .nodes
        .iter()
        .find(|node| node.id == target)
        .expect("beta node")
        .rect
        .center();
    assert_eq!(state.select_at(center), Some(target.to_string()));

    let blank = Point {
        x: state.symbol_layout().expect("symbol layout").width - 1.0,
        y: 1.0,
    };
    assert_eq!(state.select_at(blank), None);
    assert_eq!(state.selected(), None);
    assert!(state.highlighted().is_empty());
}

#[test]
fn hit_test_uses_screen_coordinates_after_pan_and_zoom() {
    let (_, map, callgraph) = build();
    let mut state = CanvasState::new(map);
    assert!(state.drill_down("src/a.ts", &callgraph));

    let target = "src/a.ts#method:Service.run";
    let center = state
        .symbol_layout()
        .expect("symbol layout")
        .nodes
        .iter()
        .find(|node| node.id == target)
        .expect("run node")
        .rect
        .center();

    state.pan_by(80.0, 40.0);
    state.zoom_at(Point { x: 600.0, y: 400.0 }, 1.5);
    let screen = screen_of(&state, center);
    assert_eq!(state.select_at(screen), Some(target.to_string()));
}

#[test]
fn pan_and_zoom_are_readable_and_lod_follows_threshold() {
    let (_, map, _) = build();
    let mut state = CanvasState::new(map);

    assert_eq!(state.viewport().zoom, 1.0);
    assert!(state.labels_visible(), "labels visible at default zoom");

    let anchor = Point { x: 600.0, y: 400.0 };
    let world_under_anchor = state.viewport().world_point(anchor);
    state.zoom_at(anchor, 0.5);
    assert!(state.viewport().zoom < LABEL_ZOOM_THRESHOLD);
    assert!(!state.labels_visible(), "labels hide below LOD threshold");
    let moved = state.viewport().world_point(anchor);
    assert!(
        (moved.x - world_under_anchor.x).abs() <= 0.01
            && (moved.y - world_under_anchor.y).abs() <= 0.01,
        "zoom must keep the anchor's world point fixed"
    );

    state.zoom_at(anchor, 2.0);
    assert!(state.labels_visible(), "labels return above LOD threshold");

    state.zoom_at(anchor, 0.001);
    assert_eq!(state.viewport().zoom, MIN_ZOOM);
    state.zoom_at(anchor, 1000.0);
    assert_eq!(state.viewport().zoom, MAX_ZOOM);

    let before = state.viewport();
    state.pan_by(35.0, -20.0);
    assert_eq!(state.viewport().pan_x, before.pan_x + 35.0);
    assert_eq!(state.viewport().pan_y, before.pan_y - 20.0);
}

fn synthetic_5k() -> (LayersArtifact, CallGraphArtifact) {
    let modules = (0..5000)
        .map(|index| {
            let id = format!("m{index:04}.ts");
            LayeredModule {
                id,
                path: String::new(),
                kind: "source",
                language: "typescript",
                lines: 10,
                layer: LAYER_ORDER[index % 6],
                rule: None,
                reason: None,
            }
        })
        .collect::<Vec<_>>();
    let edges = modules
        .windows(2)
        .map(|pair| LayeredEdge {
            source: pair[0].id.clone(),
            target: pair[1].id.clone(),
            kind: "import".to_string(),
            direction_adjusted: false,
        })
        .collect::<Vec<_>>();

    let layers = LayersArtifact {
        schema_version: 1,
        tool_version: "0.1.0".to_string(),
        repo: RepoInfo {
            path: "synthetic".to_string(),
            head: None,
            is_git: false,
        },
        stats: Stats {
            files: 5000,
            modules: 5000,
            edges: edges.len(),
            unresolved: 0,
        },
        layer_order: LAYER_ORDER.to_vec(),
        modules,
        unclassified: Vec::new(),
        edges,
        unresolved: Vec::new(),
    };

    let module = "m0000.ts";
    let symbols = (0..40)
        .map(|index| SymbolNode {
            id: format!("{module}#function:f{index:02}"),
            module: module.to_string(),
            path: module.to_string(),
            kind: "function",
            name: format!("f{index:02}"),
            line: index + 1,
            column: 1,
        })
        .collect::<Vec<_>>();
    let call_edges = symbols
        .windows(2)
        .map(|pair| CallEdge {
            source: pair[0].id.clone(),
            target: pair[1].id.clone(),
            kind: "call".to_string(),
            line: 1,
            column: 1,
        })
        .collect::<Vec<_>>();

    let callgraph = CallGraphArtifact {
        schema_version: CALLGRAPH_SCHEMA_VERSION,
        tool_version: "0.1.0".to_string(),
        repo: RepoInfo {
            path: "synthetic".to_string(),
            head: None,
            is_git: false,
        },
        stats: CallGraphStats {
            files: 1,
            symbols: symbols.len(),
            edges: call_edges.len(),
            module_edges: 0,
            unattributed_calls: 0,
            unresolved_imports: 0,
            external_calls: 0,
        },
        symbols,
        edges: call_edges,
        module_edges: Vec::new(),
        unresolved: Vec::new(),
    };

    (layers, callgraph)
}

#[test]
fn synthetic_5k_start_drill_and_return_stay_within_budget() {
    let (layers, callgraph) = synthetic_5k();
    let config = LayoutConfig::default();

    let mut start = Vec::new();
    let mut drill = Vec::new();
    let mut back = Vec::new();

    for _ in 0..3 {
        let timer = Instant::now();
        let map = layout_map(&layers, config);
        let mut state = CanvasState::new(map);
        start.push(timer.elapsed().as_secs_f64() * 1000.0);

        let timer = Instant::now();
        assert!(state.drill_down("m0000.ts", &callgraph));
        drill.push(timer.elapsed().as_secs_f64() * 1000.0);

        let timer = Instant::now();
        assert!(state.go_back());
        back.push(timer.elapsed().as_secs_f64() * 1000.0);
    }

    let median = |values: &[f64]| {
        let mut sorted = values.to_vec();
        sorted.sort_by(|left, right| left.total_cmp(right));
        sorted[sorted.len() / 2]
    };
    let p95 = |values: &[f64]| {
        let mut sorted = values.to_vec();
        sorted.sort_by(|left, right| left.total_cmp(right));
        sorted[(sorted.len() * 95).div_ceil(100) - 1]
    };

    eprintln!(
        "5k budget: start median={:.1}ms p95={:.1}ms; drill median={:.1}ms p95={:.1}ms; back median={:.1}ms p95={:.1}ms",
        median(&start),
        p95(&start),
        median(&drill),
        p95(&drill),
        median(&back),
        p95(&back)
    );
    eprintln!(
        "hardware: os={} cpus={} processor={}",
        std::env::consts::OS,
        std::env::var("NUMBER_OF_PROCESSORS").unwrap_or_else(|_| "unknown".to_string()),
        std::env::var("PROCESSOR_IDENTIFIER").unwrap_or_else(|_| "unknown".to_string())
    );

    assert!(
        start.iter().all(|value| *value <= 2000.0),
        "start over budget"
    );
    assert!(
        drill.iter().all(|value| *value <= 2000.0),
        "drilldown over budget"
    );
    assert!(
        back.iter().all(|value| *value <= 2000.0),
        "return over budget"
    );
}
