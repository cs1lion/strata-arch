//! ticket 05 画布交互的纯状态模型。
//!
//! 平移、缩放、命中测试、下钻与返回、邻居高亮、LOD 阈值都在此模型上计算并断言；
//! GPUI 只把模型画出来并转发事件，不参与状态判断。

use crate::layout::{MapLayout, Point, Rect};
use crate::playback::{EdgeRole, PlaybackState};
use arch_core::callgraph::CallGraphArtifact;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

pub const MIN_ZOOM: f32 = 0.25;
pub const MAX_ZOOM: f32 = 4.0;
pub const LABEL_ZOOM_THRESHOLD: f32 = 0.6;

/// 画布底色；标签淡化对比度以此为背景实测。
pub const CANVAS_BACKGROUND: u32 = 0x0d1117;
/// 节点标签颜色。
pub const NODE_LABEL: u32 = 0xe6edf3;

/// 符号种类底色。关闭态与淡出态共用同一张表。
pub fn symbol_kind_palette(kind: &str) -> u32 {
    match kind {
        "function" => 0x244a66,
        "component" => 0x4a3768,
        "class" => 0x5f4524,
        "method" => 0x23543d,
        _ => 0x2a3440,
    }
}

/// 模块层底色。
pub fn layer_palette(layer: &str, is_unclassified: bool) -> u32 {
    if is_unclassified {
        return 0x3a2f22;
    }
    match layer {
        "cli" => 0x244a66,
        "commands" => 0x23543d,
        "components" => 0x4a3768,
        "server" => 0x5f4524,
        "utils" => 0x2f4652,
        "tests" => 0x53303c,
        "external" => 0x243b3d,
        _ => 0x2a3440,
    }
}

/// 模块节点所在的层带底色；模块节点的淡化叠加在它上面。
pub fn band_background_palette(is_unclassified: bool, is_external: bool) -> u32 {
    if is_unclassified {
        0x1f1a12
    } else if is_external {
        0x101d21
    } else {
        0x11161c
    }
}

pub const SYMBOL_CANVAS_WIDTH: f32 = 1200.0;
pub const SYMBOL_CANVAS_HEIGHT: f32 = 800.0;
pub const SYMBOL_PADDING: f32 = 20.0;
pub const SYMBOL_NODE_WIDTH: f32 = 220.0;
pub const SYMBOL_NODE_HEIGHT: f32 = 44.0;
pub const SYMBOL_GAP_X: f32 = 16.0;
pub const SYMBOL_GAP_Y: f32 = 12.0;

/// 视口：`screen = world * zoom + pan`。
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Viewport {
    pub pan_x: f32,
    pub pan_y: f32,
    pub zoom: f32,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            pan_x: 0.0,
            pan_y: 0.0,
            zoom: 1.0,
        }
    }
}

impl Viewport {
    pub fn screen_point(&self, world: Point) -> Point {
        Point {
            x: world.x * self.zoom + self.pan_x,
            y: world.y * self.zoom + self.pan_y,
        }
    }

    pub fn world_point(&self, screen: Point) -> Point {
        Point {
            x: (screen.x - self.pan_x) / self.zoom,
            y: (screen.y - self.pan_y) / self.zoom,
        }
    }

    pub fn pan_by(&mut self, dx: f32, dy: f32) {
        self.pan_x += dx;
        self.pan_y += dy;
    }

    /// 以屏幕坐标 `anchor` 为不动点缩放；缩放钳制在 `[MIN_ZOOM, MAX_ZOOM]`。
    pub fn zoom_at(&mut self, anchor: Point, factor: f32) {
        let next = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        if (next - self.zoom).abs() < f32::EPSILON {
            return;
        }
        let world = self.world_point(anchor);
        self.zoom = next;
        self.pan_x = anchor.x - world.x * next;
        self.pan_y = anchor.y - world.y * next;
    }
}

