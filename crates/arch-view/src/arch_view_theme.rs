//! `crates/arch-view/src/theme.rs` —— 对接 `strata-arch` 现有模型的暗黑主题与语义配色模块
//!
//! 对接说明：
//! 1. 严格兼容 `crate::view`（`CANVAS_BACKGROUND`、`NODE_LABEL`、`symbol_kind_palette`、`layer_palette`、
//!    `band_background_palette`）与 `crate::diff`（`DiffKind`、`DIFF_ADDED_TINT` 等）的既有常量与测试断言。
//! 2. 在不改动 `layout.rs` / `view.rs` 纯几何计算与单元测试的前提下，为 `MapView`、`TimelineView` 和 `DiffView`
//!    提供工业级暗黑表面层级、左侧 3px 语义色条、逆向依赖告警色（`direction_adjusted`）与 1-Hop 邻域聚光灯染色。

use gpui::{px, rgb, rgba, App, Global, Hsla, Pixels};

use crate::diff::{DiffKind, DIFF_ADDED_TINT, DIFF_LABEL, DIFF_MODIFIED_TINT, DIFF_REMOVED_TINT};
use crate::layout::NodeLayout;
use crate::view::{
    band_background_palette, layer_palette, symbol_kind_palette, SymbolNodeLayout,
    CANVAS_BACKGROUND, NODE_LABEL,
};

/// 节点卡片在 GPUI `canvas()` 中的解析后绘制样式
#[derive(Debug, Clone, Copy)]
pub struct NodeCardStyle {
    pub bg: Hsla,
    pub border: Hsla,
    pub border_width: Pixels,
    pub accent_bar: Hsla,
    pub badge_bg: Hsla,
    pub badge_fg: Hsla,
    pub label_color: Hsla,
    pub sublabel_color: Hsla,
    pub shadow: Hsla,
    pub port_fill: Hsla,
}

/// `strata-arch` 全局视觉 Token 表（注册为 `gpui::Global`）
#[derive(Debug, Clone)]
pub struct StrataTheme {
    // 1. 画布与层带表面层级 (Canvas & Layer Bands)
    pub canvas_bg: Hsla,
    pub dot_grid: Hsla,
    pub dot_grid_major: Hsla,
    pub hud_panel_bg: Hsla,
    pub hud_border: Hsla,
    /// 搜索面板命中项选中背景（收敛自 main.rs 硬编码 `rgb(0x1f3a5f)`）
    pub search_hit_bg: Hsla,
    /// 选中边框（收敛自 main.rs `selection_border` 硬编码 `rgb(0xf2f5f8)`）
    pub selection_border: Hsla,
    /// 命中高亮边框（收敛自 main.rs `selection_border` 硬编码 `rgb(0xd98a3d)`）
    pub selection_highlight_border: Hsla,

    // 2. 节点卡片表面层级 (Node Card Surfaces)
    pub card_bg: Hsla,
    pub card_selected_bg: Hsla,
    pub card_neighbor_bg: Hsla,
    pub card_shadow: Hsla,
    pub card_border_default: Hsla,
    pub card_border_selected: Hsla,
    pub card_border_search_match: Hsla,

    // 3. 文本层级 (Typography)
    pub text_primary: Hsla,
    pub text_secondary: Hsla,
    pub text_muted: Hsla,

    // 4. 边与架构违规信号 (Edges & Architectural Risk Signals)
    pub edge_normal: Hsla,
    pub edge_strong: Hsla,
    /// `EdgeLayout::direction_adjusted == true`（逆向跨层调用/架构违规边）告警色
    pub edge_direction_adjusted: Hsla,
    pub port_default: Hsla,

    // 5. 几何尺寸常量 (Geometry Tokens)
    pub grid_spacing: Pixels,
    pub band_radius: Pixels,
    pub card_radius: Pixels,
    pub accent_bar_width: Pixels,
    pub non_neighbor_dim_factor: f32,
}

impl Global for StrataTheme {}

