//! ticket 07 路径播放的纯状态机。
//!
//! 追光、暂停、前进/后退、静态完整路径、帧调度与 fly-to 都在此模型上计算；
//! 真实渲染帧时间依赖硬件与驱动，本模型只提供逐帧更新的耗时归档。

use crate::layout::{Point, Rect};
use arch_core::guided_path::GuidedPathArtifact;
use std::collections::{BTreeMap, BTreeSet};

pub const PATH_OPACITY: f32 = 1.0;
pub const NON_PATH_OPACITY: f32 = 0.25;

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
    pub fn current_edge(&self) -> Option<(String, String)> {
        if self.current < 2 {
            return None;
        }
        Some((
            self.steps[self.current - 2].symbol.clone(),
            self.steps[self.current - 1].symbol.clone(),
        ))
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
