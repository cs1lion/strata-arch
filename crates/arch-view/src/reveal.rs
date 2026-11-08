//! P5 首屏分层揭示的纯状态机。
//!
//! 进度只由 GPUI 动画时钟给出的「已播时长占比」派生：UI 侧用 `with_animation`
//! 把框架时间映射成 `0..=1` 的 progress，再喂给本模块。本模块不读墙钟、不用随机源、
//! 不注册定时器，也不记录帧数——帧率只决定中间帧的疏密，不改变总时长。
//! 揭示一次性：播完停在最终态，不循环、不呼吸、不自动重播；任何交互可调用 `skip()`
//! 立即落到最终态。

/// 揭示总时长（毫秒）。固定时长驱动：慢机器上只是帧更少，总时长不变。
pub const REVEAL_DURATION_MS: u64 = 900;
/// 「升起」的起始下沉像素；最终态位移为 0。
pub const REVEAL_RISE_PX: f32 = 12.0;

#[derive(Debug, Clone, PartialEq)]
pub struct RevealState {
    enabled: bool,
    started: bool,
    finished: bool,
    stages: usize,
    /// 已播时长占比 `0..=1`，只由框架动画时钟写入。
    progress: f32,
}

impl RevealState {
    /// `enabled=false` 即关闭态：等价于已结束的最终态，不请求任何动画帧。
    pub fn new(enabled: bool, stages: usize) -> Self {
        if enabled && stages > 0 {
            Self {
                enabled: true,
                started: false,
                finished: false,
                stages,
                progress: 0.0,
            }
        } else {
            Self {
                enabled: false,
                started: false,
                finished: true,
                stages,
                progress: 1.0,
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

    /// 正在揭示：调用方需要继续观察动画进度。
    pub fn is_active(&self) -> bool {
        self.enabled && self.started && !self.finished
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }

    pub fn stages(&self) -> usize {
        self.stages
    }

    /// 已播时长占比（`0..=1`），即框架动画时钟给出的进度。
    pub fn progress(&self) -> f32 {
        if !self.enabled || !self.started || self.finished {
            1.0
        } else {
            self.progress
        }
    }

    /// 数据就绪后调用一次；只有启用且未开始时生效，重复调用无操作。
    pub fn start(&mut self) -> bool {
        if !self.enabled || self.started || self.finished || self.stages == 0 {
            return false;
        }
        self.started = true;
        self.progress = 0.0;
        true
    }

    /// 写入框架动画时钟给出的已播时长占比；进度单调不减，到 1 即结束。
    ///
    /// 未启动、已结束或关闭态都是无操作，避免迟到帧把最终态拖回中途。
    pub fn set_progress(&mut self, progress: f32) {
        if !self.is_active() {
            return;
        }
        self.progress = self.progress.max(progress.clamp(0.0, 1.0));
        if self.progress >= 1.0 {
            self.finished = true;
        }
    }

    /// 交互打断：立即落到最终态；返回是否确实中断了一次进行中的揭示。
    pub fn skip(&mut self) -> bool {
        if !self.enabled || self.finished {
            return false;
        }
        self.started = true;
        self.progress = 1.0;
        self.finished = true;
        true
    }

    /// 第 `stage` 层（按传入的层序）的进度 `0..=1`。
    ///
    /// 各层带按层序均分整段时长：第 `stage` 层在 `stage / 层数` 处开始淡入，
    /// 用 `1 / 层数` 的时长完成。`pending` 与关闭态返回 1.0：数据未就绪或未启用时
    /// 不得让内容消失。
    pub fn progress_for_stage(&self, stage: usize) -> f32 {
        if !self.enabled || !self.started || self.finished {
            return 1.0;
        }
        if stage >= self.stages {
            return 1.0;
        }
        let stages = self.stages as f32;
        (self.progress * stages - stage as f32).clamp(0.0, 1.0)
    }

    pub fn opacity_for_stage(&self, stage: usize) -> f32 {
        self.progress_for_stage(stage).clamp(0.0, 1.0)
    }

    /// 「升起」位移：起始下沉 `REVEAL_RISE_PX`，随进度回到 0。
    pub fn offset_for_stage(&self, stage: usize) -> f32 {
        (1.0 - self.opacity_for_stage(stage)) * REVEAL_RISE_PX
    }

    pub fn overall_progress(&self) -> f32 {
        self.progress()
    }
}

/// 首屏主动效互斥：存在 P1 路径播放时，P5 必须让位。
pub const fn reveal_allowed(requested: bool, p1_playback: bool) -> bool {
    requested && !p1_playback
}
