//! 对接 `strata-arch` 现有模型的 GPUI 矢量画布渲染器（适配版）

use std::collections::BTreeSet;

use gpui::{
    canvas, div, fill, point, prelude::*, px, quad, size, BorderStyle, Bounds, Corners, Div, Edges,
    Hsla, PathBuilder, Pixels, Point as GpuiPoint,
};

use crate::arch_view_theme::StrataTheme;
use crate::layout::{BandLayout, MapLayout, NodeLayout, Point as ArchPoint, Rect as ArchRect};
use crate::playback::PlaybackState;
use crate::search::SearchState;
use crate::view::{
    group_edges, paint_symbol_edges, paint_symbol_edges_with_roles, rect_fully_outside,
    segment_fully_outside, CanvasState, EdgeRun, EdgeVisual, Level, PaintedSymbolEdge,
    SymbolLayout, SymbolNodeLayout, Viewport, CULL_MARGIN,
};

#[derive(Clone, Copy, Default)]
pub struct CanvasRenderSkip {
    pub bands: bool,
    pub edges: bool,
    pub nodes: bool,
    pub labels: bool,
}

#[derive(Clone, Copy)]
pub struct CanvasRenderTuning {
    pub batch_edges: bool,
    pub cull_viewport: bool,
}

impl Default for CanvasRenderTuning {
    fn default() -> Self {
        Self {
            batch_edges: true,
            cull_viewport: true,
        }
    }
}

#[inline]
fn to_window_point(pt: ArchPoint, canvas_origin: GpuiPoint<Pixels>) -> GpuiPoint<Pixels> {
    point(canvas_origin.x + px(pt.x), canvas_origin.y + px(pt.y))
}

#[inline]
fn to_window_bounds(rect: ArchRect, canvas_origin: GpuiPoint<Pixels>) -> Bounds<Pixels> {
    Bounds {
        origin: point(canvas_origin.x + px(rect.x), canvas_origin.y + px(rect.y)),
        size: size(px(rect.width.max(0.0)), px(rect.height.max(0.0))),
    }
}

/// 点阵背景的行列上限：防止高分视口（如 4K）在 zoom 使步长落在 14–15px 区间时，
/// 单帧仍循环生成约 42,000 个 `paint_quad`（超出实际可见点阵需求）。
const MAX_DOT_GRID_CELLS: usize = 12_000;

pub fn paint_dot_grid(
    window: &mut gpui::Window,
    bounds: Bounds<Pixels>,
    viewport: &Viewport,
    theme: &StrataTheme,
) {
    let base_step = f32::from(theme.grid_spacing) * viewport.zoom.max(0.25);
    // 步长翻倍阈值提高至 18.0px：消除「14–15px 区间仍翻倍、点数随之翻倍」的高分屏开销。
    let step = if base_step < 18.0 {
        base_step * 2.0
    } else {
        base_step
    };
    if step <= 0.0 {
        return;
    }
    let offset_x = viewport.pan_x.rem_euclid(step);
    let offset_y = viewport.pan_y.rem_euclid(step);
    let width = f32::from(bounds.size.width);
    let height = f32::from(bounds.size.height);
    let dot_size = size(px(1.5), px(1.5));
    let rows_per_step = (height / step).max(1.0);
    let cols_per_step = (width / step).max(1.0);
    // 仅在点数远超上限时按等比比例收敛，正常视口保持原有点阵密度。
    let uncapped_cells = rows_per_step * cols_per_step;
    let factor = if uncapped_cells > 16_000.0 {
        MAX_DOT_GRID_CELLS as f32 / uncapped_cells
    } else {
        1.0
    };
    let max_rows = (rows_per_step * factor).floor().clamp(1.0, rows_per_step) as usize;
    let max_cols = (cols_per_step * factor).floor().clamp(1.0, cols_per_step) as usize;
    let mut row_idx = 0usize;
    let mut y = offset_y;
    while y < height && row_idx < max_rows {
        let mut col_idx = 0usize;
        let mut x = offset_x;
        while x < width && col_idx < max_cols {
            let is_major = row_idx.is_multiple_of(4) && col_idx.is_multiple_of(4);
            let color = if is_major {
                theme.dot_grid_major
            } else {
                theme.dot_grid
            };
            let dot_bounds = Bounds {
                origin: point(bounds.origin.x + px(x), bounds.origin.y + px(y)),
                size: dot_size,
            };
            window.paint_quad(fill(dot_bounds, color));
            x += step;
            col_idx += 1;
        }
        y += step;
        row_idx += 1;
    }
}