impl Default for StrataTheme {
    fn default() -> Self {
        Self::obsidian_dark()
    }
}

impl StrataTheme {
    /// 构建与 `view::CANVAS_BACKGROUND (0x0d1117)` 基准对齐的黑曜石暗色主题
    pub fn obsidian_dark() -> Self {
        Self {
            canvas_bg: rgb(CANVAS_BACKGROUND).into(),
            dot_grid: rgba(0x262c3670).into(),
            dot_grid_major: rgba(0x38415299).into(),
            hud_panel_bg: rgba(0x161b22e6).into(),
            hud_border: rgb(0x30363d).into(),
            search_hit_bg: rgb(0x1f3a5f).into(),
            selection_border: rgb(0xf2f5f8).into(),
            selection_highlight_border: rgb(0xd98a3d).into(),

            card_bg: rgb(0x161b22).into(),
            card_selected_bg: rgb(0x1f2937).into(),
            card_neighbor_bg: rgb(0x19212c).into(),
            card_shadow: rgba(0x00000078).into(),
            card_border_default: rgb(0x30363d).into(),
            card_border_selected: rgb(0x58a6ff).into(),
            card_border_search_match: rgb(0xf59e0b).into(),

            text_primary: rgb(NODE_LABEL).into(),
            text_secondary: rgb(0x8b949e).into(),
            text_muted: rgb(0x6e7681).into(),

            edge_normal: rgb(0x484f58).into(),
            edge_strong: rgb(0x58a6ff).into(),
            edge_direction_adjusted: rgb(0xf85149).into(),
            port_default: rgb(0x6e7681).into(),

            grid_spacing: px(24.0),
            band_radius: px(8.0),
            card_radius: px(6.0),
            accent_bar_width: px(3.0),
            non_neighbor_dim_factor: 0.22,
        }
    }

    /// 在 `main.rs` 启动窗口前注册全局主题：`StrataTheme::init_global(cx);`
    pub fn init_global(cx: &mut App) {
        cx.set_global(Self::obsidian_dark());
    }

    /// 将 `0xRRGGBB` 整数色值与不透明度 `alpha (0.0..=1.0)` 转换为 GPUI `Hsla`
    #[inline]
    pub fn hex_with_alpha(hex_rgb: u32, alpha: f32) -> Hsla {
        let mut color: Hsla = rgb(hex_rgb).into();
        color.a = alpha.clamp(0.0, 1.0);
        color
    }

    /// 解析架构分层（`LAYER_ORDER`: `ui`/`app`/`domain`/`infra`/`unclassified`/`external`
    /// 以及 `view::layer_palette` 中的细分模块层）对应的语义高光色
    pub fn layer_accent(&self, layer: &str, is_unclassified: bool, is_external: bool) -> Hsla {
        if is_unclassified {
            return rgb(0xd29922).into(); // 未分类模块：琥珀黄提醒
        }
        if is_external {
            return rgb(0x39c5cf).into(); // 外部依赖包：青色标识
        }
        match layer {
            "ui" | "components" => rgb(0xa371f7).into(), // 表现层：紫罗兰
            "app" | "commands" | "cli" => rgb(0x58a6ff).into(), // 应用/命令层：电光蓝
            "domain" => rgb(0x3fb950).into(),            // 领域核心层：翠绿
            "infra" | "server" | "utils" => rgb(0xd29922).into(), // 基础设施层：暖金
            "tests" => rgb(0xf778ba).into(),             // 测试层：珊瑚粉
            _ => {
                // 回退复用 `view::layer_palette` 既有调色板并提亮明度
                let base_hex = layer_palette(layer, is_unclassified);
                let mut hsla: Hsla = rgb(base_hex).into();
                hsla.l = (hsla.l + 0.22).min(0.72);
                hsla
            }
        }
    }

