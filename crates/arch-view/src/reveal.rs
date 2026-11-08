//! P5 首屏分层揭示的纯状态机。
//!
//! 进度只由帧计数派生：调用方在每个 GPUI 帧回调里调用一次 `advance_frame()`，
//! 本模块不读墙钟、不用随机源、不注册定时器。揭示一次性：播完停在最终态，
//! 不循环、不呼吸、不自动重播；任何交互可调用 `skip()` 立即落到最终态。

/// 单个层带的淡入帧数（60fps 下约 167ms）。
pub const REVEAL_FADE_FRAMES: u32 = 10;
/// 相邻层带之间的隔帧数，让「层序」可被看清而不是同时出现。
pub const REVEAL_GAP_FRAMES: u32 = 4;
/// 「升起」的起始下沉像素；最终态位移为 0。
pub const REVEAL_RISE_PX: f32 = 12.0;

#[derive(Debug, Clone, PartialEq)]
pub struct RevealState {
    enabled: bool,
    started: bool,
    finished: bool,
    stages: usize,
    frames: u32,
}

impl RevealState {
    /// `enabled=false` 即默认关：等价于已结束的最终态，不请求任何帧。
    pub fn new(enabled: bool, stages: usize) -> Self {
        if enabled && stages > 0 {
            Self {
                enabled: true,
                started: false,
                finished: false,
                stages,
                frames: 0,
            }
        } else {
            Self {
                enabled: false,
                started: false,
                finished: true,
                stages,
                frames: 0,
            }
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// 已启用、数据尚未就绪；此时不得推进，内容保持最终可见态。
    pub fn is_pending(&self) -> bool {
        self.enabled && !self.started && !self.finished
    }

    /// 正在揭示：调用方需要续订下一帧。
    pub fn is_active(&self) -> bool {
        self.enabled && self.started && !self.finished
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }

    pub fn stages(&self) -> usize {
        self.stages
    }

    pub fn frames(&self) -> u32 {
        self.frames
    }

    pub fn total_frames(&self) -> u32 {
        if self.stages == 0 {
            0
        } else {
            let stages = self.stages as u32;
            stages * REVEAL_FADE_FRAMES + (stages - 1) * REVEAL_GAP_FRAMES
        }
    }

    /// 数据就绪后调用一次；只有启用且未开始时生效，重复调用无操作。
    pub fn start(&mut self) -> bool {
        if !self.enabled || self.started || self.finished || self.stages == 0 {
            return false;
        }
        self.started = true;
        self.frames = 0;
        true
    }

    /// 推进一帧；返回推进后是否仍在揭示中。播完即冻结，不回绕。
    pub fn advance_frame(&mut self) -> bool {
        if !self.is_active() {
            return false;
        }
        let total = self.total_frames();
        self.frames = self.frames.saturating_add(1);
        if self.frames >= total {
            self.frames = total;
            self.finished = true;
        }
        self.is_active()
    }

    /// 交互打断：立即落到最终态；返回是否确实中断了一次进行中的揭示。
    pub fn skip(&mut self) -> bool {
        if !self.enabled || self.finished {
            return false;
        }
        self.started = true;
        self.frames = self.total_frames();
        self.finished = true;
        true
    }

    /// 第 `stage` 层（按传入的层序）的进度 `0..=1`。
    ///
    /// `pending` 与关闭态返回 1.0：数据未就绪或未启用时不得让内容消失。
    pub fn progress_for_stage(&self, stage: usize) -> f32 {
        if !self.enabled || !self.started || self.finished {
            return 1.0;
        }
        if stage >= self.stages {
            return 1.0;
        }
        let start = stage as u32 * (REVEAL_FADE_FRAMES + REVEAL_GAP_FRAMES);
        let local = self.frames.saturating_sub(start).min(REVEAL_FADE_FRAMES);
        local as f32 / REVEAL_FADE_FRAMES as f32
    }

    pub fn opacity_for_stage(&self, stage: usize) -> f32 {
        self.progress_for_stage(stage).clamp(0.0, 1.0)
    }

    /// 「升起」位移：起始下沉 `REVEAL_RISE_PX`，随进度回到 0。
    pub fn offset_for_stage(&self, stage: usize) -> f32 {
        (1.0 - self.opacity_for_stage(stage)) * REVEAL_RISE_PX
    }

    pub fn overall_progress(&self) -> f32 {
        if !self.enabled || !self.started || self.finished {
            return 1.0;
        }
        let total = self.total_frames();
        if total == 0 {
            1.0
        } else {
            (self.frames as f32 / total as f32).clamp(0.0, 1.0)
        }
    }
}

/// 首屏主动效互斥：存在 P1 路径播放时，P5 必须让位。
pub const fn reveal_allowed(requested: bool, p1_playback: bool) -> bool {
    requested && !p1_playback
}
