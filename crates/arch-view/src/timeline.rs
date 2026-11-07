//! ticket 09 演化时间线的纯状态模型。
//!
//! 每个时间线刻度对应「截至该提交」重新采样的历史产物；拖动刻度只切换读数，
//! 不回退到最新值。热力映射固定，模块 ID 与地图产物一致。

use arch_core::history::{HistoryArtifact, HistoryModule};
use std::collections::BTreeSet;

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
}

impl TimelineState {
    /// 新状态默认停在最新提交；空输入时 `current` 为 0。
    pub fn new(entries: Vec<TimelineEntry>) -> Self {
        let current = entries.len().saturating_sub(1);
        Self {
            entries,
            current,
            selected: None,
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

    /// 拖动时间线；越界返回 `false` 且索引不变。
    pub fn scrub_to(&mut self, index: usize) -> bool {
        if index >= self.entries.len() {
            return false;
        }
        self.current = index;
        true
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
