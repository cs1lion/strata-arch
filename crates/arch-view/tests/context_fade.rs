//! ticket 16（P1/G1 完成版）的 B 档断言：上下文淡出 + 模块/符号同步。
//!
//! 这些断言先于实现提交。核心性质：
//!
//! 1. 淡化是「步骤索引切换时的状态变化」，同一步内任何帧的透明度必须恒定；
//!    所有透明度只能取 `{PATH_OPACITY, CONTEXT_OPACITY}`（淡出态）或旧值 `0.25`（关闭态）。
//! 2. 当前步节点及其 incoming/outgoing 边是最高层；非路径元素保持 `CONTEXT_OPACITY`
//!    下限，标签对比度按 WCAG AA 实测。
//! 3. 关闭开关时逐位回到旧行为：路径 1.0 / 非路径 0.25、仅 incoming 边被强调、边不淡化。
//! 4. 淡化节点仍可命中、可下钻；绘制顺序 Context → Path → Current，同角色保持原序。
//! 5. 模块级与符号级视图共用同一 `current` 索引；下钻/返回不改变步骤与高亮。
//! 6. 四种反向用例不出现异常态或错误层级。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use arch_core::callgraph::{build_call_graph, CallGraphArtifact};
use arch_core::guided_path::{build_path, GuidedPathArtifact, PathStep};
use arch_core::layers::build_layers;
use arch_core::RepoInfo;
use arch_view::layout::{layout_map, LayoutConfig};
use arch_view::playback::{
    opacity_alpha_byte, EdgeRole, PlaybackState, VisualRole, CONTEXT_OPACITY, NON_PATH_OPACITY,
    PATH_OPACITY,
};
use arch_view::view::{
    band_background_palette, layer_palette, paint_symbol_edges, paint_symbol_edges_with_roles,
    symbol_kind_palette, CanvasState, SymbolLayout, CANVAS_BACKGROUND, NODE_LABEL,
};

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

/// 3 步、跨 3 个模块、三步均为 `unclassified`：`src/b.ts` → `src/a.ts` → `src/util.ts`。
fn three_step_path() -> GuidedPathArtifact {
    build_path(
        &fixture(),
        "test",
        "src/b.ts#function:caller",
        "src/util.ts#function:helper",
    )
    .expect("fixture path caller -> helper")
}

/// 2 步、起终点同模块：`src/a.ts` 内的 `beta -> alpha`。
fn two_step_same_module() -> GuidedPathArtifact {
    build_path(
        &fixture(),
        "test",
        "src/a.ts#function:beta",
        "src/a.ts#function:alpha",
    )
    .expect("fixture path beta -> alpha")
}

fn single_step() -> GuidedPathArtifact {
    build_path(
        &fixture(),
        "test",
        "src/a.ts#function:alpha",
        "src/a.ts#function:alpha",
    )
    .expect("single step path")
}

fn advance_to(playback: &mut PlaybackState, target: usize) {
    playback.play();
    for step in 0..target {
        assert!(
            playback.advance(),
            "advance to step {target} must succeed at step {}",
            step + 1
        );
    }
    assert_eq!(playback.current(), target);
}

fn pair(source: &str, target: &str) -> (String, String) {
    (source.to_string(), target.to_string())
}

fn pairs(items: &[(&str, &str)]) -> BTreeSet<(String, String)> {
    items
        .iter()
        .map(|(source, target)| pair(source, target))
        .collect()
}

fn drilled(module: &str) -> (CanvasState, CallGraphArtifact, SymbolLayout) {
    let layers = build_layers(&fixture(), "test").expect("layers fixture");
    let map = layout_map(&layers, LayoutConfig::default());
    let mut state = CanvasState::new(map);
    let graph = callgraph();
    assert!(
        state.drill_down(module, &graph),
        "fixture must drill into {module}"
    );
    let layout = state.symbol_layout().expect("drilled layout").clone();
    (state, graph, layout)
}

