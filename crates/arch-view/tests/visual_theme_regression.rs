use arch_view::arch_view_theme::StrataTheme;
use arch_view::diff;
use arch_view::layout::{NodeLayout, Rect};
use arch_view::view;

#[test]
fn theme_constants_match_view_and_diff_baselines() {
    let theme = StrataTheme::obsidian_dark();
    assert_eq!(theme.canvas_bg, gpui::rgb(view::CANVAS_BACKGROUND).into());
    assert_eq!(theme.text_primary, gpui::rgb(view::NODE_LABEL).into());
    assert_eq!(
        StrataTheme::hex_with_alpha(diff::DIFF_LABEL, 1.0),
        gpui::rgb(diff::DIFF_LABEL).into()
    );
}

#[test]
fn non_neighbor_nodes_dimmed_when_selection_active() {
    let theme = StrataTheme::obsidian_dark();
    let node = NodeLayout {
        id: "src/a.ts".to_string(),
        layer: "components",
        rect: Rect::new(0.0, 0.0, 100.0, 40.0),
        label_rect: Rect::new(4.0, 4.0, 92.0, 32.0),
        hit_rect: Rect::new(0.0, 0.0, 100.0, 40.0),
        z_index: 1,
        visual_role: "normal",
        is_unclassified: false,
        is_external: false,
    };
    let dimmed = theme.resolve_module_node_style(&node, false, false, true, false, 1.0);
    assert!(
        (dimmed.bg.a - 0.22).abs() < 1e-6,
        "bg alpha {}",
        dimmed.bg.a
    );
    assert!(
        (dimmed.label_color.a - 0.22).abs() < 1e-6,
        "label alpha {}",
        dimmed.label_color.a
    );

    let normal = theme.resolve_module_node_style(&node, false, false, false, false, 1.0);
    assert!((normal.bg.a - 1.0).abs() < 1e-6, "bg alpha {}", normal.bg.a);
    assert!(
        (normal.label_color.a - 1.0).abs() < 1e-6,
        "label alpha {}",
        normal.label_color.a
    );
}

#[test]
fn direction_adjusted_edges_use_warning_color() {
    let theme = StrataTheme::obsidian_dark();
    let (adjusted_color, adjusted_w) = theme.resolve_edge_style(false, true, 1.0, 1.0);
    let (normal_color, normal_w) = theme.resolve_edge_style(false, false, 1.0, 1.0);
    assert_ne!(adjusted_color, normal_color);
    assert!(f32::from(adjusted_w) > f32::from(normal_w));
}