    /// 解析 `BandLayout`（分层泳道）的底色与边框色，兼容 `view::band_background_palette`
    pub fn band_style(
        &self,
        layer: &str,
        is_unclassified: bool,
        is_external: bool,
        reveal_alpha: f32,
    ) -> (Hsla, Hsla, Hsla) {
        let bg_hex = band_background_palette(is_unclassified, is_external);
        let fill_color = Self::hex_with_alpha(bg_hex, (0.78 * reveal_alpha).clamp(0.0, 1.0));
        let accent = self.layer_accent(layer, is_unclassified, is_external);

        let mut border_color = accent;
        border_color.a = (0.28 * reveal_alpha).clamp(0.0, 1.0);

        let mut header_color = accent;
        header_color.a = (0.90 * reveal_alpha).clamp(0.0, 1.0);

        (fill_color, border_color, header_color)
    }

    /// 解析宏观模块节点（`NodeLayout`）的卡片视觉样式
    #[allow(clippy::too_many_arguments)]
    pub fn resolve_module_node_style(
        &self,
        node: &NodeLayout,
        is_selected: bool,
        is_highlighted_neighbor: bool,
        has_any_selection: bool,
        is_search_match: bool,
        reveal_alpha: f32,
    ) -> NodeCardStyle {
        // 若当前有选中节点或搜索过滤，且本节点不在 1-Hop 高亮集内，则压暗至 `non_neighbor_dim_factor`
        let focus_factor = if has_any_selection && !is_selected && !is_highlighted_neighbor {
            self.non_neighbor_dim_factor
        } else {
            1.0
        };
        let alpha = (reveal_alpha * focus_factor).clamp(0.05, 1.0);

        let accent = self.layer_accent(node.layer, node.is_unclassified, node.is_external);
        let mut accent_bar = accent;
        accent_bar.a = alpha;

        // 表面背景：将 `view::layer_palette` 的深色底调与黑曜石卡片底色融合
        let palette_hex = layer_palette(node.layer, node.is_unclassified);
        let mut base_bg = if is_selected {
            self.card_selected_bg
        } else if is_highlighted_neighbor {
            self.card_neighbor_bg
        } else {
            let mut tint: Hsla = rgb(palette_hex).into();
            tint.s = (tint.s * 0.65).clamp(0.0, 1.0);
            tint.l = (tint.l * 0.55).clamp(0.08, 0.18);
            tint
        };
        base_bg.a = alpha;

        // 边框优先级：选中态 (电光蓝) > 搜索命中 (琥珀色) > 1-Hop 邻居 > 默认细边框
        let (mut border, border_width) = if is_selected {
            (self.card_border_selected, px(1.8))
        } else if is_search_match {
            (self.card_border_search_match, px(1.6))
        } else if is_highlighted_neighbor {
            let mut b = accent;
            b.a = 0.65;
            (b, px(1.25))
        } else {
            (self.card_border_default, px(1.0))
        };
        border.a *= alpha;

        let mut badge_bg = accent;
        badge_bg.a = 0.18 * alpha;

        let mut label_color = self.text_primary;
        label_color.a = alpha;

        let mut sublabel_color = self.text_secondary;
        sublabel_color.a = alpha * 0.85;

        let mut shadow = self.card_shadow;
        shadow.a *= focus_factor * reveal_alpha;

        let mut port_fill = if is_selected || is_highlighted_neighbor {
            accent
        } else {
            self.port_default
        };
        port_fill.a = alpha;

        NodeCardStyle {
            bg: base_bg,
            border,
            border_width,
            accent_bar,
            badge_bg,
            badge_fg: accent_bar,
            label_color,
            sublabel_color,
            shadow,
            port_fill,
        }
    }

