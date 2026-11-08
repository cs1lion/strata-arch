//! ticket 09 演化时间线的纯状态模型。
//!
//! 每个时间线刻度对应「截至该提交」重新采样的历史产物；拖动刻度只切换读数，
//! 不回退到最新值。热力映射固定，模块 ID 与地图产物一致。

use arch_core::history::{HistoryArtifact, HistoryModule};
use std::collections::BTreeSet;

/// 每个刻度在 1x 速度下的停留时长（毫秒）。
pub const PLAYBACK_DWELL_MS: f32 = 700.0;
/// 可切换的播放速度档位；`0.5x` 最慢，`2x` 最快。
pub const PLAYBACK_SPEEDS: [f32; 3] = [0.5, 1.0, 2.0];
/// 采样/截断时的可见退化标记文案。
pub const DEGRADED_LABEL: &str = "采样结果（非完整历史）";
/// 浅克隆时的可见退化标记文案。
pub const SHALLOW_LABEL: &str = "浅克隆（历史不完整）";

pub const HEAT_NEUTRAL: [u8; 3] = [0x55, 0x5a, 0x63];
pub const HEAT_STOPS: [([u8; 3], u64); 4] = [
    ([0x2d, 0x4f, 0x7a], 0),
    ([0x3b, 0x6f, 0xa5], 1),
    ([0x55, 0x8f, 0xa5], 4),
    ([0xc0, 0x6b, 0x3a], 11),
];

