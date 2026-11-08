//! ticket 07 路径播放的纯状态机。
//!
//! 追光、暂停、前进/后退、静态完整路径、帧调度与 fly-to 都在此模型上计算；
//! 真实渲染帧时间依赖硬件与驱动，本模型只提供逐帧更新的耗时归档。

use crate::layout::{Point, Rect};
use arch_core::guided_path::GuidedPathArtifact;
use std::collections::{BTreeMap, BTreeSet};

pub const PATH_OPACITY: f32 = 1.0;
/// 当前步节点的透明度；与路径节点同值，但层级最高。
pub const CURRENT_OPACITY: f32 = 1.0;
/// 非路径元素（上下文）的淡化下限。
///
/// 依据：`div.opacity()` 在 GPUI 中按 primitive 乘 alpha，标签与节点底色同时淡化。
/// 在 0.68 时，全部调色板的标签对比度实测最低约 5.2:1，高于 WCAG AA 正文 4.5:1；
/// 旧值 0.25 实测约 2.1:1，不可辨认。低于 0.54 会跌破 4.5:1，0.68 留出约 15% 余量。
pub const CONTEXT_OPACITY: f32 = 0.68;
pub const NON_PATH_OPACITY: f32 = 0.25;

/// 把透明度换算成 8 位 alpha，保证绘制层逐帧取值确定。
pub fn opacity_alpha_byte(opacity: f32) -> u8 {
    (opacity.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// 淡出态（动画开启）的视觉角色；层级 `Context < Path < Current`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum VisualRole {
    Context,
    Path,
    Current,
}

impl VisualRole {
    pub fn opacity(self) -> f32 {
        match self {
            VisualRole::Current | VisualRole::Path => PATH_OPACITY,
            VisualRole::Context => CONTEXT_OPACITY,
        }
    }

    pub fn rank(self) -> u8 {
        match self {
            VisualRole::Context => 0,
            VisualRole::Path => 1,
            VisualRole::Current => 2,
        }
    }
}

/// 边的视觉角色；只有当前步的 incoming/outgoing 边被强调。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeRole {
    Context,
    Path,
    Current,
}

impl EdgeRole {
    pub fn opacity(self) -> f32 {
        match self {
            EdgeRole::Current | EdgeRole::Path => PATH_OPACITY,
            EdgeRole::Context => CONTEXT_OPACITY,
        }
    }