pub fn paint_layer_bands(
    window: &mut gpui::Window,
    canvas_bounds: Bounds<Pixels>,
    bands: &[BandLayout],
    viewport: &Viewport,
    cull_viewport: bool,
    reveal_alpha: f32,
    theme: &StrataTheme,
) {
    let vp_size = (
        f32::from(canvas_bounds.size.width),
        f32::from(canvas_bounds.size.height),
    );
    let radius = theme.band_radius * viewport.zoom.clamp(0.6, 1.2);
    for band in bands {
        let screen_rect = viewport.screen_rect(band.rect);
        if cull_viewport && rect_fully_outside(screen_rect, vp_size, CULL_MARGIN) {
            continue;
        }
        let (fill_color, border_color, accent_color) = theme.band_style(
            band.layer,
            band.is_unclassified,
            band.is_external,
            reveal_alpha,
        );
        let bounds = to_window_bounds(screen_rect, canvas_bounds.origin);
        window.paint_quad(quad(
            bounds,
            Corners::all(radius),
            fill_color,
            Edges::all(px(1.0)),
            border_color,
            BorderStyle::default(),
        ));
        let header_bar = Bounds {
            origin: point(bounds.origin.x + px(1.0), bounds.origin.y + px(6.0)),
            size: size(px(3.5), px((16.0 * viewport.zoom).clamp(10.0, 22.0))),
        };
        window.paint_quad(quad(
            header_bar,
            Corners::all(px(2.0)),
            accent_color,
            Edges::all(px(0.0)),
            gpui::transparent_black(),
            BorderStyle::default(),
        ));
    }
}

