use std::collections::BTreeSet;

use arch_view::playback::PlaybackState;
use arch_view::search::{FocusLabel, SearchState};
use arch_view::view::{group_edges, EdgeVisual as EdgeDraw};
use gpui::{canvas, div, point, prelude::*, px, rgb, AnyElement, IntoElement, PathBuilder};

pub(crate) fn search_panel_element(search: &SearchState) -> AnyElement {
    let query = if search.query().is_empty() {
        "输入以筛选模块".to_string()
    } else {
        search.query().to_string()
    };
    let theme = arch_view::arch_view_theme::StrataTheme::obsidian_dark();
    let mut panel = div()
        .id("search-panel")
        .absolute()
        .left(px(12.))
        .top(px(12.))
        .w(px(420.))
        .flex()
        .flex_col()
        .gap(px(4.))
        .px(px(10.))
        .py(px(8.))
        .rounded(px(6.))
        .border_1()
        .border_color(theme.hud_border)
        .bg(theme.hud_panel_bg)
        .text_xs()
        .text_color(rgb(0xe6edf3))
        .child(format!("搜索模块 · {query}"))
        .child(div().text_color(rgb(0x8b9bb0)).child(search.summary()));
    let start = search.list_start();
    for (offset, hit) in search.visible_hits().iter().enumerate() {
        let selected = start + offset == search.selected();
        panel = panel.child(
            div()
                .px(px(6.))
                .py(px(2.))
                .rounded(px(4.))
                .bg(if selected {
                    theme.search_hit_bg
                } else {
                    theme.canvas_bg
                })
                .child(hit.id.clone()),
        );
    }
    panel.into_any_element()
}

/// 聚焦态角落标签：聚焦模块 id + 上游 / 下游一跳计数。
pub(crate) fn focus_label_element(label: &FocusLabel) -> AnyElement {
    let theme = arch_view::arch_view_theme::StrataTheme::obsidian_dark();
    div()
        .id("focus-label")
        .absolute()
        .right(px(12.))
        .bottom(px(12.))
        .px(px(10.))
        .py(px(6.))
        .rounded(px(6.))
        .border_1()
        .border_color(theme.hud_border)
        .bg(theme.hud_panel_bg)
        .text_xs()
        .text_color(rgb(0xe6edf3))
        .child(label.text())
        .into_any_element()
}

pub(crate) struct PlaybackOverlay {
    pub(crate) path_modules: BTreeSet<String>,
    pub(crate) current_module: Option<String>,
    pub(crate) current_symbol: Option<String>,
    pub(crate) current_edge: Option<(String, String)>,
    pub(crate) len: usize,
    pub(crate) current: usize,
    pub(crate) playing: bool,
    pub(crate) animation: bool,
    pub(crate) fly_to: bool,
}

impl PlaybackOverlay {
    pub(crate) fn from_state(playback: &PlaybackState) -> Self {
        let steps = playback.steps();
        let modules = steps
            .iter()
            .map(|step| step.module.clone())
            .collect::<Vec<_>>();
        Self {
            path_modules: modules.iter().cloned().collect(),
            current_module: playback.current_module().map(ToOwned::to_owned),
            current_symbol: playback.current_symbol().map(ToOwned::to_owned),
            current_edge: playback.current_edge(),
            len: steps.len(),
            current: playback.current(),
            playing: playback.is_playing(),
            animation: playback.animation_enabled(),
            fly_to: playback.fly_to_enabled(),
        }
    }
}

