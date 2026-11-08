//! 「先开窗、后加载」的加载态纯模型与数据准备入口。
//!
//! 窗口先打开并呈现这里的加载态文案，数据准备在后台线程分阶段完成后
//! 再由视图切换到真实内容。加载态的文案必须说明「正在准备什么」，
//! 失败时必须把错误写进可见文案；本模块不读墙钟、不用随机源。

use std::path::Path;
use std::process::Command;

use arch_core::history::{build_history_prefixes, HistoryOptions};
use arch_core::layers::{build_layers, LayersArtifact};

use crate::layout::{layout_map, LayoutConfig, MapLayout};
use crate::timeline::{TimelineEntry, TimelineState};

/// 加载态的阶段。`SamplingHistory` 在 `git log` 完成后带上刻度总数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadingStage {
    ReadingCommits,
    SamplingHistory { ticks: usize },
    BuildingLayers,
    ComputingLayout,
}

impl LoadingStage {
    pub fn label(&self) -> String {
        match self {
            Self::ReadingCommits => "读取提交历史（git log）".to_string(),
            Self::SamplingHistory { ticks } => format!("准备历史：共 {ticks} 个刻度（一次遍历）"),
            Self::BuildingLayers => "构建分层".to_string(),
            Self::ComputingLayout => "计算布局".to_string(),
        }
    }
}

/// 加载态的帧计数：动效相位只由帧数派生，不使用墙钟。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadingState {
    stage: LoadingStage,
    frames: u64,
}

impl LoadingState {
    pub fn new() -> Self {
        Self {
            stage: LoadingStage::ReadingCommits,
            frames: 0,
        }
    }

    pub fn with_stage(stage: LoadingStage) -> Self {
        Self { stage, frames: 0 }
    }

    pub fn stage(&self) -> &LoadingStage {
        &self.stage
    }

    pub fn set_stage(&mut self, stage: LoadingStage) {
        self.stage = stage;
    }

    pub fn advance_frame(&mut self) {
        self.frames = self.frames.wrapping_add(1);
    }

    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// 由帧计数派生的动效相位（0..4）。
    pub fn phase(&self) -> usize {
        (self.frames % 4) as usize
    }

    pub fn headline(&self) -> &'static str {
        "正在准备演化时间线"
    }

    /// 窗口内必须显示的非空文案：标题 + 当前阶段 + 一句说明。
    pub fn visible_lines(&self) -> Vec<String> {
        vec![
            self.headline().to_string(),
            format!("当前阶段：{}", self.stage.label()),
            "数据准备好后自动切换到时间线视图".to_string(),
        ]
    }
}

impl Default for LoadingState {
    fn default() -> Self {
        Self::new()
    }
}

/// 视图当前的内容状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadStatus {
    Loading(LoadingState),
    Failed { message: String },
    Ready,
}

impl LoadStatus {
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready)
    }

    pub fn is_loading(&self) -> bool {
        matches!(self, Self::Loading(_))
    }

    /// 加载/失败时必须给出可读文案；就绪后不再显示状态面板。
    pub fn visible_lines(&self) -> Vec<String> {
        match self {
            Self::Loading(loading) => loading.visible_lines(),
            Self::Failed { message } => vec![
                "数据准备失败".to_string(),
                message.clone(),
                "窗口保持打开，请检查仓库路径与 git 历史".to_string(),
            ],
            Self::Ready => Vec::new(),
        }
    }
}

/// `git log` 产出的一行提交。
#[derive(Debug, Clone)]
pub struct CommitRow {
    pub hash: String,
    pub time: u64,
    pub subject: String,
}

pub struct PreparedTimeline {
    pub timeline: TimelineState,
    pub layout: MapLayout,
}

/// 读取 first-parent 提交列表。
pub fn list_first_parent_commits(repo: &Path) -> Result<Vec<CommitRow>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args([
            "log",
            "--reverse",
            "--first-parent",
            "--format=%H%x09%ct%x09%s",
        ])
        .output()
        .map_err(|error| format!("运行 git log 失败: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git log 失败: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let mut commits = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let mut parts = line.splitn(3, '\t');
        let hash = parts.next().unwrap_or_default().to_string();
        let time = parts
            .next()
            .unwrap_or_default()
            .parse::<u64>()
            .map_err(|_| format!("无法解析提交时间: {line}"))?;
        let subject = parts.next().unwrap_or_default().to_string();
        commits.push(CommitRow {
            hash,
            time,
            subject,
        });
    }
    Ok(commits)
}

/// 一次遍历生成所有刻度前缀，语义与同步路径一致。
pub fn sample_history(repo: &Path, commits: &[CommitRow]) -> Result<Vec<TimelineEntry>, String> {
    if commits.is_empty() {
        return Err("没有可用的 first-parent 提交".to_string());
    }
    let tick_times = commits.iter().map(|commit| commit.time).collect::<Vec<_>>();
    let artifacts = build_history_prefixes(
        repo,
        env!("CARGO_PKG_VERSION"),
        HistoryOptions::default(),
        &tick_times,
    )
    .map_err(|error| format!("构建历史产物失败: {error}"))?;

    Ok(commits
        .iter()
        .cloned()
        .zip(artifacts)
        .map(|(commit, artifact)| TimelineEntry {
            hash: commit.hash,
            time: commit.time,
            subject: commit.subject,
            artifact,
        })
        .collect())
}

pub fn build_layer_artifact(repo: &Path) -> Result<LayersArtifact, String> {
    build_layers(repo, env!("CARGO_PKG_VERSION"))
        .map_err(|error| format!("构建层产物失败: {error}"))
}

pub fn compute_layout(artifact: &LayersArtifact) -> MapLayout {
    layout_map(artifact, LayoutConfig::default())
}

/// 同步准备路径：测试用它作为计数基准，正式路径按阶段在后台调用上面的函数。
pub fn prepare_timeline(repo: &Path) -> Result<PreparedTimeline, String> {
    let commits = list_first_parent_commits(repo)?;
    let entries = sample_history(repo, &commits)?;
    let artifact = build_layer_artifact(repo)?;
    let layout = compute_layout(&artifact);
    Ok(PreparedTimeline {
        timeline: TimelineState::new(entries),
        layout,
    })
}