#[allow(clippy::too_many_arguments)]
pub fn paint_cubic_bezier_arrow(
    window: &mut gpui::Window,
    start: GpuiPoint<Pixels>,
    end: GpuiPoint<Pixels>,
    stroke_width: Pixels,
    color: Hsla,
    zoom: f32,
    vertical_flow: bool,
    draw_arrowhead: bool,
) {
    let dx = f32::from(end.x) - f32::from(start.x);
    let dy = f32::from(end.y) - f32::from(start.y);
    if dx.hypot(dy) < 2.0 {
        return;
    }
    let (ctrl1, ctrl2) = if vertical_flow {
        let offset_y = (dy.abs() * 0.45).clamp(24.0 * zoom, 140.0 * zoom);
        let sign_y = if dy >= 0.0 { 1.0 } else { -1.0 };
        (
            point(start.x, start.y + px(offset_y * sign_y)),
            point(end.x, end.y - px(offset_y * sign_y)),
        )
    } else {
        let offset_x = (dx.abs() * 0.45).clamp(28.0 * zoom, 150.0 * zoom);
        let sign_x = if dx >= 0.0 { 1.0 } else { -1.0 };
        (
            point(start.x + px(offset_x * sign_x), start.y),
            point(end.x - px(offset_x * sign_x), end.y),
        )
    };
    let mut builder = PathBuilder::stroke(stroke_width);
    builder.move_to(start);
    builder.cubic_bezier_to(end, ctrl1, ctrl2);
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
    if !draw_arrowhead {
        return;
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
    let left_wing = point(
        px(base_x - uy * arrow_half_w),
        px(base_y + ux * arrow_half_w),
    );
    let right_wing = point(
        px(base_x + uy * arrow_half_w),
        px(base_y - ux * arrow_half_w),
    );
    let mut arrow = PathBuilder::fill();
    arrow.move_to(end);
    arrow.line_to(left_wing);
    arrow.line_to(right_wing);
    arrow.close();
    if let Ok(arrow_path) = arrow.build() {
        window.paint_path(arrow_path, color);
    }
}

#[inline]
fn curve_cull_margin(zoom: f32, dy: f32) -> f32 {
    // 曲线控制点外扩量（与 `paint_cubic_bezier_arrow` 的控制点偏移同量级）：
    // 视口裁剪时用它对齐固定 `CULL_MARGIN`，避免起止点在视口外侧、曲线中段向视口内弯曲时整条曲线被提前裁剪。
    let offset = (dy.abs() * 0.45).clamp(24.0 * zoom, 140.0 * zoom);
    CULL_MARGIN.max(offset)
}

#[allow(clippy::too_many_arguments)]
pub fn paint_module_edges(
    window: &mut gpui::Window,
    canvas_bounds: Bounds<Pixels>,
    edges: &[EdgeVisual],
    runs: &[EdgeRun],
    zoom: f32,
    cull_viewport: bool,
    _reveal_alpha: f32,
    theme: &StrataTheme,
) {
    let vp_size = (
        f32::from(canvas_bounds.size.width),
        f32::from(canvas_bounds.size.height),
    );
    for run in runs {
        let (color, stroke_w) =
            theme.resolve_edge_style(run.strong, run.adjusted, run.opacity, zoom);
        let mut batched_builder = PathBuilder::stroke(stroke_w);
        let mut has_batched_segments = false;
        for &edge_idx in &run.edges {
            let Some(edge) = edges.get(edge_idx) else {
                continue;
            };
            let cull_margin = curve_cull_margin(zoom, edge.to.y - edge.from.y);
            if cull_viewport && segment_fully_outside(edge.from, edge.to, vp_size, cull_margin) {
                continue;
            }
            let start = to_window_point(edge.from, canvas_bounds.origin);
            let end = to_window_point(edge.to, canvas_bounds.origin);
            if run.strong || run.adjusted {
                paint_cubic_bezier_arrow(window, start, end, stroke_w, color, zoom, true, true);
            } else {
                let dy = f32::from(end.y) - f32::from(start.y);
                let offset_y = (dy.abs() * 0.45).clamp(24.0 * zoom, 140.0 * zoom);
                let sign_y = if dy >= 0.0 { 1.0 } else { -1.0 };
                let ctrl1 = point(start.x, start.y + px(offset_y * sign_y));
                let ctrl2 = point(end.x, end.y - px(offset_y * sign_y));
                batched_builder.move_to(start);
                batched_builder.cubic_bezier_to(end, ctrl1, ctrl2);
                has_batched_segments = true;
            }
        }
        if has_batched_segments {
            if let Ok(path) = batched_builder.build() {
                window.paint_path(path, color);
            }
        }
    }
}

pub fn paint_symbol_edges_canvas(
    window: &mut gpui::Window,
    canvas_bounds: Bounds<Pixels>,
    painted_edges: &[PaintedSymbolEdge],
    zoom: f32,
    cull_viewport: bool,
    theme: &StrataTheme,
) {
    let vp_size = (
        f32::from(canvas_bounds.size.width),
        f32::from(canvas_bounds.size.height),
    );
    for edge in painted_edges {
        if cull_viewport && segment_fully_outside(edge.from, edge.to, vp_size, CULL_MARGIN) {
            continue;
        }
        let (color, stroke_w) = theme.resolve_edge_style(edge.strong, false, edge.opacity, zoom);
        let start = to_window_point(edge.from, canvas_bounds.origin);
        let end = to_window_point(edge.to, canvas_bounds.origin);
        paint_cubic_bezier_arrow(window, start, end, stroke_w, color, zoom, false, true);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn paint_module_nodes_canvas(
    window: &mut gpui::Window,
    canvas_bounds: Bounds<Pixels>,
    nodes: &[NodeLayout],
    viewport: &Viewport,
    selected: Option<&str>,
    highlighted: &BTreeSet<String>,
    search_matches: &BTreeSet<String>,
    cull_viewport: bool,
    reveal_alpha: f32,
    theme: &StrataTheme,
) {
    let vp_size = (
        f32::from(canvas_bounds.size.width),
        f32::from(canvas_bounds.size.height),
    );
    let has_any_selection = selected.is_some() || !search_matches.is_empty();
    let radius = theme.card_radius * viewport.zoom.clamp(0.55, 1.25);
    for node in nodes {
        let screen_rect = viewport.screen_rect(node.rect);
        if cull_viewport && rect_fully_outside(screen_rect, vp_size, CULL_MARGIN) {
            continue;
        }
        let is_selected = selected == Some(node.id.as_str());
        let is_neighbor = highlighted.contains(&node.id);
        let is_search_match = search_matches.contains(&node.id);
        let style = theme.resolve_module_node_style(
            node,
            is_selected,
            is_neighbor,
            has_any_selection,
            is_search_match,
            reveal_alpha,
        );
        let bounds = to_window_bounds(screen_rect, canvas_bounds.origin);
        if style.shadow.a > 0.05 {
            let shadow_bounds = Bounds {
                origin: point(
                    bounds.origin.x + px(1.5),
                    bounds.origin.y + px(3.5 * viewport.zoom),
                ),
                size: bounds.size,
            };
            window.paint_quad(quad(
                shadow_bounds,
                Corners::all(radius),
                style.shadow,
                Edges::all(px(0.0)),
                gpui::transparent_black(),
                BorderStyle::default(),
            ));
        }
        window.paint_quad(quad(
            bounds,
            Corners::all(radius),
            style.bg,
            Edges::all(style.border_width),
            style.border,
            BorderStyle::default(),
        ));
        let bar_w = theme.accent_bar_width * viewport.zoom.clamp(0.7, 1.2);
        let bar_bounds = Bounds {
            origin: point(
                bounds.origin.x + px(1.0),
                bounds.origin.y + px(4.0 * viewport.zoom),
            ),
            size: size(
                bar_w,
                (bounds.size.height - px(8.0 * viewport.zoom)).max(px(4.0)),
            ),
        };
        window.paint_quad(quad(
            bar_bounds,
            Corners::all(px(2.0)),
            style.accent_bar,
            Edges::all(px(0.0)),
            gpui::transparent_black(),
            BorderStyle::default(),
        ));
    }
}

#[allow(clippy::too_many_arguments)]
pub fn paint_symbol_nodes_canvas(
    window: &mut gpui::Window,
    canvas_bounds: Bounds<Pixels>,
    nodes: &[SymbolNodeLayout],
    viewport: &Viewport,
    selected: Option<&str>,
    caller: Option<&str>,
    callee: Option<&str>,
    cull_viewport: bool,
    theme: &StrataTheme,
) {
    let vp_size = (
        f32::from(canvas_bounds.size.width),
        f32::from(canvas_bounds.size.height),
    );
    let has_focus = selected.is_some() || caller.is_some() || callee.is_some();
    let radius = theme.card_radius * viewport.zoom.clamp(0.55, 1.25);
    for node in nodes {
        let screen_rect = viewport.screen_rect(node.rect);
        if cull_viewport && rect_fully_outside(screen_rect, vp_size, CULL_MARGIN) {
            continue;
        }
        let is_selected = selected == Some(node.id.as_str());
        let is_caller = caller == Some(node.id.as_str());
        let is_callee = callee == Some(node.id.as_str());
        let style =
            theme.resolve_symbol_node_style(node, is_selected, is_caller, is_callee, has_focus);
        let bounds = to_window_bounds(screen_rect, canvas_bounds.origin);
        window.paint_quad(quad(
            bounds,
            Corners::all(radius),
            style.bg,
            Edges::all(style.border_width),
            style.border,
            BorderStyle::default(),
        ));
        let bar_w = theme.accent_bar_width * viewport.zoom.clamp(0.7, 1.2);
        let bar_bounds = Bounds {
            origin: point(
                bounds.origin.x + px(1.0),
                bounds.origin.y + px(4.0 * viewport.zoom),
            ),
            size: size(
                bar_w,
                (bounds.size.height - px(8.0 * viewport.zoom)).max(px(4.0)),
            ),
        };
        window.paint_quad(quad(
            bar_bounds,
            Corners::all(px(2.0)),
            style.accent_bar,
            Edges::all(px(0.0)),
            gpui::transparent_black(),
            BorderStyle::default(),
        ));
    }
}

#[allow(clippy::too_many_arguments)]
pub fn render_strata_canvas_surface(
    state: &CanvasState,
    search: &SearchState,
    playback: Option<&PlaybackState>,
    fade_enabled: bool,
    reveal_alpha: f32,
    skip: CanvasRenderSkip,
    tuning: CanvasRenderTuning,
    theme: &StrataTheme,
) -> Div {
    let viewport = state.viewport();
    let level = state.level();
    let selected_id = state.selected().map(str::to_string);
    let highlighted_ids = state.highlighted();
    let search_matches: BTreeSet<String> = search
        .focus_set()
        .map(|s| s.members.clone())
        .unwrap_or_default();

    let map_clone: MapLayout = state.map().clone();
    let symbols_clone: Option<SymbolLayout> = state.symbol_layout().cloned();
    let theme_for_canvas = theme.clone();
    let selected_for_canvas = selected_id.clone();
    let highlighted_for_canvas = highlighted_ids.clone();
    let search_for_canvas = search_matches.clone();
    let level_for_canvas = level.clone();
    let focus_for_canvas = search.focus_set().cloned();

    let precomputed_symbol_edges: Vec<PaintedSymbolEdge> = match (&level_for_canvas, &symbols_clone)
    {
        (Level::Symbols { .. }, Some(sym_layout)) => {
            let mut edges = if let Some(pb) = playback {
                if fade_enabled {
                    paint_symbol_edges_with_roles(sym_layout, &viewport, pb)
                } else {
                    paint_symbol_edges(sym_layout, &viewport, pb.current_edge().as_ref())
                }
            } else {
                paint_symbol_edges(sym_layout, &viewport, None)
            };
            for e in &mut edges {
                e.opacity *= reveal_alpha;
            }
            edges
        }
        _ => Vec::new(),
    };

    div().size_full().relative().bg(theme.canvas_bg).child(
        canvas(
            move |bounds, _window, _cx| bounds,
            move |bounds, _prepaint, window, _cx| {
                paint_dot_grid(window, bounds, &viewport, &theme_for_canvas);
                match &level_for_canvas {
                    Level::Modules => {
                        if !skip.bands {
                            paint_layer_bands(
                                window,
                                bounds,
                                &map_clone.bands,
                                &viewport,
                                tuning.cull_viewport,
                                reveal_alpha,
                                &theme_for_canvas,
                            );
                        }
                        if !skip.edges {
                            let vp_size =
                                (f32::from(bounds.size.width), f32::from(bounds.size.height));
                            let focus = focus_for_canvas.as_ref();
                            let edges = map_clone
                                .edges
                                .iter()
                                .filter_map(|edge| {
                                    let from = viewport.screen_point(edge.from);
                                    let to = viewport.screen_point(edge.to);
                                    if tuning.cull_viewport
                                        && segment_fully_outside(from, to, vp_size, CULL_MARGIN)
                                    {
                                        return None;
                                    }
                                    let opacity = match focus {
                                        Some(set) => {
                                            set.edge_opacity(&edge.source, &edge.target)
                                                * reveal_alpha
                                        }
                                        None => reveal_alpha,
                                    };
                                    Some(EdgeVisual {
                                        from,
                                        to,
                                        adjusted: edge.direction_adjusted,
                                        strong: false,
                                        opacity,
                                    })
                                })
                                .collect::<Vec<_>>();
                            let runs = group_edges(&edges, tuning.batch_edges);
                            paint_module_edges(
                                window,
                                bounds,
                                &edges,
                                &runs,
                                viewport.zoom,
                                tuning.cull_viewport,
                                1.0,
                                &theme_for_canvas,
                            );
                        }
                        if !skip.nodes {
                            paint_module_nodes_canvas(
                                window,
                                bounds,
                                &map_clone.nodes,
                                &viewport,
                                selected_for_canvas.as_deref(),
                                &highlighted_for_canvas,
                                &search_for_canvas,
                                tuning.cull_viewport,
                                reveal_alpha,
                                &theme_for_canvas,
                            );
                        }
                    }
                    Level::Symbols { .. } => {
                        if let Some(sym_layout) = &symbols_clone {
                            if !skip.edges {
                                paint_symbol_edges_canvas(
                                    window,
                                    bounds,
                                    &precomputed_symbol_edges,
                                    viewport.zoom,
                                    tuning.cull_viewport,
                                    &theme_for_canvas,
                                );
                            }
                            if !skip.nodes {
                                paint_symbol_nodes_canvas(
                                    window,
                                    bounds,
                                    &sym_layout.nodes,
                                    &viewport,
                                    selected_for_canvas.as_deref(),
                                    None,
                                    None,
                                    tuning.cull_viewport,
                                    &theme_for_canvas,
                                );
                            }
                        }
                    }
                }
            },
        )
        .size_full(),
    )
}

/// 票 24 D 节：map 实际绘制步的淡出 α 合成。此前 `a *= opacity` 内联在 `main.rs` 的绘制闭包里，
/// 且无任何测试锁定（`resolve_module_node_style` 的单测锁的是 canvas 路径，map 命令不走它）。
/// 提为纯函数以锁死「乘法而非替换」语义。
///
/// 约束：不得写成 `a = opacity`——压暗态（`a = 0.22`）与播放透明度（`0.68`）叠加必须得
/// `0.1496`；替换会得 `0.68`，这正是判别性测试 `fade_multiplies_not_replaces` 要咬住的差异。
pub fn apply_fade_alpha(bg: Hsla, border: Hsla, accent: Hsla, opacity: f32) -> (Hsla, Hsla, Hsla) {
    let mut bg = bg;
    bg.a *= opacity;
    let mut border = border;
    border.a *= opacity;
    let mut accent = accent;
    accent.a *= opacity;
    (bg, border, accent)
}

/// 票 29：单色 α 缩放。提为纯函数以锁死「乘法而非替换」语义——此前该写法内联在 `main.rs`
/// 的 `TimelineView` 绘制闭包（`bg.a *= item.opacity` / `border.a *= item.opacity`）且无测试锁定。
///
/// 约束：不得写成 `c.a = opacity`。判别输入必须取 `a != 1.0`（见
/// `tests/alpha_scale_contract.rs` 的 `timeline_alpha_multiplies_not_replaces`）——
/// 若喂 `a = 1.0`，乘法与替换恰好同值，测试会失去判别性（`fade_alpha_contract.rs` 的 T2/T3 教训）。
pub fn scale_alpha(c: Hsla, opacity: f32) -> Hsla {
    let mut out = c;
    out.a *= opacity;
    out
}

/// 票 29：三色一组的 α 缩放（`map` 层带的 fill / border / header）。与 `apply_fade_alpha`
/// 语义等价（等价性由 `symbol_and_band_agree_with_apply_fade_alpha` 钉住），独立成函数只为让
/// `main.rs` 内不再残留任何内联 `*.a *=` 乘法（负向守护 `main-no-inline-alpha-multiply` 的前提）。
pub fn scale_alpha3(trio: (Hsla, Hsla, Hsla), opacity: f32) -> (Hsla, Hsla, Hsla) {
    let (first, second, third) = trio;
    (
        scale_alpha(first, opacity),
        scale_alpha(second, opacity),
        scale_alpha(third, opacity),
    )
}