pub(crate) fn edge_canvas(
    edges: Vec<EdgeDraw>,
    width: f32,
    height: f32,
    zoom: f32,
    batch_edges: bool,
) -> impl IntoElement {
    div()
        .absolute()
        .left(px(0.))
        .top(px(0.))
        .w(px(width))
        .h(px(height))
        .child(canvas(
            |_, _, _| (),
            move |bounds, _, window, _cx| {
                let theme = arch_view::arch_view_theme::StrataTheme::obsidian_dark();
                for run in group_edges(&edges, batch_edges) {
                    let (color, stroke_w) =
                        theme.resolve_edge_style(run.strong, run.adjusted, run.opacity, zoom);
                    let mut batched = PathBuilder::stroke(stroke_w);
                    let mut has_batched = false;
                    for index in &run.edges {
                        let edge = &edges[*index];
                        let start = point(
                            bounds.origin.x + px(edge.from.x),
                            bounds.origin.y + px(edge.from.y),
                        );
                        let end = point(
                            bounds.origin.x + px(edge.to.x),
                            bounds.origin.y + px(edge.to.y),
                        );
                        let dy = f32::from(end.y) - f32::from(start.y);
                        let offset_y = (dy.abs() * 0.45).clamp(24.0 * zoom, 140.0 * zoom);
                        let sign_y = if dy >= 0.0 { 1.0 } else { -1.0 };
                        let ctrl1 = point(start.x, start.y + px(offset_y * sign_y));
                        let ctrl2 = point(end.x, end.y - px(offset_y * sign_y));
                        if run.strong || run.adjusted || !batch_edges {
                            let mut b = PathBuilder::stroke(stroke_w);
                            b.move_to(start);
                            b.cubic_bezier_to(end, ctrl1, ctrl2);
                            if let Ok(path) = b.build() {
                                window.paint_path(path, color);
                            }
                            let vx = f32::from(end.x) - f32::from(ctrl2.x);
                            let vy = f32::from(end.y) - f32::from(ctrl2.y);
                            let len = vx.hypot(vy).max(0.001);
                            let ux = vx / len;
                            let uy = vy / len;
                            let arrow_len = 7.0 * zoom.clamp(0.65, 1.3);
                            let arrow_half_w = 3.5 * zoom.clamp(0.65, 1.3);
                            let base_x = f32::from(end.x) - ux * arrow_len;
                            let base_y = f32::from(end.y) - uy * arrow_len;
                            let left = point(
                                px(base_x - uy * arrow_half_w),
                                px(base_y + ux * arrow_half_w),
                            );
                            let right = point(
                                px(base_x + uy * arrow_half_w),
                                px(base_y - ux * arrow_half_w),
                            );
                            let mut arrow = PathBuilder::fill();
                            arrow.move_to(end);
                            arrow.line_to(left);
                            arrow.line_to(right);
                            arrow.close();
                            if let Ok(ap) = arrow.build() {
                                window.paint_path(ap, color);
                            }
                        } else {
                            batched.move_to(start);
                            batched.cubic_bezier_to(end, ctrl1, ctrl2);
                            has_batched = true;
                        }
                    }
                    if has_batched {
                        if let Ok(path) = batched.build() {
                            window.paint_path(path, color);
                        }
                    }
                }
            },
        ))
}

pub(crate) fn selection_border(
    id: &str,
    theme: &arch_view::arch_view_theme::StrataTheme,
    default: gpui::Hsla,
    selected: Option<&str>,
    highlighted: &BTreeSet<String>,
    focus: Option<&str>,
) -> gpui::Hsla {
    if focus == Some(id) || selected == Some(id) {
        theme.selection_border
    } else if highlighted.contains(id) {
        theme.selection_highlight_border
    } else {
        default
    }
}

/// 将模块路径拆分为（文件名主标题，精简目录副标题），超长目录左侧省略。
pub(crate) fn split_module_label(id: &str) -> (String, Option<String>) {
    let Some((dir, file)) = id.rsplit_once('/') else {
        return (id.to_string(), None);
    };
    if dir.is_empty() {
        return (file.to_string(), None);
    }
    const MAX_DIR_CHARS: usize = 22;
    let subtitle = if dir.chars().count() > MAX_DIR_CHARS {
        let tail = dir
            .chars()
            .rev()
            .take(MAX_DIR_CHARS - 2)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<String>();
        format!("…{tail}")
    } else {
        dir.to_string()
    };
    (file.to_string(), Some(subtitle))
}

#[cfg(test)]
mod tests {
    use super::split_module_label;

    #[test]
    fn split_module_label_extracts_file_and_dir() {
        assert_eq!(
            split_module_label("src/components/Button.tsx"),
            ("Button.tsx".to_string(), Some("src/components".to_string()))
        );
        assert_eq!(
            split_module_label("index.ts"),
            ("index.ts".to_string(), None)
        );
    }

    #[test]
    fn split_module_label_truncates_long_directory_safely() {
        let (title, sub) =
            split_module_label("packages/core/src/features/editor/toolbar/Button.tsx");
        assert_eq!(title, "Button.tsx");
        let sub = sub.expect("subtitle");
        assert!(sub.starts_with('…'));
        assert!(sub.chars().count() <= 22);
    }
}