#[derive(Debug)]
pub struct TimelineEntry {
    pub hash: String,
    pub time: u64,
    pub subject: String,
    pub artifact: HistoryArtifact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Heat {
    pub color: [u8; 3],
    pub size_scale_percent: u16,
    pub neutral: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimelineFlags {
    pub sampled: bool,
    pub truncated: bool,
    pub shallow: bool,
}

/// 固定的热力映射：颜色按 churn 总量分档，尺寸按整数百分比放大。
pub fn heat_for_churn(total: u64, commit_count: usize) -> Heat {
    let color = HEAT_STOPS
        .iter()
        .rev()
        .find(|(_, threshold)| total >= *threshold)
        .map(|(color, _)| *color)
        .unwrap_or(HEAT_NEUTRAL);
    Heat {
        color,
        size_scale_percent: (100 + commit_count.min(10) * 3 + (total.min(20) as usize)) as u16,
        neutral: false,
    }
}

pub struct TimelineState {
    entries: Vec<TimelineEntry>,
    current: usize,
    selected: Option<String>,
    missing: BTreeSet<usize>,
    gap_index: Option<usize>,
}

impl TimelineState {
    /// 新状态默认停在最新提交；空输入时 `current` 为 0。
    pub fn new(entries: Vec<TimelineEntry>) -> Self {
        let current = entries.len().saturating_sub(1);
        Self {
            entries,
            current,
            selected: None,
            missing: BTreeSet::new(),
            gap_index: None,
        }
    }

    /// 构造带采样空洞的时间线；空洞刻度保留位置，但不可被拖动命中。
    pub fn new_with_holes(entries: Vec<TimelineEntry>, missing: BTreeSet<usize>) -> Self {
        let total = entries.len();
        let mut sanitized = BTreeSet::new();
        sanitized.extend(missing.into_iter().filter(|index| *index < total));
        let mut current = total.saturating_sub(1);
        while current > 0 && sanitized.contains(&current) {
            current -= 1;
        }
        Self {
            entries,
            current,
            selected: None,
            missing: sanitized,
            gap_index: None,
        }
    }

    pub fn ticks(&self) -> usize {
        self.entries.len()
    }

    pub fn current_index(&self) -> usize {
        self.current
    }

    pub fn current_entry(&self) -> Option<&TimelineEntry> {
        self.entries.get(self.current)
    }

    pub fn entries(&self) -> &[TimelineEntry] {
        &self.entries
    }

    /// 拖动时间线；越界或命中采样空洞返回 `false` 且索引不变。
    pub fn scrub_to(&mut self, index: usize) -> bool {
        if index >= self.entries.len() {
            return false;
        }
        if self.missing.contains(&index) {
            self.gap_index = Some(index);
            return false;
        }
        self.current = index;
        self.gap_index = None;
        true
    }

    /// 已实际采样的刻度数（空洞不计入）。
    pub fn sampled_ticks(&self) -> usize {
        self.entries.len().saturating_sub(self.missing.len())
    }

    /// 可见的已采样刻度数文案。
    pub fn sampling_label(&self) -> String {
        let mut label = format!("已采样 {}/{} 刻度", self.sampled_ticks(), self.ticks());
        if let Some(entry) = self.current_entry() {
            let stats = &entry.artifact.stats;
            if entry.artifact.truncated && stats.commits_available > 0 {
                label.push_str(&format!(
                    "（窗口 {}/{} 提交）",
                    stats.commits_scanned, stats.commits_available
                ));
            }
        }
        label
    }

    /// 上一次跳到采样空洞时的显式提示；成功拖动后清空。
    pub fn gap_notice(&self) -> Option<String> {
        self.gap_index.map(|index| {
            format!(
                "第 {}/{} 刻度未采样（历史采样空洞），仍显示第 {}/{} 刻度",
                index + 1,
                self.ticks(),
                self.current + 1,
                self.ticks()
            )
        })
    }

    /// 当前刻度读数的稳定 JSON 快照，用于字节级复现对比。
    pub fn snapshot_json(&self) -> String {
        let mut map = serde_json::Map::new();
        map.insert("tick".to_string(), serde_json::json!(self.current));
        if let Some(entry) = self.current_entry() {
            map.insert("hash".to_string(), serde_json::json!(entry.hash));
            map.insert("time".to_string(), serde_json::json!(entry.time));
            map.insert("subject".to_string(), serde_json::json!(entry.subject));
            map.insert(
                "artifact".to_string(),
                serde_json::to_value(&entry.artifact).expect("history artifact must serialize"),
            );
        }
        serde_json::Value::Object(map).to_string()
    }

    pub fn module_ids(&self) -> BTreeSet<String> {
        self.current_entry()
            .map(|entry| {
                entry
                    .artifact
                    .modules
                    .iter()
                    .map(|module| module.id.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn module_metric(&self, id: &str) -> Option<&HistoryModule> {
        self.current_entry()?
            .artifact
            .modules
            .iter()
            .find(|module| module.id == id)
    }

    pub fn select(&mut self, id: &str) -> bool {
        if self.module_metric(id).is_some() {
            self.selected = Some(id.to_string());
            true
        } else {
            self.selected = None;
            false
        }
    }

    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    pub fn clear_selection(&mut self) {
        self.selected = None;
    }

    /// 无历史模块返回中性灰并带标记；否则按 churn 总量落在固定色阶。
    pub fn heat(&self, id: &str) -> Heat {
        match self.module_metric(id) {
            Some(module) => heat_for_churn(module.churn.total, module.commit_count),
            None => Heat {
                color: HEAT_NEUTRAL,
                size_scale_percent: 100,
                neutral: true,
            },
        }
    }

    pub fn flags(&self) -> TimelineFlags {
        self.current_entry()
            .map(|entry| TimelineFlags {
                sampled: entry.artifact.sampled,
                truncated: entry.artifact.truncated,
                shallow: entry.artifact.shallow,
            })
            .unwrap_or(TimelineFlags {
                sampled: false,
                truncated: false,
                shallow: false,
            })
    }

    /// 采样/截断/浅克隆存在时，热力不能表现成全量读数。
    pub fn heat_is_full_scale(&self) -> bool {
        let flags = self.flags();
        !(flags.sampled || flags.truncated || flags.shallow)
    }

    /// 返回地图：清空选择，时间线刻度保留。
    pub fn leave_timeline(&mut self) {
        self.selected = None;
    }
}

/// 采样/截断/浅克隆时必须给出的可见退化标记；正常历史返回 `None`。
pub fn degradation_label(flags: TimelineFlags) -> Option<&'static str> {
    if flags.sampled || flags.truncated {
        Some(DEGRADED_LABEL)
    } else if flags.shallow {
        Some(SHALLOW_LABEL)
    } else {
        None
    }
}

/// 时间线离散播放状态机：按刻度 dwell 推进，不做任何插值。
///
/// 推进只消费调用方从 GPUI 帧回调算出的时间增量（毫秒），本模型不读墙钟、不用随机源。
pub struct TimelinePlayback {
    ticks: usize,
    current: usize,
    enabled: bool,
    playing: bool,
    ended: bool,
    speed_index: usize,
    accumulated_ms: f32,
}

impl TimelinePlayback {
    /// 默认停在最新刻度（与静态时间线一致），未播放。
    pub fn new(ticks: usize) -> Self {
        Self {
            ticks,
            current: ticks.saturating_sub(1),
            enabled: true,
            playing: false,
            ended: false,
            speed_index: 1,
            accumulated_ms: 0.0,
        }
    }

    pub fn ticks(&self) -> usize {
        self.ticks
    }

    pub fn current(&self) -> usize {
        self.current
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    pub fn is_ended(&self) -> bool {
        self.ended
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn speed(&self) -> f32 {
        PLAYBACK_SPEEDS[self.speed_index]
    }

    pub fn speed_index(&self) -> usize {
        self.speed_index
    }

    /// 关闭播放：立即暂停；后续帧不再被调度。
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.playing = false;
        }
    }

    /// 循环切换速度档位，返回新速度。
    pub fn cycle_speed(&mut self) -> f32 {
        self.speed_index = (self.speed_index + 1) % PLAYBACK_SPEEDS.len();
        self.speed()
    }

    /// 开始播放。位于末刻度时只能由用户显式触发重播（回到起点），到末尾后自动停止，
    /// 不做无限循环。少于 2 个刻度或播放已关闭时返回 `false`。
    pub fn play(&mut self) -> bool {
        if !self.enabled || self.ticks < 2 {
            return false;
        }
        if self.current >= self.ticks - 1 {
            self.current = 0;
        }
        self.ended = false;
        self.playing = true;
        self.accumulated_ms = 0.0;
        true
    }

    pub fn pause(&mut self) {
        self.playing = false;
        self.accumulated_ms = 0.0;
    }

    /// 单步前进一个刻度并自动暂停；已到末刻度返回 `false`。
    pub fn step_forward(&mut self) -> bool {
        if !self.enabled {
            return false;
        }
        if self.current + 1 >= self.ticks {
            self.playing = false;
            self.ended = true;
            return false;
        }
        self.current += 1;
        self.playing = false;
        self.ended = self.current + 1 >= self.ticks;
        self.accumulated_ms = 0.0;
        true
    }

    /// 单步后退并自动暂停；已在起点返回 `false`。
    pub fn step_back(&mut self) -> bool {
        if !self.enabled || self.current == 0 {
            return false;
        }
        self.current -= 1;
        self.playing = false;
        self.ended = false;
        self.accumulated_ms = 0.0;
        true
    }

    /// 跳到首刻度并停止播放；无刻度或播放关闭时返回 `false`。
    pub fn jump_to_first(&mut self) -> bool {
        self.jump_to(0)
    }

    /// 跳到末刻度并停止播放；无刻度或播放关闭时返回 `false`。
    pub fn jump_to_last(&mut self) -> bool {
        self.jump_to(self.ticks.saturating_sub(1))
    }

    fn jump_to(&mut self, index: usize) -> bool {
        if !self.enabled || self.ticks == 0 {
            return false;
        }
        self.current = index.min(self.ticks - 1);
        self.playing = false;
        self.ended = false;
        self.accumulated_ms = 0.0;
        true
    }

    /// 消费一帧的时间增量（毫秒，由 GPUI 帧回调给出）；返回是否发生刻度切换。
    ///
    /// 每帧最多前进一个刻度并清零累计，保证每个刻度至少被渲染一帧——即使掉帧也不会
    /// 跳过某个 commit 的显示值。
    pub fn on_frame(&mut self, delta_ms: f32) -> bool {
        if !self.enabled || !self.playing || self.ticks < 2 {
            return false;
        }
        self.accumulated_ms += delta_ms.max(0.0) * self.speed();
        if self.accumulated_ms < PLAYBACK_DWELL_MS {
            return false;
        }
        self.accumulated_ms = 0.0;
        if self.current + 1 >= self.ticks {
            self.playing = false;
            self.ended = true;
            return false;
        }
        self.current += 1;
        if self.current + 1 >= self.ticks {
            self.playing = false;
            self.ended = true;
        }
        true
    }

    /// 用户拖动时间轴：停止播放并跟到目标刻度。
    pub fn on_scrub(&mut self, index: usize) {
        if index >= self.ticks {
            return;
        }
        self.current = index;
        self.playing = false;
        self.ended = false;
        self.accumulated_ms = 0.0;
    }

    /// 是否需要继续订阅 GPUI 帧回调。
    pub fn should_drive_frames(&self) -> bool {
        self.enabled && self.playing
    }
}
