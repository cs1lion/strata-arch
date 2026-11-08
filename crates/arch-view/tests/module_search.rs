//! ticket 20（模块搜索与聚焦）的 B 档断言：匹配/排序/选中窗口/一跳集合/标签/复原。
//!
//! 判据见 `docs/tickets/arch-tool/20-module-search-focus.md`。
//! 这些断言只依赖 `arch_view::search` 纯模型，不需要窗口。

use std::path::{Path, PathBuf};
use std::time::Instant;

use arch_core::layers::build_layers;
use arch_view::layout::{layout_map, EdgeLayout, LayoutConfig, MapLayout, NodeLayout, Point, Rect};
use arch_view::playback::{CONTEXT_OPACITY, PATH_OPACITY};
use arch_view::search::{
    interpret_key, KeyModifiers, SearchAction, SearchKey, SearchMode, SearchState, MAX_VISIBLE_HITS,
};
use arch_view::view::Viewport;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn node(id: &str) -> NodeLayout {
    let rect = Rect::new(0.0, 0.0, 100.0, 40.0);
    NodeLayout {
        id: id.to_string(),
        layer: "unclassified",
        rect,
        label_rect: Rect::new(6.0, 6.0, 88.0, 28.0),
        hit_rect: rect,
        z_index: 1000,
        visual_role: "unclassified",
        is_unclassified: true,
        is_external: false,
    }
}

fn edge(source: &str, target: &str) -> EdgeLayout {
    EdgeLayout {
        source: source.to_string(),
        target: target.to_string(),
        direction_adjusted: false,
        from: Point { x: 0.0, y: 0.0 },
        to: Point { x: 1.0, y: 1.0 },
        z_index: 1,
    }
}

fn layout(ids: &[&str], edges: Vec<EdgeLayout>) -> MapLayout {
    MapLayout {
        width: 1200.0,
        height: 800.0,
        bands: Vec::new(),
        nodes: ids.iter().map(|id| node(id)).collect(),
        edges,
        overlap_area: 0.0,
        overlap_area_threshold: 0.0,
    }
}

fn anchored(edges: &[(&str, &str)]) -> Vec<EdgeLayout> {
    edges
        .iter()
        .map(|(source, target)| edge(source, target))
        .collect()
}

fn state_with(ids: &[&str], edges: &[(&str, &str)]) -> SearchState {
    SearchState::new(&layout(ids, anchored(edges)))
}

fn query_hits(state: &mut SearchState, query: &str) -> Vec<String> {
    state.open();
    state.set_query(query);
    state.hits().iter().map(|hit| hit.id.clone()).collect()
}

#[test]
fn matching_is_substring_case_insensitive_and_empty_query_matches_nothing() {
    let ids = [
        "src/Button.tsx",
        "components/Button.tsx",
        "docs/button.md",
        "src/button-helper/index.ts",
        "src/other.ts",
    ];
    let mut state = state_with(&ids, &[]);

    let hits = query_hits(&mut state, "BUTTON");
    assert_eq!(
        hits,
        vec![
            "components/Button.tsx",
            "docs/button.md",
            "src/Button.tsx",
            "src/button-helper/index.ts",
        ],
        "basename 命中优先，路径命中最后，同级按 id 升序"
    );

    let exact = query_hits(&mut state, "button.tsx");
    assert_eq!(
        exact,
        vec!["components/Button.tsx", "src/Button.tsx"],
        "basename 完全相等排在最前，同级按 id 升序"
    );

    let empty = query_hits(&mut state, "");
    assert!(empty.is_empty(), "空串必须返回 0 命中");
    assert_eq!(state.summary(), "无匹配 · 0/0");
}

