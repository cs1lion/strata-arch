//! ticket 21 路线 A：B 档「断言先行」清单（实现前落盘，先于产品代码提交）。
//!
//! 路线 A 的前提是「输入期间场景元素树不变，只有面板变」。本文件在实现任何
//! `AnyView::cached` 接线之前，把这条前提钉成可自动执行的结构断言；后续重构
//! 若破坏了它（例如让 Input 态也改场景透明度），缓存就会画出过期画面。
//!
//! | 断言 | 可见行为 |
//! | --- | --- |
//! | `input_keystrokes_never_change_scene_visuals` | 逐字输入/退格/上下选择期间，场景不透明度恒为 1、无焦点集合；面板可以变，场景逐像素应不变 |
//! | `confirm_and_clear_change_scene_visuals` | `Enter` 聚焦后焦点集合出现、非焦点模块淡出；`Esc` 清除后恢复；这两条路径必须让缓存失效 |
//! | `open_close_are_panel_only_transitions` | 打开/取消搜索只改变面板，不改变场景视觉 |
//!
//! C 档预算冻结（照抄票 21 判据 1，不在实现后调整）：
//! - 命令：`pwsh -File eval/frame-timing.ps1 -Mode search -SearchPrefix "/components" -Exe target/release/arch-view.exe`（81 键 / 1ms 每键 / 79 间隔）；
//! - 固定样本：`tests/corpus/excalidraw`（802 模块 / 4363 边 / 6 层带），release 构建；
//! - 期望读数：**连续 3 次独立运行，每次独立计算 p95，3 次 p95 的最大值 ≤33ms**，同时报 median/min/max；
//! - 失败判据：任一 p95 >33ms；面板外出现非零像素差；任一失效路径出现残影；为达标改动 33ms 或像素闸门；
//! - 自动化状态：性能为脚本实测（需可见桌面会话），像素证据 `eval/pixel-evidence.ps1` 24/24 为人工项，均不进 CI。
//! - 这是绝对阈值，不是两批相对比较；不得改成顺序两批 A/B（票 10 记录 32% 批次漂移）。

use arch_view::layout::{MapLayout, NodeLayout, Rect};
use arch_view::search::{SearchAction, SearchKey::*, SearchState};
use arch_view::view::Viewport;

const NODE_ID: &str = "src/app.ts";

fn sample_map() -> MapLayout {
    let rect = Rect::new(10.0, 10.0, 150.0, 40.0);
    MapLayout {
        width: 1200.0,
        height: 800.0,
        bands: vec![],
        nodes: vec![NodeLayout {
            id: NODE_ID.to_string(),
            layer: "domain",
            rect,
            label_rect: rect,
            hit_rect: rect,
            z_index: 1,
            visual_role: "layer",
            is_unclassified: false,
            is_external: false,
        }],
        edges: vec![],
        overlap_area: 0.0,
        overlap_area_threshold: 0.0,
    }
}

fn opened() -> SearchState {
    let mut search = SearchState::new(&sample_map());
    assert!(
        matches!(
            search.handle(Open, Viewport::default()),
            SearchAction::Opened
        ),
        "搜索必须能从 Closed 打开"
    );
    search
}

#[test]
fn input_keystrokes_never_change_scene_visuals() {
    let mut search = opened();
    let keys = [
        Character('a'),
        Character('p'),
        Character('p'),
        Backspace,
        Next,
        Previous,
    ];
    for key in keys {
        let action = search.handle(key, Viewport::default());
        assert!(
            matches!(action, SearchAction::None),
            "输入期间的按键不得触发场景级动作（聚焦/清除/相机变化）"
        );
        assert!(
            search.focus_set().is_none(),
            "Input 期间必须没有焦点集合，否则场景会随按键变淡"
        );
        assert_eq!(
            search.module_opacity(NODE_ID),
            1.0,
            "Input 期间节点不透明度必须恒为 1"
        );
        assert_eq!(
            search.edge_opacity("src/a.ts", "src/b.ts"),
            1.0,
            "Input 期间边不透明度必须恒为 1"
        );
    }
}

#[test]
fn confirm_and_clear_change_scene_visuals() {
    let mut search = opened();
    assert!(matches!(
        search.handle(Character('a'), Viewport::default()),
        SearchAction::None
    ));

    let action = search.handle(Confirm, Viewport::default());
    assert!(
        matches!(action, SearchAction::Focused(_)),
        "Enter 必须产生聚焦动作"
    );
    assert!(
        search.focus_set().is_some(),
        "聚焦后场景出现焦点集合，缓存必须失效"
    );
    assert_eq!(search.module_opacity(NODE_ID), 1.0, "焦点模块保持不透明");
    assert_eq!(
        search.module_opacity("src/other.ts"),
        0.68,
        "非焦点模块淡出——这是场景级变化，缓存不得复用上一帧"
    );

    let action = search.handle(Close, Viewport::default());
    assert!(
        matches!(action, SearchAction::Cleared(_)),
        "Esc 必须清除聚焦"
    );
    assert!(search.focus_set().is_none(), "清除后场景恢复全不透明");
    assert_eq!(search.module_opacity("src/other.ts"), 1.0);
}

#[test]
fn open_close_are_panel_only_transitions() {
    let mut search = SearchState::new(&sample_map());
    assert!(search.focus_set().is_none());

    assert!(matches!(
        search.handle(Open, Viewport::default()),
        SearchAction::Opened
    ));
    assert!(search.focus_set().is_none(), "打开搜索不改场景");
    assert_eq!(search.module_opacity(NODE_ID), 1.0);

    assert!(matches!(
        search.handle(Close, Viewport::default()),
        SearchAction::Closed
    ));
    assert!(search.focus_set().is_none(), "取消搜索不改场景");
    assert_eq!(search.edge_opacity("src/a.ts", "src/b.ts"), 1.0);
}

#[test]
fn typed_query_changes_panel_only() {
    let mut search = opened();
    assert!(search.query().is_empty());
    assert!(matches!(
        search.handle(Character('a'), Viewport::default()),
        SearchAction::None
    ));
    assert_eq!(search.query(), "a", "query 变化属于面板状态");
    assert!(
        !search.hits().is_empty(),
        "面板应给出命中项，证明按键确实被消费"
    );
    assert!(
        search.focus_set().is_none(),
        "即使有了命中项，场景仍不得变化"
    );
}
