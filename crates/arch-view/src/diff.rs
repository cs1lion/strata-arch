//! ticket 17：双 commit 结构 diff 的纯视图模型。
//!
//! 三色块、行序、疑似重命名提示与 120ms 入场淡入都在此模型上计算并断言；
//! GPUI 只负责画出色块并把 `with_animation` 的进度写回。

use arch_core::commit_diff::CommitDiffArtifact;
use serde::Serialize;

pub const FADE_DURATION_MS: u64 = 120;
pub const FADE_FINAL_ALPHA: u8 = 255;

/// added 底色（绿）。
pub const DIFF_ADDED_TINT: u32 = 0x2f6b3f;
/// removed 底色（红）。
pub const DIFF_REMOVED_TINT: u32 = 0x7a3138;
/// modified 底色（琥珀）。
pub const DIFF_MODIFIED_TINT: u32 = 0x7a5b24;
/// 行标签颜色。
pub const DIFF_LABEL: u32 = 0xe6edf3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DiffKind {
    Added,
    Removed,
    Modified,
}

impl DiffKind {
    pub fn tint(self) -> u32 {
        match self {
            DiffKind::Added => DIFF_ADDED_TINT,
            DiffKind::Removed => DIFF_REMOVED_TINT,
            DiffKind::Modified => DIFF_MODIFIED_TINT,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            DiffKind::Added => "新增",
            DiffKind::Removed => "删除",
            DiffKind::Modified => "修改",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffRow {
    pub path: String,
    pub kind: DiffKind,
    /// 结构化摘要；不参与排序或判定。
    pub detail: String,
    /// 疑似重命名对象；仅提示，行仍保留在原类别。
    pub rename_hint: Option<String>,
    /// 该行在模型中的绘制顺序。
    pub z_index: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffModel {
    pub from_commit: String,
    pub to_commit: String,
    pub added: usize,
    pub removed: usize,
    pub modified: usize,
    pub renames: usize,
    pub rows: Vec<DiffRow>,
}

impl DiffModel {
    pub fn from_artifact(artifact: &CommitDiffArtifact) -> Self {
        let mut rows = Vec::with_capacity(
            artifact.stats.added + artifact.stats.removed + artifact.stats.modified,
        );
        for change in &artifact.added {
            rows.push(DiffRow {
                path: change.path.clone(),
                kind: DiffKind::Added,
                detail: format!(
                    "{} 个符号 · {} 条依赖",
                    change.symbol_count, change.dependency_edges
                ),
                rename_hint: change.rename_hint.clone(),
                z_index: rows.len() as i32,
            });
        }
        for change in &artifact.removed {
            rows.push(DiffRow {
                path: change.path.clone(),
                kind: DiffKind::Removed,
                detail: format!(
                    "{} 个符号 · {} 条依赖",
                    change.symbol_count, change.dependency_edges
                ),
                rename_hint: change.rename_hint.clone(),
                z_index: rows.len() as i32,
            });
        }
        for change in &artifact.modified {
            rows.push(DiffRow {
                path: change.path.clone(),
                kind: DiffKind::Modified,
                detail: format!(
                    "符号 {}→{} · 依赖 {}→{} · {}",
                    change.symbol_count_from,
                    change.symbol_count_to,
                    change.dependency_edges_from,
                    change.dependency_edges_to,
                    change.reasons.join(", ")
                ),
                rename_hint: None,
                z_index: rows.len() as i32,
            });
        }

        Self {
            from_commit: artifact.from.commit.clone(),
            to_commit: artifact.to.commit.clone(),
            added: artifact.stats.added,
            removed: artifact.stats.removed,
            modified: artifact.stats.modified,
            renames: artifact.stats.renames,
            rows,
        }
    }
}

/// 入场淡入开关与进度换算；关闭时任意进度都等于静态最终态。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DiffFade {
    enabled: bool,
}

impl DiffFade {
    pub fn new(enabled: bool) -> Self {
        Self { enabled }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// `progress` 由 GPUI 动画帧写入；`>= 1.0` 时恒为最终态。
    ///
    /// 入场用 ease-out：只影响 alpha，不改变底色 RGB；关闭时任意进度直接取最终态。
    pub fn alpha_at(&self, progress: f32) -> f32 {
        if !self.enabled || progress >= 1.0 {
            return 1.0;
        }
        let progress = progress.clamp(0.0, 1.0);
        1.0 - (1.0 - progress) * (1.0 - progress)
    }

    /// GPUI `rgba` 的打包格式：`0xRRGGBBAA`。
    pub fn packed_at(&self, kind: DiffKind, progress: f32) -> u32 {
        (kind.tint() << 8) | u32::from(crate::playback::opacity_alpha_byte(self.alpha_at(progress)))
    }
}

/// 静态渲染的最终色值，用于「关闭淡入逐位一致」的对照。
pub fn packed_static(kind: DiffKind) -> u32 {
    (kind.tint() << 8) | u32::from(FADE_FINAL_ALPHA)
}
