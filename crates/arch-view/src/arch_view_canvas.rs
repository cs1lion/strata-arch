//! GPUI 矢量画布的共享绘制原语。
//!
//! Issue #22 收口后，本模块只保留 Map / Timeline 两个视图实际共用的点阵背景绘制。
//! 原先的双轨入口（`render_strata_canvas_surface` 及其调用的 `paint_layer_bands` /
//! `paint_module_edges` / `paint_module_nodes_canvas` / `paint_symbol_nodes_canvas` /
//! `paint_symbol_edges_canvas`，以及只服务于它们的 `CanvasRenderSkip` /
//! `CanvasRenderTuning` / `paint_cubic_bezier_arrow` / `curve_cull_margin` /
//! `to_window_point` / `to_window_bounds`）已全部删除：视图侧从未接入，
//! 且缺少当前渲染路径依赖的语义（详见提交说明），保留只会留下第二套无人维护的画法。

use gpui::{fill, point, px, size, Bounds, Pixels};

use crate::arch_view_theme::StrataTheme;
use crate::view::Viewport;

/// 点阵背景的行列上限：防止高分视口（如 4K）在 zoom 使步长落在 14–15px 区间时，
/// 单帧仍循环生成约 42,000 个 `paint_quad`（超出实际可见点阵需求）。
const MAX_DOT_GRID_CELLS: usize = 12_000;

/// 在窗口画布上绘制与视口平移/缩放对齐的点阵背景。
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