    pub fn strong(self) -> bool {
        self == EdgeRole::Current
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybackStep {
    pub module: String,
    pub symbol: String,
    pub layer: &'static str,
    pub via: Option<String>,
}

pub struct PlaybackState {
    steps: Vec<PlaybackStep>,
    current: usize,
    playing: bool,
    animation_enabled: bool,
    fly_to: bool,
    frame_requests: u64,
}

impl PlaybackState {
    pub fn new(path: &GuidedPathArtifact) -> Self {
        Self {
            steps: path
                .steps
                .iter()
                .map(|step| PlaybackStep {
                    module: step.module.clone(),
                    symbol: step.symbol.clone(),
                    layer: step.layer,
                    via: step.via.clone(),
                })
                .collect(),
            current: 0,
            playing: false,
            animation_enabled: false,
            fly_to: false,
            frame_requests: 0,
        }
    }

    pub fn steps(&self) -> &[PlaybackStep] {
        &self.steps
    }

    /// 已高亮的步数：`0` 表示尚未开始，`len` 表示完整路径。
    pub fn current(&self) -> usize {
        self.current
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    pub fn animation_enabled(&self) -> bool {
        self.animation_enabled
    }

    pub fn fly_to_enabled(&self) -> bool {
        self.fly_to
    }

    pub fn frame_requests(&self) -> u64 {
        self.frame_requests
    }

    /// 动画关闭时路径以完整静态形式可见，且不再调度动画帧。
    pub fn static_full_path(&self) -> bool {
        !self.animation_enabled
    }

    pub fn needs_animation_frame(&self) -> bool {
        self.playing && self.animation_enabled
    }

    pub fn play(&mut self) {
        self.current = 0;
        self.playing = true;
    }

    pub fn pause(&mut self) {
        self.playing = false;
    }

    /// 前进一拍；到末尾返回 `false` 并停止，不循环。
    pub fn advance(&mut self) -> bool {
        if self.current >= self.steps.len() {
            self.playing = false;
            return false;
        }
        self.current += 1;
        if self.current == self.steps.len() {
            self.playing = false;
        }
        true
    }

    /// 后退一拍；第 0 步返回 `false`。
    pub fn back(&mut self) -> bool {
        if self.current == 0 {
            return false;
        }
        self.current -= 1;
        self.playing = false;
        true
    }

    pub fn set_animation_enabled(&mut self, enabled: bool) {
        self.animation_enabled = enabled;
    }

    pub fn set_fly_to(&mut self, enabled: bool) {
        self.fly_to = enabled;
    }

    /// 请求一帧；动画关闭或未播放时不计数。
    pub fn request_frame(&mut self) -> bool {
        if !self.needs_animation_frame() {
            return false;
        }
        self.frame_requests += 1;
        true
    }

    pub fn highlighted_nodes(&self) -> BTreeSet<String> {
        self.steps[..self.current]
            .iter()
            .map(|step| step.symbol.clone())
            .collect()
    }

    pub fn highlighted_edges(&self) -> BTreeSet<(String, String)> {
        self.steps[..self.current]
            .windows(2)
            .map(|pair| (pair[0].symbol.clone(), pair[1].symbol.clone()))
            .collect()
    }

    /// 进入当前步的那条边；第 1 步没有入边。
    ///
    /// 保留给关闭态（回归判据）：旧绘制只强调 incoming。
    pub fn current_edge(&self) -> Option<(String, String)> {
        if self.current < 2 {
            return None;
        }
        Some((
            self.steps[self.current - 2].symbol.clone(),
            self.steps[self.current - 1].symbol.clone(),
        ))
    }

    /// 当前步的符号；尚未开始时为 `None`。
    pub fn current_symbol(&self) -> Option<&str> {
        self.current
            .checked_sub(1)
            .and_then(|index| self.steps.get(index))
            .map(|step| step.symbol.as_str())
    }

    /// 当前步所属模块；尚未开始时为 `None`。模块级与符号级视图共用此索引。
    pub fn current_module(&self) -> Option<&str> {
        self.current
            .checked_sub(1)
            .and_then(|index| self.steps.get(index))
            .map(|step| step.module.as_str())
    }

    fn role_of(symbol: &str, current: Option<&str>, steps: &[PlaybackStep]) -> VisualRole {
        if current == Some(symbol) {
            VisualRole::Current
        } else if steps.iter().any(|step| step.symbol == symbol) {
            VisualRole::Path
        } else {
            VisualRole::Context
        }
    }

    /// 节点角色：当前步 `Current`、其余路径节点 `Path`、非路径节点 `Context`。
    pub fn node_role(&self, symbol: &str) -> VisualRole {
        Self::role_of(symbol, self.current_symbol(), &self.steps)
    }

    pub fn node_context_opacity(&self, symbol: &str) -> f32 {
        self.node_role(symbol).opacity()
    }

    /// 路径上全部相邻的符号边。
    pub fn path_edges(&self) -> BTreeSet<(String, String)> {
        self.steps
            .windows(2)
            .map(|pair| (pair[0].symbol.clone(), pair[1].symbol.clone()))
            .collect()
    }

    /// 当前步的 incoming 与 outgoing 边；第 1 步只有 outgoing，末步只有 incoming。
    pub fn current_edges(&self) -> BTreeSet<(String, String)> {
        let mut edges = BTreeSet::new();
        if self.current >= 2 {
            edges.insert((
                self.steps[self.current - 2].symbol.clone(),
                self.steps[self.current - 1].symbol.clone(),
            ));
        }
        if self.current >= 1 && self.current < self.steps.len() {
            edges.insert((
                self.steps[self.current - 1].symbol.clone(),
                self.steps[self.current].symbol.clone(),
            ));
        }
        edges
    }

    pub fn edge_role(&self, source: &str, target: &str) -> EdgeRole {
        let pair = (source.to_string(), target.to_string());
        if self.current_edges().contains(&pair) {
            EdgeRole::Current
        } else if self.path_edges().contains(&pair) {
            EdgeRole::Path
        } else {
            EdgeRole::Context
        }
    }

    pub fn edge_context_opacity(&self, source: &str, target: &str) -> f32 {
        self.edge_role(source, target).opacity()
    }

    /// 模块路径边：相邻步骤跨模块时产生的边；同模块相邻对跳过（避免零长度边）。
    pub fn module_path_edges(&self) -> BTreeSet<(String, String)> {
        self.steps
            .windows(2)
            .filter(|pair| pair[0].module != pair[1].module)
            .map(|pair| (pair[0].module.clone(), pair[1].module.clone()))
            .collect()
    }

    pub fn current_module_edges(&self) -> BTreeSet<(String, String)> {
        let mut edges = BTreeSet::new();
        if self.current >= 2 {
            let source = &self.steps[self.current - 2].module;
            let target = &self.steps[self.current - 1].module;
            if source != target {
                edges.insert((source.clone(), target.clone()));
            }
        }
        if self.current >= 1 && self.current < self.steps.len() {
            let source = &self.steps[self.current - 1].module;
            let target = &self.steps[self.current].module;
            if source != target {
                edges.insert((source.clone(), target.clone()));
            }
        }
        edges
    }

    pub fn module_role(&self, module: &str) -> VisualRole {
        if self.current_module() == Some(module) {
            VisualRole::Current
        } else if self.steps.iter().any(|step| step.module == module) {
            VisualRole::Path
        } else {
            VisualRole::Context
        }
    }

    pub fn module_context_opacity(&self, module: &str) -> f32 {
        self.module_role(module).opacity()
    }

    pub fn module_edge_role(&self, source: &str, target: &str) -> EdgeRole {
        let pair = (source.to_string(), target.to_string());
        if self.current_module_edges().contains(&pair) {
            EdgeRole::Current
        } else if self.module_path_edges().contains(&pair) {
            EdgeRole::Path
        } else {
            EdgeRole::Context
        }
    }

    /// 绘制顺序：Context → Path → Current；同角色保持传入顺序（稳定排序）。
    pub fn paint_order(&self, symbols: &[&str]) -> Vec<usize> {
        let mut order = (0..symbols.len()).collect::<Vec<_>>();
        order.sort_by_key(|index| self.node_role(symbols[*index]).rank());
        order
    }

    pub fn module_paint_order(&self, modules: &[&str]) -> Vec<usize> {
        let mut order = (0..modules.len()).collect::<Vec<_>>();
        order.sort_by_key(|index| self.module_role(modules[*index]).rank());
        order
    }

    /// 非路径节点淡出，路径节点保持可见。
    pub fn node_opacity(&self, symbol: &str) -> f32 {
        if self.steps.iter().any(|step| step.symbol == symbol) {
            PATH_OPACITY
        } else {
            NON_PATH_OPACITY
        }
    }

    /// fly-to 开启且给定节点矩形时，镜头中心取当前步骤节点中心。
    pub fn camera_center(&self, rects: &BTreeMap<String, Rect>) -> Option<Point> {
        if !self.fly_to {
            return None;
        }
        let step = if self.current > 0 {
            &self.steps[self.current - 1]
        } else {
            self.steps.first()?
        };
        rects.get(&step.symbol).map(Rect::center)
    }
}
