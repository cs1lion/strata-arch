//! ticket 05 画布交互的纯状态模型。
//!
//! 平移、缩放、命中测试、下钻与返回、邻居高亮、LOD 阈值都在此模型上计算并断言；
//! GPUI 只把模型画出来并转发事件，不参与状态判断。

use crate::layout::{MapLayout, Point, Rect};
use arch_core::callgraph::CallGraphArtifact;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

pub const MIN_ZOOM: f32 = 0.25;
pub const MAX_ZOOM: f32 = 4.0;
pub const LABEL_ZOOM_THRESHOLD: f32 = 0.6;

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
}

/// 把符号边映射到屏幕坐标并决定是否强调「当前边」。
///
/// 这是 `main.rs` 符号场景的唯一绘制入口：只有与 `current_edge` 完全一致的边
/// 才标记 `strong`，其余边保持普通样式。
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
        self.viewport.pan_by(dx, dy);
    }

    pub fn zoom_at(&mut self, anchor: Point, factor: f32) {
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