const CALLER: &str = "src/b.ts#function:caller";
const ALPHA: &str = "src/a.ts#function:alpha";
const HELPER: &str = "src/util.ts#function:helper";
const BETA: &str = "src/a.ts#function:beta";

const MODULE_B: &str = "src/b.ts";
const MODULE_A: &str = "src/a.ts";
const MODULE_UTIL: &str = "src/util.ts";
const MODULE_CONTEXT: &str = "src/components.tsx";

fn assert_opacity(actual: f32, expected: f32, label: &str) {
    assert!(
        (actual - expected).abs() < f32::EPSILON,
        "{label}: expected {expected}, got {actual}"
    );
}

// ---------------------------------------------------------------------------
// 1. 离散语义：同一步多帧渲染，透明度序列恒定，且不出现中间值
// ---------------------------------------------------------------------------

#[test]
fn fade_is_a_discrete_step_state_not_a_timeline() {
    let graph = callgraph();
    let path = three_step_path();
    let mut playback = PlaybackState::new(&path);
    advance_to(&mut playback, 2);

    let symbols = graph
        .symbols
        .iter()
        .map(|symbol| symbol.id.as_str())
        .collect::<Vec<_>>();
    let edges = graph
        .edges
        .iter()
        .map(|edge| (edge.source.as_str(), edge.target.as_str()))
        .collect::<Vec<_>>();

    let snapshot = |playback: &PlaybackState| {
        let nodes = symbols
            .iter()
            .map(|symbol| playback.node_context_opacity(symbol))
            .collect::<Vec<_>>();
        let edge_opacities = edges
            .iter()
            .map(|(source, target)| playback.edge_context_opacity(source, target))
            .collect::<Vec<_>>();
        (nodes, edge_opacities)
    };

    let first = snapshot(&playback);
    let second = snapshot(&playback);
    let third = snapshot(&playback);
    assert_eq!(
        first, second,
        "same step must render identical opacity frame 1 -> 2"
    );
    assert_eq!(
        second, third,
        "same step must render identical opacity frame 2 -> 3"
    );

    let allowed = [PATH_OPACITY, CONTEXT_OPACITY];
    for value in first.0.iter().chain(first.1.iter()) {
        assert!(
            allowed
                .iter()
                .any(|candidate| (*candidate - *value).abs() < f32::EPSILON),
            "discrete fade must not interpolate: {value} is not one of {allowed:?}"
        );
    }
    assert!(
        !first
            .0
            .iter()
            .chain(first.1.iter())
            .any(|value| (*value - NON_PATH_OPACITY).abs() < f32::EPSILON),
        "fade-on state must not fall back to the legacy 0.25 opacity"
    );

    // 绘制层同一入口也要逐帧恒定。
    let (_state, _graph, layout) = drilled(MODULE_A);
    let painted_frames = (0..3)
        .map(|_| {
            paint_symbol_edges_with_roles(&layout, &Default::default(), &playback)
                .into_iter()
                .map(|edge| edge.opacity)
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert_eq!(painted_frames[0], painted_frames[1]);
    assert_eq!(painted_frames[1], painted_frames[2]);

    eprintln!(
        "discrete step {}/{}: node opacities {:?}",
        playback.current(),
        playback.steps().len(),
        first.0
    );
    eprintln!(
        "discrete step {}/{}: edge opacities {:?}",
        playback.current(),
        playback.steps().len(),
        first.1
    );
}

// ---------------------------------------------------------------------------
// 2. 每步数值角色：路径节点/边/非路径元素的 alpha 与层级
// ---------------------------------------------------------------------------

#[test]
fn roles_are_numeric_per_step() {
    let graph = callgraph();
    let path = three_step_path();
    let mut playback = PlaybackState::new(&path);

    let symbols = graph
        .symbols
        .iter()
        .map(|symbol| symbol.id.as_str())
        .collect::<Vec<_>>();
    let edges = graph
        .edges
        .iter()
        .map(|edge| (edge.source.as_str(), edge.target.as_str()))
        .collect::<Vec<_>>();
    let layers = build_layers(&fixture(), "test").expect("layers fixture");
    let map = layout_map(&layers, LayoutConfig::default());

    let caller_pair = pairs(&[(CALLER, ALPHA)]);
    let helper_pair = pairs(&[(ALPHA, HELPER)]);
    let module_ba = pairs(&[(MODULE_B, MODULE_A)]);
    let module_au = pairs(&[(MODULE_A, MODULE_UTIL)]);

    for step in 1..=3 {
        advance_to(&mut playback, step);

        match step {
            1 => {
                assert_eq!(playback.current_symbol(), Some(CALLER));
                assert_eq!(playback.current_module(), Some(MODULE_B));
                assert_eq!(playback.current_edges(), caller_pair);
                assert_eq!(
                    playback.edge_role(CALLER, ALPHA),
                    EdgeRole::Current,
                    "outgoing edge of the current step must be top layer"
                );
                assert_eq!(playback.edge_role(ALPHA, HELPER), EdgeRole::Path);
                assert_eq!(playback.edge_role(BETA, ALPHA), EdgeRole::Context);
                assert_eq!(playback.current_module_edges(), module_ba);
                assert_eq!(
                    playback.module_edge_role(MODULE_B, MODULE_A),
                    EdgeRole::Current
                );
                assert_opacity(
                    playback.node_context_opacity(CALLER),
                    1.0,
                    "current node opacity",
                );
                assert_opacity(
                    playback.node_context_opacity(BETA),
                    CONTEXT_OPACITY,
                    "context node opacity",
                );
                assert_opacity(
                    playback.module_context_opacity(MODULE_CONTEXT),
                    CONTEXT_OPACITY,
                    "context module opacity",
                );
            }
            2 => {
                assert_eq!(playback.current_symbol(), Some(ALPHA));
                assert_eq!(playback.current_module(), Some(MODULE_A));
                assert_eq!(
                    playback.current_edges(),
                    caller_pair.union(&helper_pair).cloned().collect()
                );
                assert_eq!(playback.edge_role(CALLER, ALPHA), EdgeRole::Current);
                assert_eq!(playback.edge_role(ALPHA, HELPER), EdgeRole::Current);
                assert_eq!(
                    playback.current_module_edges(),
                    module_ba.union(&module_au).cloned().collect()
                );
                assert_eq!(playback.module_role(MODULE_A), VisualRole::Current);
                assert_eq!(playback.module_role(MODULE_B), VisualRole::Path);
                assert_eq!(playback.module_role(MODULE_CONTEXT), VisualRole::Context);
            }
            3 => {
                assert_eq!(playback.current_symbol(), Some(HELPER));
                assert_eq!(playback.current_module(), Some(MODULE_UTIL));
                assert_eq!(playback.current_edges(), helper_pair);
                assert_eq!(playback.module_role(MODULE_UTIL), VisualRole::Current);
                assert_opacity(
                    playback.node_context_opacity(HELPER),
                    1.0,
                    "current node opacity",
                );
                assert_eq!(
                    playback.module_edge_role(MODULE_UTIL, MODULE_A),
                    EdgeRole::Context
                );
            }
            _ => unreachable!(),
        }

        let context_nodes = symbols
            .iter()
            .filter(|symbol| playback.node_role(symbol) == VisualRole::Context)
            .count();
        let context_edges = edges
            .iter()
            .filter(|(source, target)| playback.edge_role(source, target) == EdgeRole::Context)
            .count();
        let legacy_faded = symbols
            .iter()
            .filter(|symbol| {
                (playback.node_opacity(symbol) - NON_PATH_OPACITY).abs() < f32::EPSILON
            })
            .count();
        let context_modules = map
            .nodes
            .iter()
            .filter(|node| playback.module_role(&node.id) == VisualRole::Context)
            .count();
        eprintln!(
            "step {}/{}: context nodes {}/{} (legacy 0.25 fades {}/{}), context edges {}/{}, context modules {}/{}",
            playback.current(),
            playback.steps().len(),
            context_nodes,
            symbols.len(),
            legacy_faded,
            symbols.len(),
            context_edges,
            edges.len(),
            context_modules,
            map.nodes.len()
        );
    }
}

// ---------------------------------------------------------------------------
// 3. 淡化下限：每个调色板的标签对比度实测（WCAG AA）
// ---------------------------------------------------------------------------

fn hex(color: u32) -> (f32, f32, f32) {
    (
        ((color >> 16) & 0xff) as f32 / 255.0,
        ((color >> 8) & 0xff) as f32 / 255.0,
        (color & 0xff) as f32 / 255.0,
    )
}

fn blend(fg: (f32, f32, f32), bg: (f32, f32, f32), alpha: f32) -> (f32, f32, f32) {
    (
        fg.0 * alpha + bg.0 * (1.0 - alpha),
        fg.1 * alpha + bg.1 * (1.0 - alpha),
        fg.2 * alpha + bg.2 * (1.0 - alpha),
    )
}

fn channel_luminance(value: f32) -> f32 {
    if value <= 0.03928 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn luminance(color: (f32, f32, f32)) -> f32 {
    0.2126 * channel_luminance(color.0)
        + 0.7152 * channel_luminance(color.1)
        + 0.0722 * channel_luminance(color.2)
}

fn contrast_ratio(foreground: (f32, f32, f32), background: (f32, f32, f32)) -> f32 {
    let first = luminance(foreground);
    let second = luminance(background);
    let (lighter, darker) = if first >= second {
        (first, second)
    } else {
        (second, first)
    };
    (lighter + 0.05) / (darker + 0.05)
}

/// `div.opacity()` 在 GPUI 中按 primitive 逐层乘 alpha：先节点底色压到背景上，
/// 再让标签以同一 alpha 压到「已淡化的节点底色」上。
fn label_contrast(node_color: u32, backdrop: u32, opacity: f32) -> f32 {
    let backdrop = hex(backdrop);
    let node = blend(hex(node_color), backdrop, opacity);
    let label = blend(hex(NODE_LABEL), node, opacity);
    contrast_ratio(label, node)
}

#[test]
fn context_floor_keeps_labels_legible() {
    assert_opacity(CONTEXT_OPACITY, 0.68, "context opacity floor");
    assert_eq!(
        opacity_alpha_byte(CONTEXT_OPACITY),
        173,
        "0.68 must deterministically map to alpha byte 173"
    );
    assert_eq!(opacity_alpha_byte(1.0), 255);
    assert_eq!(opacity_alpha_byte(0.0), 0);

    let symbol_kinds = [
        "function",
        "component",
        "class",
        "method",
        "module-toplevel",
        "other",
    ];
    let mut minimum = f32::MAX;
    let mut minimum_label = String::new();
    for kind in symbol_kinds {
        let ratio = label_contrast(
            symbol_kind_palette(kind),
            CANVAS_BACKGROUND,
            CONTEXT_OPACITY,
        );
        eprintln!("contrast at {CONTEXT_OPACITY} symbol[{kind}]: {ratio:.3}:1");
        if ratio < minimum {
            minimum = ratio;
            minimum_label = format!("symbol[{kind}]");
        }
    }

    let layers = [
        ("cli", false, false),
        ("commands", false, false),
        ("components", false, false),
        ("server", false, false),
        ("utils", false, false),
        ("tests", false, false),
        ("external", false, true),
        ("other", false, false),
        ("unclassified", true, false),
    ];
    for (layer, unclassified, external) in layers {
        let ratio = label_contrast(
            layer_palette(layer, unclassified),
            band_background_palette(unclassified, external),
            CONTEXT_OPACITY,
        );
        eprintln!("contrast at {CONTEXT_OPACITY} layer[{layer}]: {ratio:.3}:1");
        if ratio < minimum {
            minimum = ratio;
            minimum_label = format!("layer[{layer}]");
        }
    }

    // 改前同口径：非路径元素使用旧值 0.25。
    let legacy_minimum = symbol_kinds
        .iter()
        .map(|kind| {
            label_contrast(
                symbol_kind_palette(kind),
                CANVAS_BACKGROUND,
                NON_PATH_OPACITY,
            )
        })
        .fold(f32::MAX, f32::min);
    eprintln!("contrast at {NON_PATH_OPACITY} (legacy) minimum: {legacy_minimum:.3}:1");
    eprintln!("context floor: minimum {minimum:.3}:1 at {minimum_label} (WCAG AA needs 4.5:1)");

    assert!(
        minimum >= 5.0,
        "context fade must keep labels at >= 5.0:1 (AA 4.5 + margin), got {minimum:.3}:1 at {minimum_label}"
    );
    assert!(
        legacy_minimum < 4.5,
        "legacy 0.25 is expected to fail AA; if it passes, the measured comparison is stale: {legacy_minimum:.3}:1"
    );
}

// ---------------------------------------------------------------------------
// 4. 绘制顺序与命中：Context → Path → Current，同角色原序；淡化节点仍可点
// ---------------------------------------------------------------------------

#[test]
fn paint_order_and_hit_testing_are_preserved() {
    let (mut state, graph, layout) = drilled(MODULE_A);
    let path = two_step_same_module();
    let mut playback = PlaybackState::new(&path);
    advance_to(&mut playback, 2);

    let ids = layout
        .nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<Vec<_>>();
    let order = playback.paint_order(&ids);
    let roles = order
        .iter()
        .map(|index| playback.node_role(ids[*index]))
        .collect::<Vec<_>>();
    assert!(
        roles
            .windows(2)
            .all(|window| window[0].rank() <= window[1].rank()),
        "paint order must be Context -> Path -> Current, got {roles:?}"
    );
    assert_eq!(
        roles.last(),
        Some(&VisualRole::Current),
        "the current step must be painted last (top layer), order {order:?}"
    );
    for role in [VisualRole::Context, VisualRole::Path, VisualRole::Current] {
        let filtered = order
            .iter()
            .copied()
            .filter(|index| playback.node_role(ids[*index]) == role)
            .collect::<Vec<_>>();
        assert!(
            filtered.windows(2).all(|window| window[0] < window[1]),
            "same role must keep the original layer order: {filtered:?}"
        );
    }

    // 被淡化的上下文节点仍然可以命中。
    let context_node = layout
        .nodes
        .iter()
        .find(|node| playback.node_role(&node.id) == VisualRole::Context)
        .expect("fixture must contain a context symbol in src/a.ts");
    let hit = state.select_at(context_node.rect.center());
    assert_eq!(
        hit.as_deref(),
        Some(context_node.id.as_str()),
        "faded context node must stay clickable"
    );

    // 当前步节点在其中心仍然是命中结果（命中框与 z_index 规则未改）。
    let current_node = layout
        .nodes
        .iter()
        .find(|node| playback.node_role(&node.id) == VisualRole::Current)
        .expect("current node exists");
    let hit = state.select_at(current_node.rect.center());
    assert_eq!(hit.as_deref(), Some(current_node.id.as_str()));

    // 模块级：被淡化的模块仍可下钻。
    let layers = build_layers(&fixture(), "test").expect("layers");
    let map = layout_map(&layers, LayoutConfig::default());
    let mut modules = CanvasState::new(map.clone());
    let mut module_playback = PlaybackState::new(&three_step_path());
    advance_to(&mut module_playback, 2);
    let context_module = map
        .nodes
        .iter()
        .filter(|node| module_playback.module_role(&node.id) == VisualRole::Context)
        .find(|node| graph.symbols.iter().any(|symbol| symbol.module == node.id))
        .expect("fixture must contain a drillable context module");
    let hit = modules.select_at(context_module.rect.center());
    assert_eq!(hit.as_deref(), Some(context_module.id.as_str()));
    assert!(
        modules.drill_down(&context_module.id, &graph),
        "faded context module must remain drillable"
    );

    eprintln!("paint order for src/a.ts: {order:?}, roles {roles:?}");
}

// ---------------------------------------------------------------------------
// 5. 模块/符号同步：下钻与返回不改变步骤索引与高亮
// ---------------------------------------------------------------------------

#[test]
fn module_and_symbol_views_share_the_step_index() {
    let layers = build_layers(&fixture(), "test").expect("layers");
    let map = layout_map(&layers, LayoutConfig::default());
    let mut state = CanvasState::new(map);
    let graph = callgraph();
    let mut playback = PlaybackState::new(&three_step_path());
    advance_to(&mut playback, 2);

    let module_focus = playback.current_module().map(ToOwned::to_owned);
    let symbol_focus = playback.current_symbol().map(ToOwned::to_owned);
    assert_eq!(module_focus.as_deref(), Some(MODULE_A));
    assert_eq!(symbol_focus.as_deref(), Some(ALPHA));
    assert_eq!(playback.module_role(MODULE_A), VisualRole::Current);
    assert_opacity(
        playback.module_context_opacity(MODULE_A),
        1.0,
        "current module opacity",
    );

    assert!(state.drill_down(MODULE_A, &graph));
    assert_eq!(playback.current(), 2, "drill must not move the step index");
    assert_eq!(playback.current_symbol(), Some(ALPHA));
    assert_eq!(playback.node_role(ALPHA), VisualRole::Current);

    assert!(state.go_back());
    assert_eq!(
        playback.current(),
        2,
        "go_back must not move the step index"
    );
    assert_eq!(playback.current_module(), Some(MODULE_A));
    assert_eq!(playback.module_role(MODULE_A), VisualRole::Current);
    assert_eq!(playback.current_symbol(), Some(ALPHA));

    // 两个视图的焦点都来自同一步：模块视图聚焦的模块 == 符号视图当前步的模块。
    let step = &playback.steps()[playback.current() - 1];
    assert_eq!(step.module, module_focus.clone().unwrap());
    assert_eq!(step.symbol, symbol_focus.clone().unwrap());
    eprintln!(
        "sync: current {}/{}, module {module_focus:?}, symbol {symbol_focus:?}",
        playback.current(),
        playback.steps().len()
    );
}

// ---------------------------------------------------------------------------
// 6. 反向用例：单步 / 全 unclassified / 同模块 / 路径覆盖全部节点
// ---------------------------------------------------------------------------

fn synthetic_path(steps: &[(&str, &str, &'static str)]) -> GuidedPathArtifact {
    assert!(!steps.is_empty(), "synthetic path needs steps");
    GuidedPathArtifact {
        schema_version: 1,
        tool_version: "test".to_string(),
        repo: RepoInfo {
            path: "tests/fixtures/callgraph".to_string(),
            head: None,
            is_git: false,
        },
        from: steps[0].1.to_string(),
        to: steps[steps.len() - 1].1.to_string(),
        found: true,
        length: steps.len(),
        steps: steps
            .iter()
            .enumerate()
            .map(|(index, (module, symbol, layer))| PathStep {
                module: (*module).to_string(),
                symbol: (*symbol).to_string(),
                layer,
                via: if index == 0 {
                    None
                } else {
                    Some("call".to_string())
                },
                evidence: None,
                layer_from: if index == 0 { None } else { Some(layer) },
                layer_to: layer,
                conditional: false,
                test_only: false,
                narrative: String::new(),
            })
            .collect(),
    }
}

#[test]
fn reverse_cases_do_not_produce_invalid_layers() {
    // (a) 单步路径：没有前后上下文，也没有任何当前/路径边。
    let single = single_step();
    assert_eq!(single.steps.len(), 1);
    let mut playback = PlaybackState::new(&single);
    advance_to(&mut playback, 1);
    assert!(playback.current_edges().is_empty());
    assert!(playback.path_edges().is_empty());
    assert!(playback.module_path_edges().is_empty());
    assert!(playback.current_module_edges().is_empty());
    assert_eq!(playback.node_role(ALPHA), VisualRole::Current);
    assert_eq!(playback.module_role(MODULE_A), VisualRole::Current);

    // (b) 3 步路径全部 unclassified：角色仍成立，不因层缺失而降级。
    let path = three_step_path();
    assert!(
        path.steps.iter().all(|step| step.layer == "unclassified"),
        "fixture must stay all-unclassified"
    );
    let mut playback = PlaybackState::new(&path);
    advance_to(&mut playback, 2);
    let step_symbols = [CALLER, ALPHA, HELPER];
    let current_symbols = step_symbols
        .iter()
        .filter(|symbol| playback.node_role(symbol) == VisualRole::Current)
        .count();
    assert_eq!(current_symbols, 1, "exactly one current node");
    for symbol in step_symbols {
        assert!(
            playback.node_role(symbol) != VisualRole::Context,
            "path symbol {symbol} must not be a context node"
        );
    }

    // (c) 起终点同模块：模块路径没有边，当前模块唯一。
    let same = two_step_same_module();
    let mut playback = PlaybackState::new(&same);
    advance_to(&mut playback, 2);
    assert!(playback.module_path_edges().is_empty());
    assert!(playback.current_module_edges().is_empty());
    assert_eq!(playback.module_role(MODULE_A), VisualRole::Current);
    assert_eq!(playback.module_role(MODULE_B), VisualRole::Context);
    assert_opacity(
        playback.module_context_opacity(MODULE_B),
        CONTEXT_OPACITY,
        "context module opacity",
    );

    // (d) 路径覆盖全部节点：模块级 0 个可淡出对象。
    let layers = build_layers(&fixture(), "test").expect("layers");
    let map = layout_map(&layers, LayoutConfig::default());
    let module_steps = map
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node.id.as_str(), "unclassified"))
        .collect::<Vec<_>>();
    let mut playback = PlaybackState::new(&synthetic_path(&module_steps));
    let last = playback.steps().len();
    advance_to(&mut playback, last);
    let context_modules = map
        .nodes
        .iter()
        .filter(|node| playback.module_role(&node.id) == VisualRole::Context)
        .count();
    assert_eq!(
        context_modules, 0,
        "path covering all modules must fade nothing"
    );

    // (d') 符号级：覆盖 src/a.ts 全部符号后 0 个可淡出对象。
    let graph = callgraph();
    let symbol_steps = graph
        .symbols
        .iter()
        .filter(|symbol| symbol.module == MODULE_A)
        .map(|symbol| (MODULE_A, symbol.id.as_str(), "unclassified"))
        .collect::<Vec<_>>();
    assert!(!symbol_steps.is_empty());
    let mut playback = PlaybackState::new(&synthetic_path(&symbol_steps));
    let last = playback.steps().len();
    advance_to(&mut playback, last);
    let (_, _, layout) = drilled(MODULE_A);
    let context_symbols = layout
        .nodes
        .iter()
        .filter(|node| playback.node_role(&node.id) == VisualRole::Context)
        .count();
    assert_eq!(
        context_symbols, 0,
        "path covering all symbols must fade nothing"
    );
    let current_symbols = layout
        .nodes
        .iter()
        .filter(|node| playback.node_role(&node.id) == VisualRole::Current)
        .count();
    assert_eq!(current_symbols, 1);
    eprintln!(
        "reverse: single-step empty edges ok; all-unclassified ok; same-module no module edges ok; full coverage context {context_modules}/{context_symbols}"
    );
}

// ---------------------------------------------------------------------------
// 7. 关闭态逐位回归：旧 0.25、仅 incoming 强调、边不淡化
// ---------------------------------------------------------------------------

#[test]
fn fade_off_keeps_the_legacy_path() {
    assert_opacity(PATH_OPACITY, 1.0, "legacy path opacity");
    assert_opacity(NON_PATH_OPACITY, 0.25, "legacy non-path opacity");

    let mut playback = PlaybackState::new(&three_step_path());
    advance_to(&mut playback, 2);
    assert!(
        !playback.animation_enabled(),
        "fade must be off by default (legacy behavior)"
    );
    assert_opacity(playback.node_opacity(CALLER), 1.0, "legacy path node");
    assert_opacity(playback.node_opacity(BETA), 0.25, "legacy context node");
    assert_eq!(
        playback.current_edge(),
        Some((CALLER.to_string(), ALPHA.to_string())),
        "legacy current edge is incoming-only"
    );
    assert_eq!(
        playback.current_edges(),
        pairs(&[(CALLER, ALPHA), (ALPHA, HELPER)],),
        "fade model may know outgoing, but the legacy painter must not use it"
    );

    let (_state, _graph, layout) = drilled(MODULE_A);
    // 2 步同模块路径：第 2 步的 incoming (beta, alpha) 在 src/a.ts 布局里可见。
    let mut playback = PlaybackState::new(&two_step_same_module());
    advance_to(&mut playback, 2);
    let legacy = paint_symbol_edges(
        &layout,
        &Default::default(),
        playback.current_edge().as_ref(),
    );
    assert!(
        legacy
            .iter()
            .all(|edge| (edge.opacity - 1.0).abs() < f32::EPSILON),
        "legacy painter must not fade any edge"
    );
    let strong = legacy
        .iter()
        .filter(|edge| edge.strong)
        .map(|edge| (edge.source.as_str(), edge.target.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(strong, vec![(BETA, ALPHA)]);

    // 第 1 步的 outgoing 在旧口径下不强调（playback_wiring 的 step1 strong=0 判据保留）。
    advance_to(&mut playback, 1);
    let legacy = paint_symbol_edges(
        &layout,
        &Default::default(),
        playback.current_edge().as_ref(),
    );
    assert_eq!(
        legacy.iter().filter(|edge| edge.strong).count(),
        0,
        "legacy step 1 has no incoming edge and must stay unhighlighted"
    );
    eprintln!(
        "legacy: node opacities {:?}, strong edges {:?}",
        [playback.node_opacity(CALLER), playback.node_opacity(BETA)],
        strong
    );
}

// ---------------------------------------------------------------------------
// 8. 绘制层角色：current incoming+outgoing 最高层，其余上下文统一淡化
// ---------------------------------------------------------------------------

#[test]
fn painted_symbol_edges_use_the_step_roles() {
    let (_state, _graph, layout) = drilled(MODULE_A);
    let path = two_step_same_module();
    let mut playback = PlaybackState::new(&path);

    // 第 1 步：outgoing 边是最高层（旧口径为 0 条 strong，新口径必须 1 条）。
    advance_to(&mut playback, 1);
    let painted = paint_symbol_edges_with_roles(&layout, &Default::default(), &playback);
    let strong = painted
        .iter()
        .filter(|edge| edge.strong)
        .map(|edge| (edge.source.as_str(), edge.target.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        strong,
        vec![(BETA, ALPHA)],
        "step 1 must promote its outgoing edge"
    );
    assert!(
        painted
            .iter()
            .filter(|edge| !edge.strong)
            .all(|edge| (edge.opacity - CONTEXT_OPACITY).abs() < f32::EPSILON),
        "non-current edges must all use the context floor"
    );

    // 第 2 步：incoming 边仍是最强；其余保持上下文淡化。
    advance_to(&mut playback, 2);
    let painted = paint_symbol_edges_with_roles(&layout, &Default::default(), &playback);
    let strong = painted
        .iter()
        .filter(|edge| edge.strong)
        .map(|edge| (edge.source.as_str(), edge.target.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(strong, vec![(BETA, ALPHA)]);
    assert_eq!(
        painted.iter().filter(|edge| edge.strong).count(),
        1,
        "only the current edge is strong"
    );
    eprintln!(
        "painted edges: {} total, {} strong, context opacity {}",
        painted.len(),
        painted.iter().filter(|edge| edge.strong).count(),
        CONTEXT_OPACITY
    );
}