#[test]
fn ranking_is_stable_and_four_level() {
    let ids = [
        "a/Button.tsx",
        "b/Button.tsx",
        "c/ButtonPanel.tsx",
        "d/xButton.tsx",
        "e/button/index.ts",
        "f/unrelated.ts",
    ];
    let mut state = state_with(&ids, &[]);

    assert_eq!(
        query_hits(&mut state, "button"),
        vec![
            "a/Button.tsx",
            "b/Button.tsx",
            "c/ButtonPanel.tsx",
            "d/xButton.tsx",
            "e/button/index.ts",
        ],
        "rank 0 > 1 > 2 > 3，同级 id 升序"
    );

    let first = query_hits(&mut state, "button");
    let second = query_hits(&mut state, "button");
    assert_eq!(first, second, "同一 query 两次结果必须一致");
}

#[test]
fn selection_does_not_wrap_and_window_follows() {
    let ids = [
        "m/aa.ts", "m/ab.ts", "m/ac.ts", "m/ad.ts", "m/ae.ts", "m/af.ts", "m/ag.ts", "m/ah.ts",
        "m/ai.ts", "m/aj.ts",
    ];
    let mut state = state_with(&ids, &[]);
    state.open();
    state.set_query("a");
    assert!(state.hit_count() >= 10);

    state.move_selection(-1);
    assert_eq!(state.selected(), 0, "首项向上不循环");
    state.move_selection(100);
    assert_eq!(state.selected(), state.hit_count() - 1, "末项向下不循环");

    state.move_selection(-3);
    assert_eq!(
        state.list_start(),
        state.selected() + 1 - MAX_VISIBLE_HITS,
        "选中移出窗口时窗口跟随"
    );
    assert!(
        state
            .visible_hits()
            .iter()
            .any(|hit| hit.id == state.hits()[state.selected()].id),
        "选中项必须可见"
    );
}

#[test]
fn hit_count_keeps_total_while_list_shows_six() {
    let ids = [
        "q/mod-a.ts",
        "q/mod-b.ts",
        "q/mod-c.ts",
        "q/mod-d.ts",
        "q/mod-e.ts",
        "q/mod-f.ts",
        "q/mod-g.ts",
        "q/mod-h.ts",
    ];
    let mut state = state_with(&ids, &[]);
    state.open();
    state.set_query("mod");
    assert_eq!(state.hit_count(), 8);
    assert_eq!(state.visible_hits().len(), MAX_VISIBLE_HITS);
    assert_eq!(state.summary(), "前 6/共 8");
}

#[test]
fn focus_set_is_one_hop_bidirectional_and_excludes_two_hops() {
    let ids = ["A", "B", "C", "D", "E", "F"];
    // A -> B、A -> A（自环）、C -> A、D -> A、E -> C（二跳）、E -> F（两个非成员）。
    let edges = [
        ("A", "B"),
        ("A", "A"),
        ("A", "B"),
        ("C", "A"),
        ("D", "A"),
        ("E", "C"),
        ("E", "F"),
    ];
    let mut state = state_with(&ids, &edges);
    state.open();
    state.set_query("a");
    assert_eq!(state.selected_hit().map(|hit| hit.id.as_str()), Some("A"));
    let request = state.confirm(Viewport::default()).expect("confirm");
    assert_eq!(request.target, Point { x: 50.0, y: 20.0 });

    let set = state.focus_set().expect("focus set");
    assert_eq!(
        set.upstream.iter().cloned().collect::<Vec<_>>(),
        vec!["C", "D"],
        "上游只含直接来源，自环不计入"
    );
    assert_eq!(
        set.downstream.iter().cloned().collect::<Vec<_>>(),
        vec!["A", "B"],
        "下游含自环，B 的重复边只计一次"
    );
    assert_eq!(set.members.len(), 4, "成员为 A/B/C/D，E 是二跳不进入");
    assert!(!set.contains("E"));
    assert!(!set.contains("F"));
}