    /// 解析下钻视图中单个 AST 符号节点（`SymbolNodeLayout`）的卡片样式，兼容 `view::symbol_kind_palette`
    pub fn resolve_symbol_node_style(
        &self,
        node: &SymbolNodeLayout,
        is_selected: bool,
        is_caller: bool,
        is_callee: bool,
        has_focus: bool,
    ) -> NodeCardStyle {
        let in_neighborhood = !has_focus || is_selected || is_caller || is_callee;
        let alpha = if in_neighborhood {
            1.0
        } else {
            self.non_neighbor_dim_factor
        };

        let accent = match node.kind {
            "function" => rgb(0x58a6ff).into(),
            "component" => rgb(0xa371f7).into(),
            "class" => rgb(0xf0883e).into(),
            "method" => rgb(0x3fb950).into(),
            _ => {
                let hex = symbol_kind_palette(node.kind);
                let mut c: Hsla = rgb(hex).into();
                c.l = (c.l + 0.25).min(0.75);
                c
            }
        };

        let mut accent_bar = accent;
        accent_bar.a = alpha;

        let base_hex = symbol_kind_palette(node.kind);
        let mut bg: Hsla = if is_selected {
            self.card_selected_bg
        } else {
            let mut c: Hsla = rgb(base_hex).into();
            c.l = (c.l * 0.52).clamp(0.09, 0.16);
            c
        };
        bg.a = alpha;

        let (mut border, border_width) = if is_selected {
            (self.card_border_selected, px(1.8))
        } else if is_caller {
            // 上游 Caller：用翠绿描边提示“谁调了我”
            (rgb(0x3fb950).into(), px(1.4))
        } else if is_callee {
            // 下游 Callee：用电光蓝描边提示“我调了谁”
            (rgb(0x58a6ff).into(), px(1.4))
        } else {
            (self.card_border_default, px(1.0))
        };
        border.a *= alpha;

        let mut badge_bg = accent;
        badge_bg.a = 0.20 * alpha;

        let mut label_color = self.text_primary;
        label_color.a = alpha;

        let mut sublabel_color = self.text_secondary;
        sublabel_color.a = alpha * 0.85;

        let mut shadow = self.card_shadow;
        shadow.a *= alpha;

        let mut port_fill = accent;
        port_fill.a = alpha;

        NodeCardStyle {
            bg,
            border,
            border_width,
            accent_bar,
            badge_bg,
            badge_fg: accent_bar,
            label_color,
            sublabel_color,
            shadow,
            port_fill,
        }
    }

    /// 解析 `EdgeVisual` / `EdgeRun` / `PaintedSymbolEdge` 的颜色与线宽
    /// - `adjusted == true`：代表 `arch-core` 检测到逆向跨层依赖（`direction_adjusted`），用醒目的珊瑚红警示
    /// - `strong == true`：代表当前选中节点关联的活跃调用边，用电光蓝加粗展示
    pub fn resolve_edge_style(
        &self,
        strong: bool,
        adjusted: bool,
        opacity: f32,
        zoom: f32,
    ) -> (Hsla, Pixels) {
        let mut color = if adjusted {
            self.edge_direction_adjusted
        } else if strong {
            self.edge_strong
        } else {
            self.edge_normal
        };
        color.a = opacity.clamp(0.05, 1.0);

        let base_w = if strong {
            2.15
        } else if adjusted {
            1.75
        } else {
            1.25
        };
        let width = px(base_w * zoom.clamp(0.65, 1.35));
        (color, width)
    }

    /// 解析 `DiffView` (`crates/arch-view/src/diff.rs`) 中每一行的卡片底色、左侧色条与文字颜色
    pub fn resolve_diff_row_colors(&self, kind: DiffKind, fade_alpha: f32) -> (Hsla, Hsla, Hsla) {
        let bg = Self::hex_with_alpha(kind.tint(), (0.28 * fade_alpha).clamp(0.0, 1.0));
        let accent_hex = match kind {
            DiffKind::Added => 0x3fb950,
            DiffKind::Removed => 0xf85149,
            DiffKind::Modified => 0xd29922,
        };
        let accent = Self::hex_with_alpha(accent_hex, fade_alpha);
        let text = Self::hex_with_alpha(DIFF_LABEL, fade_alpha);
        let _ = (DIFF_ADDED_TINT, DIFF_REMOVED_TINT, DIFF_MODIFIED_TINT);
        (bg, accent, text)
    }
}