/// 短镜头转场：相机在 `from` 与 `to` 之间按 `progress` 线性插值。
///
/// `progress` 只由 UI 用 GPUI 的动画帧进度写入，模型自身不读墙钟。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraTransition {
    pub from: Viewport,
    pub to: Viewport,
    pub progress: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolNodeLayout {
    pub id: String,
    pub module: String,
    /// `function` / `component` / `class` / `method` / `module-toplevel`。
    pub kind: &'static str,
    pub name: String,
    pub rect: Rect,
    pub label_rect: Rect,
    pub hit_rect: Rect,
    pub z_index: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolEdgeLayout {
    pub source: String,
    pub target: String,
    pub kind: String,
    pub from: Point,
    pub to: Point,
    pub z_index: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolLayout {
    pub module: String,
    pub width: f32,
    pub height: f32,
    pub nodes: Vec<SymbolNodeLayout>,
    pub edges: Vec<SymbolEdgeLayout>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PaintedSymbolEdge {
    pub source: String,
    pub target: String,
    pub from: Point,
    pub to: Point,
    pub strong: bool,
    pub opacity: f32,
}

/// 把符号边映射到屏幕坐标并决定是否强调「当前边」。
///
/// 这是 `main.rs` 符号场景的唯一绘制入口：只有与 `current_edge` 完全一致的边
/// 才标记 `strong`，其余边保持普通样式。这是关闭开关时的旧路径（回归判据），
/// 所有边都不淡化。
pub fn paint_symbol_edges(
    layout: &SymbolLayout,
    viewport: &Viewport,
    current_edge: Option<&(String, String)>,
) -> Vec<PaintedSymbolEdge> {
    layout
        .edges
        .iter()
        .map(|edge| PaintedSymbolEdge {
            source: edge.source.clone(),
            target: edge.target.clone(),
            from: viewport.screen_point(edge.from),
            to: viewport.screen_point(edge.to),
            strong: current_edge
                .map(|(source, target)| source == &edge.source && target == &edge.target)
                .unwrap_or(false),
            opacity: crate::playback::PATH_OPACITY,
        })
        .collect()
}

/// 淡出态入口：当前步的 incoming/outgoing 边 `strong`，其余路径边保持可见，
/// 非路径边按 `CONTEXT_OPACITY` 淡化。
pub fn paint_symbol_edges_with_roles(
    layout: &SymbolLayout,
    viewport: &Viewport,
    playback: &PlaybackState,
) -> Vec<PaintedSymbolEdge> {
    layout
        .edges
        .iter()
        .map(|edge| {
            let role = playback.edge_role(&edge.source, &edge.target);
            PaintedSymbolEdge {
                source: edge.source.clone(),
                target: edge.target.clone(),
                from: viewport.screen_point(edge.from),
                to: viewport.screen_point(edge.to),
                strong: role == EdgeRole::Current,
                opacity: role.opacity(),
            }
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Level {
    Modules,
    Symbols { module: String },
}

#[derive(Debug, Clone, Copy)]
struct SavedView {
    viewport: Viewport,
}

pub struct CanvasState {
    map: MapLayout,
    symbols: Option<SymbolLayout>,
    neighbors: BTreeMap<String, BTreeSet<String>>,
    viewport: Viewport,
    selected: Option<String>,
    saved: Option<SavedView>,
    transition: Option<CameraTransition>,
    transition_enabled: bool,
}

impl CanvasState {
    pub fn new(map: MapLayout) -> Self {
        Self {
            map,
            symbols: None,
            neighbors: BTreeMap::new(),
            viewport: Viewport::default(),
            selected: None,
            saved: None,
            transition: None,
            transition_enabled: true,
        }
    }

    pub fn level(&self) -> Level {
        match &self.symbols {
            Some(layout) => Level::Symbols {
                module: layout.module.clone(),
            },
            None => Level::Modules,
        }
    }

    pub fn viewport(&self) -> Viewport {
        self.viewport
    }

    /// fly-to 接线点：把目标世界坐标移到视口中心。
    ///
    /// 只调整平移、不改缩放；目标为空或已在中心时返回 `false`。
    pub fn apply_fly_to(&mut self, target: Option<Point>, viewport_size: (f32, f32)) -> bool {
        let Some(target) = target else {
            return false;
        };
        let center = Point {
            x: viewport_size.0 / 2.0,
            y: viewport_size.1 / 2.0,
        };
        let zoom = self.viewport.zoom;
        let pan_x = center.x - target.x * zoom;
        let pan_y = center.y - target.y * zoom;
        if (pan_x - self.viewport.pan_x).abs() < f32::EPSILON
            && (pan_y - self.viewport.pan_y).abs() < f32::EPSILON
        {
            return false;
        }
        self.viewport.pan_x = pan_x;
        self.viewport.pan_y = pan_y;
        true
    }

    pub fn map(&self) -> &MapLayout {
        &self.map
    }

    pub fn transition_enabled(&self) -> bool {
        self.transition_enabled
    }

    /// 关闭转场时立即清掉进行中的转场，相机停在当前插值位置。
    pub fn set_transition_enabled(&mut self, enabled: bool) {
        self.transition_enabled = enabled;
        if !enabled {
            self.transition = None;
        }
    }

    pub fn transition(&self) -> Option<CameraTransition> {
        self.transition
    }

    /// 启动短镜头转场；返回 `true` 表示相机将按帧插值到目标中心。
    ///
    /// 目标为空、已在目标中心或转场关闭时不启动转场；转场关闭时直接复用
    /// `apply_fly_to` 的即时行为（回归判据）。
    pub fn begin_fly_to(&mut self, target: Option<Point>, viewport_size: (f32, f32)) -> bool {
        let Some(target) = target else {
            return false;
        };
        let center = Point {
            x: viewport_size.0 / 2.0,
            y: viewport_size.1 / 2.0,
        };
        let zoom = self.viewport.zoom;
        let to = Viewport {
            pan_x: center.x - target.x * zoom,
            pan_y: center.y - target.y * zoom,
            zoom,
        };
        if (to.pan_x - self.viewport.pan_x).abs() < f32::EPSILON
            && (to.pan_y - self.viewport.pan_y).abs() < f32::EPSILON
        {
            return false;
        }
        if !self.transition_enabled {
            self.apply_fly_to(Some(target), viewport_size);
            return false;
        }
        self.transition = Some(CameraTransition {
            from: self.viewport,
            to,
            progress: 0.0,
        });
        true
    }

    /// 写入 GPUI 动画帧给出的进度（`0..=1`）；相机同步到对应插值位置。
    ///
    /// `progress = 1` 时落在精确终点并清除转场状态。
    pub fn set_transition_progress(&mut self, progress: f32) {
        let Some(mut transition) = self.transition else {
            return;
        };
        let progress = progress.clamp(0.0, 1.0);
        transition.progress = progress;
        self.viewport = Viewport {
            pan_x: transition.from.pan_x + (transition.to.pan_x - transition.from.pan_x) * progress,
            pan_y: transition.from.pan_y + (transition.to.pan_y - transition.from.pan_y) * progress,
            zoom: transition.from.zoom,
        };
        if progress >= 1.0 {
            self.viewport = transition.to;
            self.transition = None;
        } else {
            self.transition = Some(transition);
        }
    }

    /// 中断转场并保留当前插值位置；返回是否确实中断了一次转场。
    pub fn cancel_transition(&mut self) -> bool {
        self.transition.take().is_some()
    }

    pub fn symbol_layout(&self) -> Option<&SymbolLayout> {
        self.symbols.as_ref()
    }

    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    pub fn can_go_back(&self) -> bool {
        self.saved.is_some()
    }

    pub fn labels_visible(&self) -> bool {
        self.viewport.zoom >= LABEL_ZOOM_THRESHOLD
    }

    /// 下钻到模块的符号视图；模块没有符号时返回 `false` 且状态不变。
    pub fn drill_down(&mut self, module: &str, artifact: &CallGraphArtifact) -> bool {
        if self.symbols.is_some() {
            return false;
        }
        let layout = build_symbol_layout(module, artifact);
        if layout.nodes.is_empty() {
            return false;
        }

        let mut neighbors = BTreeMap::new();
        let ids = layout
            .nodes
            .iter()
            .map(|node| node.id.as_str())
            .collect::<BTreeSet<_>>();
        for edge in &artifact.edges {
            if ids.contains(edge.source.as_str()) {
                neighbors
                    .entry(edge.source.clone())
                    .or_insert_with(BTreeSet::new)
                    .insert(edge.target.clone());
            }
            if ids.contains(edge.target.as_str()) {
                neighbors
                    .entry(edge.target.clone())
                    .or_insert_with(BTreeSet::new)
                    .insert(edge.source.clone());
            }
        }

        self.saved = Some(SavedView {
            viewport: self.viewport,
        });
        self.symbols = Some(layout);
        self.neighbors = neighbors;
        self.selected = None;
        self.transition = None;
        self.viewport = Viewport::default();
        true
    }

    /// 返回模块地图；恢复下钻前的平移与缩放。
    pub fn go_back(&mut self) -> bool {
        let Some(saved) = self.saved.take() else {
            return false;
        };
        self.symbols = None;
        self.neighbors.clear();
        self.selected = None;
        self.transition = None;
        self.viewport = saved.viewport;
        true
    }

    /// 命中测试使用屏幕坐标；空白处点击清空选择。
    pub fn select_at(&mut self, screen: Point) -> Option<String> {
        let world = self.viewport.world_point(screen);
        let hit = self
            .current_nodes()
            .into_iter()
            .filter(|(_, hit_rect, _)| hit_rect.contains(world))
            .max_by_key(|(_, _, z_index)| *z_index)
            .map(|(id, _, _)| id);
        self.selected = hit.clone();
        hit
    }

    pub fn highlighted(&self) -> BTreeSet<String> {
        match &self.selected {
            Some(selected) => self.neighbors.get(selected).cloned().unwrap_or_default(),
            None => BTreeSet::new(),
        }
    }

    pub fn pan_by(&mut self, dx: f32, dy: f32) {
        // 手动平移即中断转场：相机停在当前插值位置再叠加拖动。
        self.cancel_transition();
        self.viewport.pan_by(dx, dy);
    }

    pub fn zoom_at(&mut self, anchor: Point, factor: f32) {
        // 手动缩放即中断转场：相机停在当前插值位置再缩放。
        self.cancel_transition();
        self.viewport.zoom_at(anchor, factor);
    }

    fn current_nodes(&self) -> Vec<(String, Rect, i32)> {
        match &self.symbols {
            Some(layout) => layout
                .nodes
                .iter()
                .map(|node| (node.id.clone(), node.hit_rect, node.z_index))
                .collect(),
            None => self
                .map
                .nodes
                .iter()
                .map(|node| (node.id.clone(), node.hit_rect, node.z_index))
                .collect(),
        }
    }
}

fn build_symbol_layout(module: &str, artifact: &CallGraphArtifact) -> SymbolLayout {
    let columns = (((SYMBOL_CANVAS_WIDTH - 2.0 * SYMBOL_PADDING + SYMBOL_GAP_X)
        / (SYMBOL_NODE_WIDTH + SYMBOL_GAP_X))
        .floor()
        .max(1.0)) as usize;

    let mut nodes = Vec::new();
    let mut by_id = BTreeMap::new();
    let mut cursor_y = SYMBOL_PADDING;

    for symbol in artifact
        .symbols
        .iter()
        .filter(|symbol| symbol.module == module)
    {
        let index = nodes.len();
        let row = index / columns;
        let column = index % columns;
        let x = SYMBOL_PADDING + column as f32 * (SYMBOL_NODE_WIDTH + SYMBOL_GAP_X);
        let y = cursor_y + row as f32 * (SYMBOL_NODE_HEIGHT + SYMBOL_GAP_Y);
        let rect = Rect::new(x, y, SYMBOL_NODE_WIDTH, SYMBOL_NODE_HEIGHT);
        let label_rect = Rect::new(
            x + 6.0,
            y + 6.0,
            (SYMBOL_NODE_WIDTH - 12.0).max(0.0),
            (SYMBOL_NODE_HEIGHT - 12.0).max(0.0),
        );
        let z_index = 1000 + nodes.len() as i32;
        by_id.insert(symbol.id.as_str(), rect);
        nodes.push(SymbolNodeLayout {
            id: symbol.id.clone(),
            module: symbol.module.clone(),
            kind: symbol.kind,
            name: symbol.name.clone(),
            rect,
            label_rect,
            hit_rect: rect,
            z_index,
        });
    }

    let rows = nodes.len().div_ceil(columns.max(1));
    if rows > 0 {
        cursor_y += rows as f32 * SYMBOL_NODE_HEIGHT + rows.saturating_sub(1) as f32 * SYMBOL_GAP_Y;
    }

    let edges = artifact
        .edges
        .iter()
        .filter_map(|edge| {
            let source = by_id.get(edge.source.as_str())?;
            let target = by_id.get(edge.target.as_str())?;
            Some(SymbolEdgeLayout {
                source: edge.source.clone(),
                target: edge.target.clone(),
                kind: edge.kind.clone(),
                from: source.center(),
                to: target.center(),
                z_index: 1,
            })
        })
        .collect::<Vec<_>>();

    SymbolLayout {
        module: module.to_string(),
        width: SYMBOL_CANVAS_WIDTH,
        height: SYMBOL_CANVAS_HEIGHT.max(cursor_y + SYMBOL_PADDING),
        nodes,
        edges,
    }
}