#[test]
fn focus_label_counts_match_the_one_hop_sets_and_self_loop_counts_once() {
    let ids = ["A", "B", "C", "D"];
    let edges = [("A", "A"), ("A", "B"), ("C", "A")];
    let mut state = state_with(&ids, &edges);
    state.open();
    state.set_query("a");
    state.confirm(Viewport::default());

    let set = state.focus_set().expect("focus set").clone();
    let label = state.focus_label().expect("focus label");
    assert_eq!(label.module, "A");
    assert_eq!(
        label.upstream,
        set.upstream.len(),
        "标签上游数必须等于集合条目"
    );
    assert_eq!(
        label.downstream,
        set.downstream.len(),
        "标签下游数必须等于集合条目"
    );
    assert_eq!(label.upstream, 1, "C -> A");
    assert_eq!(label.downstream, 2, "A -> A 与 A -> B，自环只计一次");
    assert_eq!(label.text(), "聚焦 A · 上游 1 / 下游 2", "标签文案固定格式");
}

#[test]
fn focus_opacity_is_one_or_context_only() {
    let ids = ["A", "B", "C", "D", "E", "F"];
    let edges = [("A", "B"), ("C", "A"), ("E", "F")];
    let mut state = state_with(&ids, &edges);
    state.open();
    state.set_query("a");
    state.confirm(Viewport::default());

    for id in ["A", "B", "C"] {
        assert_eq!(state.module_opacity(id), PATH_OPACITY, "{id} 在焦点集合内");
    }
    for id in ["D", "E", "F"] {
        assert_eq!(state.module_opacity(id), CONTEXT_OPACITY, "{id} 在集合外");
    }

    assert_eq!(state.edge_opacity("A", "B"), PATH_OPACITY, "两端都在集合内");
    assert_eq!(state.edge_opacity("C", "A"), PATH_OPACITY, "两端都在集合内");
    assert_eq!(
        state.edge_opacity("E", "F"),
        CONTEXT_OPACITY,
        "两端都在集合外"
    );

    for edge in &edges {
        let opacity = state.edge_opacity(edge.0, edge.1);
        assert!(
            opacity == PATH_OPACITY || opacity == CONTEXT_OPACITY,
            "边的透明度只能取 {{1.0, 0.68}}"
        );
    }
}

#[test]
fn escape_clears_label_and_restores_viewport_fields_exactly() {
    let ids = ["A", "B"];
    let mut state = state_with(&ids, &[("A", "B")]);
    let viewport = Viewport {
        pan_x: 12.5,
        pan_y: -3.25,
        zoom: 1.75,
    };
    state.open();
    state.set_query("a");
    state.confirm(viewport);
    assert_eq!(state.mode(), SearchMode::Focused);
    assert!(state.focus_label().is_some(), "聚焦态必须有标签");

    let restored = state.clear_focus().expect("snapshot");
    assert_eq!(restored.pan_x, viewport.pan_x);
    assert_eq!(restored.pan_y, viewport.pan_y);
    assert_eq!(restored.zoom, viewport.zoom);
    assert_eq!(state.mode(), SearchMode::Closed);
    assert!(state.focus_label().is_none(), "清除后标签必须不可见");
    assert_eq!(state.module_opacity("A"), 1.0, "清除后淡出复原");
    assert_eq!(state.edge_opacity("A", "B"), 1.0);
}

#[test]
fn confirm_without_hits_keeps_input_state() {
    let ids = ["A"];
    let mut state = state_with(&ids, &[]);
    state.open();
    state.set_query("zzz");
    assert_eq!(state.mode(), SearchMode::Input);
    assert!(state.confirm(Viewport::default()).is_none());
    assert_eq!(state.mode(), SearchMode::Input, "空命中 Enter 不进入聚焦");
    assert!(state.cancel_input());
    assert_eq!(state.mode(), SearchMode::Closed);
    assert_eq!(state.query(), "", "关闭后 query 不残留");
}

#[test]
fn open_and_escape_state_machine() {
    let ids = ["A"];
    let mut state = state_with(&ids, &[]);
    assert_eq!(state.mode(), SearchMode::Closed);

    assert!(state.open());
    assert!(state.is_open());
    assert_eq!(state.summary(), "无匹配 · 0/0");

    state.push_char('a');
    state.push_char('b');
    state.backspace();
    assert_eq!(state.query(), "a");

    assert!(state.cancel_input());
    assert_eq!(state.mode(), SearchMode::Closed);
    assert_eq!(state.query(), "");

    state.open();
    state.set_query("a");
    state.confirm(Viewport::default());
    assert!(!state.open(), "聚焦态不重复打开面板");
}

