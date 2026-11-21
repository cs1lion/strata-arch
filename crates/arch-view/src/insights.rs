use std::collections::BTreeMap;

use arch_view::arch_view_theme::StrataTheme;
use arch_view::layout::MapLayout;
use gpui::{div, prelude::*, px, AnyElement};

/// 架构健康洞察摘要（由 `MapLayout` 纯确定性计算，零时钟/随机依赖）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArchInsights {
    pub(crate) module_count: usize,
    pub(crate) edge_count: usize,
    /// 逆向/越层依赖边 `(source, target)`（字典序去重）。
    pub(crate) reversed_edges: Vec<(String, String)>,
    /// Top 5 核心枢纽模块 `(module_id, in_degree, out_degree)`，按入度降序排列。
    pub(crate) top_hubs: Vec<(String, usize, usize)>,
    /// 入度与出度均为 0 的孤立模块数。
    pub(crate) isolated_count: usize,
}

pub(crate) fn compute_insights(map: &MapLayout) -> ArchInsights {
    let mut degrees: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for node in &map.nodes {
        degrees.entry(node.id.as_str()).or_insert((0, 0));
    }

    let mut reversed_edges: Vec<(String, String)> = Vec::new();
    for edge in &map.edges {
        if let Some(entry) = degrees.get_mut(edge.source.as_str()) {
            entry.1 += 1;
        }
        if let Some(entry) = degrees.get_mut(edge.target.as_str()) {
            entry.0 += 1;
        }
        if edge.direction_adjusted {
            reversed_edges.push((edge.source.clone(), edge.target.clone()));
        }
    }
    reversed_edges.sort();
    reversed_edges.dedup();

    let mut isolated_count = 0usize;
    let mut ranked: Vec<(String, usize, usize)> = Vec::with_capacity(degrees.len());
    for (id, (in_deg, out_deg)) in degrees {
        if in_deg == 0 && out_deg == 0 {
            isolated_count += 1;
        } else {
            ranked.push((id.to_string(), in_deg, out_deg));
        }
    }
    ranked.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| (b.1 + b.2).cmp(&(a.1 + a.2)))
            .then_with(|| a.0.cmp(&b.0))
    });
    ranked.truncate(5);

    ArchInsights {
        module_count: map.nodes.len(),
        edge_count: map.edges.len(),
        reversed_edges,
        top_hubs: ranked,
        isolated_count,
    }
}

/// 快捷键速查浮层（`?` 键开关，天然跟随全局深/浅主题）。
pub(crate) fn shortcuts_overlay_element(theme: &StrataTheme) -> AnyElement {
    const ROWS: &[(&str, &str)] = &[
        ("/ 或 Ctrl/Cmd+F", "搜索并聚焦模块"),
        ("E", "循环切换边显示模式（全部 / 仅聚焦 / 隐藏）"),
        ("0 或 R", "适应画布并居中重置视口"),
        ("+ / -", "以视口中心放大 / 缩小"),
        ("单击模块", "选中模块并展开右侧 Inspector 详情栏"),
        ("再次点击 / Enter", "下钻进入选中模块的符号级调用视图"),
        ("I", "开关左上角架构健康洞察面板"),
        ("?", "开关本快捷键速查浮层"),
        ("Esc", "依次关闭浮层、取消选中或清除层过滤"),
    ];

    let mut card = div()
        .id("shortcuts-overlay")
        .absolute()
        .left(px(360.))
        .top(px(88.))
        .w(px(460.))
        .px(px(16.))
        .py(px(14.))
        .rounded(px(10.))
        .border_1()
        .border_color(theme.hud_border)
        .bg(theme.hud_surface_bg)
        .flex()
        .flex_col()
        .gap(px(8.))
        .text_xs()
        .text_color(theme.text_primary)
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(div().text_sm().text_color(theme.text_bright).child("快捷键速查"))
                .child(div().text_color(theme.text_muted).child("按 ? 或 Esc 关闭")),
        );

    for &(key_label, desc) in ROWS {
        card = card.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(12.))
                .py(px(2.))
                .child(
                    div()
                        .px(px(7.))
                        .py(px(2.))
                        .rounded(px(4.))
                        .border_1()
                        .border_color(theme.hud_border)
                        .bg(theme.chip_bg)
                        .text_color(theme.text_contrast)
                        .child(key_label),
                )
                .child(div().text_color(theme.text_secondary).child(desc)),
        );
    }

    card.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::compute_insights;
    use arch_view::layout::{EdgeLayout, MapLayout, NodeLayout, Point, Rect};

    fn dummy_node(id: &str) -> NodeLayout {
        let rect = Rect::new(0.0, 0.0, 100.0, 40.0);
        NodeLayout {
            id: id.to_string(),
            layer: "domain",
            rect,
            label_rect: rect,
            hit_rect: rect,
            z_index: 1000,
            visual_role: "layer",
            is_unclassified: false,
            is_external: false,
        }
    }

    fn dummy_edge(src: &str, dst: &str, adjusted: bool) -> EdgeLayout {
        EdgeLayout {
            source: src.to_string(),
            target: dst.to_string(),
            direction_adjusted: adjusted,
            from: Point { x: 0.0, y: 0.0 },
            to: Point { x: 10.0, y: 10.0 },
            z_index: 1,
        }
    }

    #[test]
    fn compute_insights_ranks_hubs_and_detects_reversed_and_isolated() {
        let map = MapLayout {
            width: 800.0,
            height: 600.0,
            bands: Vec::new(),
            nodes: vec![
                dummy_node("src/ui/a.ts"),
                dummy_node("src/ui/b.ts"),
                dummy_node("src/core/hub.ts"),
                dummy_node("src/unused/orphan.ts"),
            ],
            edges: vec![
                dummy_edge("src/ui/a.ts", "src/core/hub.ts", false),
                dummy_edge("src/ui/b.ts", "src/core/hub.ts", false),
                dummy_edge("src/core/hub.ts", "src/ui/a.ts", true),
            ],
            overlap_area: 0.0,
            overlap_area_threshold: 0.0,
        };

        let insights = compute_insights(&map);
        assert_eq!(insights.module_count, 4);
        assert_eq!(insights.edge_count, 3);
        assert_eq!(insights.isolated_count, 1);
        assert_eq!(
            insights.reversed_edges,
            vec![("src/core/hub.ts".to_string(), "src/ui/a.ts".to_string())]
        );
        assert_eq!(
            insights.top_hubs.first(),
            Some(&("src/core/hub.ts".to_string(), 2, 1))
        );
    }
}
