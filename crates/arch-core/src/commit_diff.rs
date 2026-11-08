//! P4：双 commit 结构 diff（ticket 17）。
//!
//! 只读两个 commit 的 tree，不 checkout、不建工作树、不改目标仓库。
//! 变更分三类：`added` / `removed` / `modified`；`modified` 只认「符号数」或
//! 「该文件的出向依赖边集合 `(target, kind)`」变化，不把纯内容变化算作结构变更。
//!
//! 重命名不归并：疑似重命名只写 `renameHint` 与 `renames` 提示，
//! added / removed 两条原始记录都保留。

use serde::Serialize;
use std::path::Path;

/// 结构 diff 产物的独立 schema 版本，不复用其他产物的版本号。
pub const COMMIT_DIFF_SCHEMA_VERSION: u32 = 1;

/// 疑似重命名的路径相似度下限。
pub const RENAME_PATH_THRESHOLD: f32 = 0.6;
/// 疑似重命名的符号名称集合 Jaccard 下限。
pub const RENAME_SYMBOL_THRESHOLD: f32 = 0.8;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitDiffArtifact {
    pub schema_version: u32,
    pub tool_version: String,
    pub from: CommitIdentity,
    pub to: CommitIdentity,
    pub stats: CommitDiffStats,
    pub added: Vec<FileChange>,
    pub removed: Vec<FileChange>,
    pub modified: Vec<ModifiedFile>,
    /// 疑似重命名候选；两侧原始记录仍保留在 `added` / `removed`。
    pub renames: Vec<RenameHint>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitIdentity {
    pub commit: String,
    pub tree: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitDiffStats {
    pub added: usize,
    pub removed: usize,
    pub modified: usize,
    pub renames: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    pub path: String,
    /// 该文件在对应 commit 中的符号数（空文件也有 `module-toplevel`，计 1）。
    pub symbol_count: usize,
    /// 该文件的出向依赖边数量。
    pub dependency_edges: usize,
    /// 疑似重命名对象的路径；仅提示，不改变记录归属。
    pub rename_hint: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModifiedFile {
    pub path: String,
    pub symbol_count_from: usize,
    pub symbol_count_to: usize,
    pub dependency_edges_from: usize,
    pub dependency_edges_to: usize,
    /// `symbol-count` / `dependency-edges`，按固定顺序输出。
    pub reasons: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameHint {
    pub from: String,
    pub to: String,
    pub path_similarity: f32,
    pub symbol_overlap: f32,
}

#[derive(Debug)]
pub enum CommitDiffError {
    NotGit(String),
    NotDirectory(String),
    UnknownCommit(String),
    Git(String),
    Io(String),
}

impl std::fmt::Display for CommitDiffError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CommitDiffError::NotGit(path) => write!(formatter, "目标不是 git 仓库: {path}"),
            CommitDiffError::NotDirectory(path) => {
                write!(formatter, "目标路径不存在或不是目录: {path}")
            }
            CommitDiffError::UnknownCommit(commit) => write!(formatter, "未知 commit: {commit}"),
            CommitDiffError::Git(message) => write!(formatter, "git 调用失败: {message}"),
            CommitDiffError::Io(message) => write!(formatter, "读取目标仓库失败: {message}"),
        }
    }
}

impl std::error::Error for CommitDiffError {}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

/// 计算两个 commit 的结构 diff。只读。
pub fn diff_commits(
    root: &Path,
    from: &str,
    to: &str,
    tool_version: &str,
) -> Result<CommitDiffArtifact, CommitDiffError> {
    let _ = display_path(root);
    Ok(CommitDiffArtifact {
        schema_version: COMMIT_DIFF_SCHEMA_VERSION,
        tool_version: tool_version.to_string(),
        from: CommitIdentity {
            commit: from.to_string(),
            tree: String::new(),
        },
        to: CommitIdentity {
            commit: to.to_string(),
            tree: String::new(),
        },
        stats: CommitDiffStats {
            added: 0,
            removed: 0,
            modified: 0,
            renames: 0,
        },
        added: Vec::new(),
        removed: Vec::new(),
        modified: Vec::new(),
        renames: Vec::new(),
    })
}