#[test]
fn key_interpretation_covers_slash_secondary_f_and_editing_keys() {
    let none = KeyModifiers::default();
    let secondary = KeyModifiers {
        secondary: true,
        alt: false,
    };
    assert_eq!(interpret_key("/", Some("/"), none), SearchKey::Open);
    assert_eq!(interpret_key("f", Some("f"), secondary), SearchKey::Open);
    assert_eq!(
        interpret_key("f", Some("f"), none),
        SearchKey::Character('f')
    );
    assert_eq!(interpret_key("/", Some("/"), secondary), SearchKey::None);
    assert_eq!(interpret_key("escape", None, none), SearchKey::Close);
    assert_eq!(interpret_key("enter", Some("\r"), none), SearchKey::Confirm);
    assert_eq!(interpret_key("up", None, none), SearchKey::Previous);
    assert_eq!(interpret_key("down", None, none), SearchKey::Next);
    assert_eq!(interpret_key("backspace", None, none), SearchKey::Backspace);
    assert_eq!(
        interpret_key(
            "a",
            Some("a"),
            KeyModifiers {
                secondary: false,
                alt: true,
            }
        ),
        SearchKey::None,
        "Alt 组合不进入输入"
    );
}

#[test]
fn handle_maps_keys_to_actions_without_crossing_states() {
    let ids = ["A", "B"];
    let viewport = Viewport {
        pan_x: 4.0,
        pan_y: 5.0,
        zoom: 2.0,
    };
    let mut state = state_with(&ids, &[("A", "B")]);

    assert_eq!(state.handle(SearchKey::Close, viewport), SearchAction::None);
    assert_eq!(
        state.handle(SearchKey::Open, viewport),
        SearchAction::Opened
    );
    assert_eq!(
        state.handle(SearchKey::Character('a'), viewport),
        SearchAction::None
    );
    assert_eq!(state.query(), "a");
    assert_eq!(
        state.handle(SearchKey::Confirm, viewport),
        SearchAction::Focused(arch_view::search::FocusRequest {
            target: Point { x: 50.0, y: 20.0 },
            snapshot: viewport,
        })
    );
    assert_eq!(
        state.handle(SearchKey::Character('b'), viewport),
        SearchAction::None,
        "聚焦态忽略输入"
    );
    assert_eq!(state.query(), "", "Enter 后面板关闭且输入清空");
    assert_eq!(
        state.handle(SearchKey::Close, viewport),
        SearchAction::Cleared(viewport)
    );
    assert_eq!(state.mode(), SearchMode::Closed);
}

#[test]
fn matching_802_module_corpus_stays_within_budget() {
    let corpus = workspace_root().join("tests/corpus/excalidraw");
    if !corpus.is_dir() {
        eprintln!("skip: tests/corpus/excalidraw 不存在（外部语料未归档）");
        return;
    }
    let artifact = build_layers(&corpus, "0.1.0").expect("excalidraw corpus");
    let map = layout_map(&artifact, LayoutConfig::default());
    assert_eq!(map.nodes.len(), 802, "语料模块数必须为 802");

    let mut state = SearchState::new(&map);
    state.open();
    let mut samples = Vec::new();
    for _ in 0..101 {
        let timer = Instant::now();
        state.set_query("components");
        samples.push(timer.elapsed().as_secs_f64() * 1000.0);
        assert!(state.hit_count() > 0);
    }
    samples.sort_by(|left, right| left.partial_cmp(right).expect("finite"));
    let median = samples[samples.len() / 2];
    println!(
        "search: {} modules, query hits {}, median {:.3}ms, p95 {:.3}ms",
        map.nodes.len(),
        state.hit_count(),
        median,
        samples[(samples.len() as f64 * 0.95) as usize]
    );
    if !cfg!(debug_assertions) {
        assert!(
            median <= 3.0,
            "802 模块匹配中位数必须 ≤3ms，实测 {median:.3}ms"
        );
    }
}
