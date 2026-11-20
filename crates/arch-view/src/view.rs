//! ticket 05 画布交互的纯状态模型。
//!
//! 平移、缩放、命中测试、下钻与返回、邻居高亮、LOD 阈值都在此模型上计算并断言；
//! GPUI 只把模型画出来并转发事件，不参与状态判断。

use crate::layout::{MapLayout, NodeLayout, Point, Rect};
use crate::playback::{EdgeRole, PlaybackState};
use arch_core::callgraph::CallGraphArtifact;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

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

    /// 世界矩形映射到屏幕矩形（左上角变换 + 尺寸乘缩放）。
    pub fn screen_rect(&self, rect: Rect) -> Rect {
        let origin = self.screen_point(Point {
            x: rect.x,
            y: rect.y,
        });
        Rect::new(
            origin.x,
            origin.y,
            rect.width * self.zoom,
            rect.height * self.zoom,
        )
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

/// 视口裁剪的保守外扩像素：覆盖 1–2px 描边与抗锯齿，保证任何部分可见的元素不被剪掉。
pub const CULL_MARGIN: f32 = 2.0;

/// 屏幕矩形是否完全在视口外。只用于跳过绘制，不参与命中测试。
///
/// 判据是「完全在外」：只要与视口相交（含 `margin` 内）就返回 `false`，
/// 因此静止帧的可见像素应与不裁剪时完全一致。
pub fn rect_fully_outside(rect: Rect, viewport_size: (f32, f32), margin: f32) -> bool {
    rect.x > viewport_size.0 + margin
        || rect.x + rect.width < -margin
        || rect.y > viewport_size.1 + margin
        || rect.y + rect.height < -margin
}

/// 屏幕线段是否完全在视口外（按两端包围盒判定，等价于两端同在外侧）。
pub fn segment_fully_outside(
    from: Point,
    to: Point,
    viewport_size: (f32, f32),
    margin: f32,
) -> bool {
    let min_x = from.x.min(to.x);
    let max_x = from.x.max(to.x);
    let min_y = from.y.min(to.y);
    let max_y = from.y.max(to.y);
    max_x < -margin
        || min_x > viewport_size.0 + margin
        || max_y < -margin
        || min_y > viewport_size.1 + margin
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

/// 单条边的绘制几何与样式。A（批量绘制）只改变分组，不改变这里。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EdgeVisual {
    pub from: Point,
    pub to: Point,
    pub adjusted: bool,
    pub strong: bool,
    pub opacity: f32,
}

/// 一次 flush 的边分组；`edges` 是原始序列下标。
#[derive(Debug, Clone, PartialEq)]
pub struct EdgeRun {
    pub strong: bool,
    pub adjusted: bool,
    pub opacity: f32,
    pub edges: Vec<usize>,
}

/// 按原始顺序把连续同风格边分组。`batch_edges=false` 时每条边自成一组（逐边绘制）。
///
/// 几何等价判据：把任何分组模式展开后，必须与输入序列逐项相同。
pub fn group_edges(edges: &[EdgeVisual], batch_edges: bool) -> Vec<EdgeRun> {
    let mut runs: Vec<EdgeRun> = Vec::new();
    for (index, edge) in edges.iter().enumerate() {
        let mergeable = batch_edges && !edge.adjusted;
        let merge_into = mergeable
            && runs
                .last()
                .map(|run| {
                    run.strong == edge.strong
                        && run.adjusted == edge.adjusted
                        && (run.opacity - edge.opacity).abs() <= f32::EPSILON
                })
                .unwrap_or(false);
        if merge_into {
            if let Some(run) = runs.last_mut() {
                run.edges.push(index);
            }
        } else {
            runs.push(EdgeRun {
                strong: edge.strong,
                adjusted: edge.adjusted,
                opacity: edge.opacity,
                edges: vec![index],
            });
        }
    }
    runs
}

/// 节点的屏幕几何与标签排版锚点；base 与 A 共用同一计算。
#[derive(Debug, Clone, PartialEq)]
pub struct NodeVisualGeometry {
    pub rect: Rect,
    pub center: Point,
    pub size: (f32, f32),
    pub label: String,
    /// 标签排版锚点：节点左内边距 6px、垂直居中。
    pub label_anchor: Point,
}

pub fn node_visual_geometry(node: &NodeLayout, viewport: &Viewport) -> NodeVisualGeometry {
    let rect = viewport.screen_rect(node.rect);
    NodeVisualGeometry {
        rect,
        center: Point {
            x: rect.x + rect.width / 2.0,
            y: rect.y + rect.height / 2.0,
        },
        size: (rect.width, rect.height),
        label: node.id.clone(),
        label_anchor: Point {
            x: rect.x + 6.0,
            y: rect.y + rect.height / 2.0,
        },
    }
}

/// 一帧的几何：节点/标签按绘制顺序，边按分组展开后的原始顺序。
#[derive(Debug, Clone, PartialEq)]
pub struct FrameGeometry {
    pub nodes: Vec<NodeVisualGeometry>,
    pub edges: Vec<EdgeVisual>,
}

/// 生成可比较的帧几何；`batch_edges` 只影响边的分组，展开后必须与全量序列一致。
pub fn frame_geometry(map: &MapLayout, viewport: &Viewport, batch_edges: bool) -> FrameGeometry {
    let nodes = map
        .nodes
        .iter()
        .map(|node| node_visual_geometry(node, viewport))
        .collect();
    let edges = map
        .edges
        .iter()
        .map(|edge| EdgeVisual {
            from: viewport.screen_point(edge.from),
            to: viewport.screen_point(edge.to),
            adjusted: edge.direction_adjusted,
            strong: false,
            opacity: 1.0,
        })
        .collect::<Vec<_>>();
    let flattened = group_edges(&edges, batch_edges)
        .into_iter()
        .flat_map(|run| run.edges.into_iter().map(|index| edges[index]))
        .collect();
    FrameGeometry {
        nodes,
        edges: flattened,
    }
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
/// 自动启用边降噪的边数阈值：超过该数量时默认仅展示聚焦/选中相关边。
pub const EDGE_AUTO_FOCUS_THRESHOLD: usize = 200;

/// 模块视图下的边显示模式（用于大仓视觉降噪）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EdgeDisplayMode {
    /// 显示全部模块依赖边。
#[default]
All,
/// 仅显示与当前聚焦或选中模块相关的边。
FocusedOnly,
/// 隐藏普通模块依赖边（路径回放边仍保留）。
Hidden,
    }

impl EdgeDisplayMode {
    pub fn cycle(self) -> Self {
        match self {
            Self::All => Self::FocusedOnly,
            Self::FocusedOnly => Self::Hidden,
            Self::Hidden => Self::All,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "边: 全部",
            Self::FocusedOnly => "边: 仅聚焦",
            Self::Hidden => "边: 隐藏",
        }
    }
}

pub struct CanvasState {
    map: MapLayout,
    symbols: Option<SymbolLayout>,
    neighbors: BTreeMap<String, BTreeSet<String>>,
    viewport: Viewport,
    selected: Option<String>,
    hovered: Option<String>,
    saved: Option<SavedView>,
    transition: Option<CameraTransition>,
    transition_enabled: bool,
    edge_mode: EdgeDisplayMode,
}

impl CanvasState {
    pub fn new(map: MapLayout) -> Self {
        let edge_mode = if map.edges.len() > EDGE_AUTO_FOCUS_THRESHOLD {
            EdgeDisplayMode::FocusedOnly
        } else {
            EdgeDisplayMode::All
        };
        Self {
            map,
            symbols: None,
            neighbors: BTreeMap::new(),
            viewport: Viewport::default(),
            selected: None,
            hovered: None,
            saved: None,
            transition: None,
            transition_enabled: true,
            edge_mode,
        }
    }

    pub fn edge_mode(&self) -> EdgeDisplayMode {
        self.edge_mode
    }

    pub fn cycle_edge_mode(&mut self) -> EdgeDisplayMode {
        self.edge_mode = self.edge_mode.cycle();
        self.edge_mode
    }

    pub fn set_edge_mode(&mut self, mode: EdgeDisplayMode) {
        self.edge_mode = mode;
    }

    /// 以视口中心为锚点缩放，并取消进行中的相机转场。
    pub fn zoom_at_center(&mut self, factor: f32, viewport_size: (f32, f32)) {
        self.transition = None;
        let anchor = Point {
            x: viewport_size.0 * 0.5,
            y: viewport_size.1 * 0.5,
        };
        self.viewport.zoom_at(anchor, factor);
    }

    /// 将当前视图完整居中适配到视口内。
    pub fn fit_to_viewport(&mut self, viewport_size: (f32, f32)) {
        self.transition = None;
        let (content_w, content_h) = match &self.symbols {
            Some(symbols) => (symbols.width, symbols.height),
            None => (self.map.width, self.map.height),
        };
        if content_w <= 0.0
            || content_h <= 0.0
            || viewport_size.0 <= 0.0
            || viewport_size.1 <= 0.0
        {
            self.viewport = Viewport::default();
            return;
        }
        let margin = 48.0;
        let avail_w = (viewport_size.0 - margin * 2.0).max(64.0);
        let avail_h = (viewport_size.1 - margin * 2.0).max(64.0);
        let zoom = (avail_w / content_w)
            .min(avail_h / content_h)
            .clamp(MIN_ZOOM, 1.0);
        self.viewport = Viewport {
            pan_x: (viewport_size.0 - content_w * zoom) * 0.5,
            pan_y: (viewport_size.1 - content_h * zoom) * 0.5,
            zoom,
        };
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

    /// 证据抓帧用：直接设定相机；不参与交互逻辑，也不改变命中测试几何。
    pub fn set_viewport(&mut self, viewport: Viewport) {
        self.viewport = viewport;
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
        self.hovered = None;
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
        self.hovered = None;
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
        let Some(selected) = self.selected.as_deref() else {
            return BTreeSet::new();
        };
        if self.symbols.is_some() {
            return self.neighbors.get(selected).cloned().unwrap_or_default();
        }
        let mut set = BTreeSet::new();
        for edge in &self.map.edges {
            if edge.source == selected && edge.target != selected {
                set.insert(edge.target.clone());
            } else if edge.target == selected && edge.source != selected {
                set.insert(edge.source.clone());
            }
        }
        set
        
            
            
        
    }

    pub fn hovered(&self) -> Option<&str> {
        self.hovered.as_deref()
    }

    /// 命中测试更新悬停节点，仅在悬停目标变化时返回 `true`。
    pub fn hover_at(&mut self, screen: Point) -> bool {
        let world = self.viewport.world_point(screen);
        let next = self
            .current_nodes()
            .into_iter()
            .filter(|(_, hit_rect, _)| hit_rect.contains(world))
            .max_by_key(|(_, _, z_index)| *z_index)
            .map(|(id, _, _)| id);
        if self.hovered == next {
            false
        } else {
            self.hovered = next;
            true
        }
    }

    pub fn clear_selection(&mut self) -> bool {
        self.selected.take().is_some()
    }

    /// 按模块 ID 直接选中（仅模块视图生效），返回其世界坐标中心点以便相机居中。
    pub fn select_module(&mut self, id: &str) -> Option<Point> {
        if self.symbols.is_some() {
            return None;
        }
        let node = self.map.nodes.iter().find(|n| n.id == id)?;
        self.selected = Some(node.id.clone());
        Some(node.rect.center())
    }

    /// 返回模块视图下指定模块的 `(上游列表, 下游列表)`（去重、字典序、不含自身）。
    pub fn module_neighbors(&self, id: &str) -> (Vec<String>, Vec<String>) {
        let mut upstream = BTreeSet::new();
        let mut downstream = BTreeSet::new();
        for edge in &self.map.edges {
            if edge.target == id && edge.source != id {
                upstream.insert(edge.source.clone());
            }
            if edge.source == id && edge.target != id {
                downstream.insert(edge.target.clone());
            }
        }
        (
            upstream.into_iter().collect(),
            downstream.into_iter().collect(),
        )
    }

    /// 统计指定模块在调用图中的符号数量（用于决定是否允许下钻）。
    pub fn module_symbol_count(&self, module: &str, artifact: &CallGraphArtifact) -> usize {
        artifact
            .symbols
            .iter()
            .filter(|symbol| symbol.module == module)
            .count()
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

/// 在调用图中按 BFS 查找从 `from_module` 到 `to_module` 的最短可达符号对（完全确定性）。
pub fn find_reachable_symbol_pair(
    from_module: &str,
    to_module: &str,
    artifact: &CallGraphArtifact,
) -> Option<(String, String)> {
    let targets = artifact
        .symbols
        .iter()
        .filter(|symbol| symbol.module == to_module)
        .map(|symbol| symbol.id.as_str())
        .collect::<BTreeSet<_>>();
    if targets.is_empty() {
        return None;
    }

    let mut adj: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for edge in &artifact.edges {
        adj.entry(edge.source.as_str())
            .or_default()
            .push(edge.target.as_str());
    }
    for neighbors in adj.values_mut() {
        neighbors.sort_unstable();
        neighbors.dedup();
    }

    let mut starts = artifact
        .symbols
        .iter()
        .filter(|symbol| symbol.module == from_module)
        .map(|symbol| symbol.id.as_str())
        .collect::<Vec<_>>();
    starts.sort_unstable();
    starts.dedup();

    let mut visited = BTreeSet::new();
    let mut queue = VecDeque::new();
    for start in starts {
        if targets.contains(start) {
            return Some((start.to_string(), start.to_string()));
        }
        if visited.insert(start) {
            queue.push_back((start, start));
        }
    }

    while let Some((origin, current)) = queue.pop_front() {
        if let Some(neighbors) = adj.get(current) {
            for &next in neighbors {
                if targets.contains(next) {
                    return Some((origin.to_string(), next.to_string()));
                }
                if visited.insert(next) {
                    queue.push_back((origin, next));
                }
            }
        }
    }
    None
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edge_display_mode_cycles_in_order() {
        let mode = EdgeDisplayMode::default();
        assert_eq!(mode, EdgeDisplayMode::All);
        assert_eq!(mode.label(), "边: 全部");

        let focused = mode.cycle();
        assert_eq!(focused, EdgeDisplayMode::FocusedOnly);
        assert_eq!(focused.label(), "边: 仅聚焦");

        let hidden = focused.cycle();
        assert_eq!(hidden, EdgeDisplayMode::Hidden);
        assert_eq!(hidden.label(), "边: 隐藏");

        assert_eq!(hidden.cycle(), EdgeDisplayMode::All);
    }
}

