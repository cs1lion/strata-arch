//! P4：双 commit 结构 diff（ticket 17）。
//!
//! 只读两个 commit 的 tree，不 checkout、不建工作树、不改目标仓库。
//! 变更分三类：`added` / `removed` / `modified`；`modified` 只认「符号数」或
//! 「该文件的出向依赖边集合 `(target, kind)`」变化，不把纯内容变化算作结构变更。
//!
//! 重命名不归并：疑似重命名只写 `renameHint` 与 `renames` 提示，
//! added / removed 两条原始记录都保留。
//!
//! 数据源：`git ls-tree` + `git cat-file --batch` 把两个 commit 的 blob 落到
//! 仓库外的临时目录，随后复用 [`crate::build_index`] 与
//! [`crate::callgraph::build_call_graph`]。临时目录在结束时删除；目标工作区与 HEAD
//! 均不改变。

use crate::callgraph::build_call_graph;
use crate::{build_index, is_ignored_dir, is_source_file, ScanError};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

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

impl From<ScanError> for CommitDiffError {
    fn from(error: ScanError) -> Self {
        match error {
            ScanError::Io(message) => CommitDiffError::Io(message),
            other => CommitDiffError::Io(other.to_string()),
        }
    }
}

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
    let requested = PathBuf::from(root);
    if !requested.is_dir() {
        return Err(CommitDiffError::NotDirectory(display_path(&requested)));
    }

    let toplevel = git_optional(&requested, &["rev-parse", "--show-toplevel"])?
        .ok_or_else(|| CommitDiffError::NotGit(display_path(&requested)))?;
    let toplevel = PathBuf::from(toplevel.trim());

    let from_commit = resolve_commit(&toplevel, from)?;
    let to_commit = resolve_commit(&toplevel, to)?;
    let from_tree = resolve_tree(&toplevel, &from_commit)?;
    let to_tree = resolve_tree(&toplevel, &to_commit)?;

    if from_commit == to_commit || from_tree == to_tree {
        return Ok(empty_artifact(
            from_commit,
            from_tree,
            to_commit,
            to_tree,
            tool_version,
        ));
    }

    let from_snapshot = SnapshotDir::new("from")?;
    let to_snapshot = SnapshotDir::new("to")?;
    materialize_tree(&toplevel, &from_commit, from_snapshot.path())?;
    materialize_tree(&toplevel, &to_commit, to_snapshot.path())?;
    let from_facts = snapshot_facts(from_snapshot.path(), tool_version)?;
    let to_facts = snapshot_facts(to_snapshot.path(), tool_version)?;

    Ok(build_artifact(
        from_facts,
        to_facts,
        CommitIdentity {
            commit: from_commit,
            tree: from_tree,
        },
        CommitIdentity {
            commit: to_commit,
            tree: to_tree,
        },
        tool_version,
    ))
}

fn empty_artifact(
    from_commit: String,
    from_tree: String,
    to_commit: String,
    to_tree: String,
    tool_version: &str,
) -> CommitDiffArtifact {
    CommitDiffArtifact {
        schema_version: COMMIT_DIFF_SCHEMA_VERSION,
        tool_version: tool_version.to_string(),
        from: CommitIdentity {
            commit: from_commit,
            tree: from_tree,
        },
        to: CommitIdentity {
            commit: to_commit,
            tree: to_tree,
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
    }
}

fn resolve_commit(repo: &Path, revision: &str) -> Result<String, CommitDiffError> {
    let expression = format!("{revision}^{{commit}}");
    git_optional(repo, &["rev-parse", "--verify", &expression])?
        .map(|value| value.trim().to_string())
        .ok_or_else(|| CommitDiffError::UnknownCommit(revision.to_string()))
}

fn resolve_tree(repo: &Path, commit: &str) -> Result<String, CommitDiffError> {
    let expression = format!("{commit}^{{tree}}");
    git_optional(repo, &["rev-parse", "--verify", &expression])?
        .map(|value| value.trim().to_string())
        .ok_or_else(|| CommitDiffError::Git(format!("无法解析 tree: {commit}")))
}

fn git_optional(repo: &Path, args: &[&str]) -> Result<Option<String>, CommitDiffError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map_err(|error| CommitDiffError::Git(error.to_string()))?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(Some(String::from_utf8_lossy(&output.stdout).to_string()))
}

fn git(repo: &Path, args: &[&str]) -> Result<String, CommitDiffError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map_err(|error| CommitDiffError::Git(error.to_string()))?;
    if !output.status.success() {
        return Err(CommitDiffError::Git(format!(
            "git {args:?} 失败: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// 仓库外的临时树；`Drop` 时删除，保证不残留、不碰目标工作区。
struct SnapshotDir(PathBuf);

static SNAPSHOT_COUNTER: AtomicU64 = AtomicU64::new(0);

impl SnapshotDir {
    fn new(tag: &str) -> Result<Self, CommitDiffError> {
        let counter = SNAPSHOT_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "strata-commit-diff-{}-{counter}-{tag}",
            std::process::id()
        ));
        fs::create_dir_all(&path)
            .map_err(|error| CommitDiffError::Io(format!("{}（{error}）", path.display())))?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for SnapshotDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// 把 commit 的 blob 落到 `destination`；只写临时目录。
fn materialize_tree(repo: &Path, commit: &str, destination: &Path) -> Result<(), CommitDiffError> {
    let listing = git(repo, &["ls-tree", "-r", "-z", "--full-tree", commit])?;
    let mut entries = Vec::<(String, String)>::new();

    for record in listing.split('\0') {
        if record.is_empty() {
            continue;
        }
        let Some((meta, path)) = record.split_once('\t') else {
            continue;
        };
        let mut fields = meta.split(' ');
        let Some(mode) = fields.next() else { continue };
        let Some(kind) = fields.next() else { continue };
        let Some(object) = fields.next() else {
            continue;
        };
        if kind != "blob" || mode == "120000" {
            continue;
        }
        if !is_materialized_path(path) {
            continue;
        }
        entries.push((path.to_string(), object.to_string()));
    }

    entries.sort_by(|left, right| left.0.cmp(&right.0));
    if entries.is_empty() {
        return Ok(());
    }

    let mut child = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["cat-file", "--batch"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| CommitDiffError::Git(error.to_string()))?;

    if let Some(mut stdin) = child.stdin.take() {
        for (_, object) in &entries {
            writeln!(stdin, "{object}").map_err(|error| CommitDiffError::Io(error.to_string()))?;
        }
    }

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| CommitDiffError::Git("cat-file 缺少 stdout".to_string()))?;
    let mut reader = BufReader::new(stdout);

    for (path, _) in &entries {
        let mut header = String::new();
        reader
            .read_line(&mut header)
            .map_err(|error| CommitDiffError::Io(error.to_string()))?;
        let mut fields = header.split_whitespace();
        let Some(object) = fields.next() else {
            return Err(CommitDiffError::Git(format!(
                "cat-file 响应缺少对象: {header}"
            )));
        };
        let Some(kind) = fields.next() else {
            return Err(CommitDiffError::Git(format!(
                "cat-file 响应缺少类型: {header}"
            )));
        };
        if kind != "blob" {
            return Err(CommitDiffError::Git(format!(
                "cat-file 期望 blob，实际 {kind}: {object}"
            )));
        }
        let Some(size) = fields.next().and_then(|value| value.parse::<usize>().ok()) else {
            return Err(CommitDiffError::Git(format!(
                "cat-file 响应缺少尺寸: {header}"
            )));
        };

        let mut bytes = vec![0_u8; size];
        reader
            .read_exact(&mut bytes)
            .map_err(|error| CommitDiffError::Io(error.to_string()))?;
        // `--batch` 在内容后输出一个换行符。
        let mut newline = [0_u8; 1];
        reader
            .read_exact(&mut newline)
            .map_err(|error| CommitDiffError::Io(error.to_string()))?;

        let target = destination.join(path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| CommitDiffError::Io(format!("{}（{error}）", parent.display())))?;
        }
        fs::write(&target, bytes)
            .map_err(|error| CommitDiffError::Io(format!("{}（{error}）", target.display())))?;
    }

    let status = child
        .wait()
        .map_err(|error| CommitDiffError::Git(error.to_string()))?;
    if !status.success() {
        let mut stderr = String::new();
        if let Some(mut handle) = child.stderr.take() {
            let _ = handle.read_to_string(&mut stderr);
        }
        return Err(CommitDiffError::Git(format!("cat-file 退出失败: {stderr}")));
    }

    Ok(())
}

/// 只落源文件与解析配置；路径必须全部是普通组件，防止越出临时目录。
fn is_materialized_path(path: &str) -> bool {
    let candidate = Path::new(path);
    if candidate
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return false;
    }
    if path.split('/').any(|segment| is_ignored_dir(segment)) {
        return false;
    }
    if is_source_file(candidate) {
        return true;
    }
    matches!(
        candidate.file_name().and_then(|value| value.to_str()),
        Some("package.json" | "tsconfig.json" | "jsconfig.json" | "pnpm-workspace.yaml")
    )
}

#[derive(Debug, Default)]
struct SnapshotFacts {
    files: BTreeSet<String>,
    symbol_counts: BTreeMap<String, usize>,
    symbol_names: BTreeMap<String, BTreeSet<String>>,
    dependency_edges: BTreeMap<String, BTreeSet<(String, String)>>,
}

fn snapshot_facts(directory: &Path, tool_version: &str) -> Result<SnapshotFacts, CommitDiffError> {
    let index = build_index(directory, tool_version)?;
    let callgraph = build_call_graph(directory, tool_version)?;

    let mut facts = SnapshotFacts::default();
    for module in &index.modules {
        if module.kind == "source" {
            facts.files.insert(module.id.clone());
        }
    }
    for symbol in &callgraph.symbols {
        *facts
            .symbol_counts
            .entry(symbol.module.clone())
            .or_insert(0) += 1;
        if symbol.kind != "module-toplevel" {
            facts
                .symbol_names
                .entry(symbol.module.clone())
                .or_default()
                .insert(symbol.name.clone());
        }
    }
    for edge in &index.edges {
        facts
            .dependency_edges
            .entry(edge.source.clone())
            .or_default()
            .insert((edge.target.clone(), edge.kind.clone()));
    }
    Ok(facts)
}

fn build_artifact(
    from: SnapshotFacts,
    to: SnapshotFacts,
    from_identity: CommitIdentity,
    to_identity: CommitIdentity,
    tool_version: &str,
) -> CommitDiffArtifact {
    let added_paths = to
        .files
        .difference(&from.files)
        .cloned()
        .collect::<Vec<_>>();
    let removed_paths = from
        .files
        .difference(&to.files)
        .cloned()
        .collect::<Vec<_>>();

    let mut modified = Vec::new();
    for path in from.files.intersection(&to.files) {
        let from_count = from.symbol_counts.get(path).copied().unwrap_or(0);
        let to_count = to.symbol_counts.get(path).copied().unwrap_or(0);
        let from_edges = from.dependency_edges.get(path);
        let to_edges = to.dependency_edges.get(path);
        let edges_changed = match (from_edges, to_edges) {
            (None, None) => false,
            (Some(left), Some(right)) => left != right,
            _ => true,
        };
        if from_count == to_count && !edges_changed {
            continue;
        }
        let mut reasons = Vec::new();
        if from_count != to_count {
            reasons.push("symbol-count".to_string());
        }
        if edges_changed {
            reasons.push("dependency-edges".to_string());
        }
        modified.push(ModifiedFile {
            path: path.clone(),
            symbol_count_from: from_count,
            symbol_count_to: to_count,
            dependency_edges_from: from_edges.map_or(0, BTreeSet::len),
            dependency_edges_to: to_edges.map_or(0, BTreeSet::len),
            reasons,
        });
    }
    modified.sort_by(|left, right| left.path.cmp(&right.path));

    let mut renames = Vec::new();
    for removed in &removed_paths {
        for added in &added_paths {
            let similarity = path_similarity(removed, added);
            if similarity < RENAME_PATH_THRESHOLD {
                continue;
            }
            let overlap =
                symbol_overlap(from.symbol_names.get(removed), to.symbol_names.get(added));
            if overlap < RENAME_SYMBOL_THRESHOLD {
                continue;
            }
            renames.push(RenameHint {
                from: removed.clone(),
                to: added.clone(),
                path_similarity: similarity,
                symbol_overlap: overlap,
            });
        }
    }
    renames.sort_by(|left, right| {
        left.from
            .cmp(&right.from)
            .then_with(|| left.to.cmp(&right.to))
    });

    let added = added_paths
        .iter()
        .map(|path| FileChange {
            path: path.clone(),
            symbol_count: to.symbol_counts.get(path).copied().unwrap_or(0),
            dependency_edges: to.dependency_edges.get(path).map_or(0, BTreeSet::len),
            rename_hint: best_removed_source(path, &renames, true),
        })
        .collect::<Vec<_>>();
    let removed = removed_paths
        .iter()
        .map(|path| FileChange {
            path: path.clone(),
            symbol_count: from.symbol_counts.get(path).copied().unwrap_or(0),
            dependency_edges: from.dependency_edges.get(path).map_or(0, BTreeSet::len),
            rename_hint: best_removed_source(path, &renames, false),
        })
        .collect::<Vec<_>>();

    CommitDiffArtifact {
        schema_version: COMMIT_DIFF_SCHEMA_VERSION,
        tool_version: tool_version.to_string(),
        from: from_identity,
        to: to_identity,
        stats: CommitDiffStats {
            added: added.len(),
            removed: removed.len(),
            modified: modified.len(),
            renames: renames.len(),
        },
        added,
        removed,
        modified,
        renames,
    }
}

/// 为 added 记录找改名源，为 removed 记录找改名目标；多个候选时取路径相似度最高者。
fn best_removed_source(path: &str, renames: &[RenameHint], forward: bool) -> Option<String> {
    let mut best: Option<&RenameHint> = None;
    for hint in renames {
        let matches = if forward {
            hint.to == path
        } else {
            hint.from == path
        };
        if !matches {
            continue;
        }
        if best
            .map(|current| hint.path_similarity > current.path_similarity)
            .unwrap_or(true)
        {
            best = Some(hint);
        }
    }
    best.map(|hint| {
        if forward {
            hint.from.clone()
        } else {
            hint.to.clone()
        }
    })
}

/// 路径相似度 = 同目录 0.4 + 同扩展名 0.2 + 文件名公共前缀比例 0.2 + 公共后缀比例 0.2。
pub fn path_similarity(left: &str, right: &str) -> f32 {
    let left_path = Path::new(left);
    let right_path = Path::new(right);
    let mut score = 0.0;
    if left_path.parent() == right_path.parent() {
        score += 0.4;
    }
    if left_path.extension() == right_path.extension() {
        score += 0.2;
    }
    let left_stem = left_path.file_stem().and_then(|value| value.to_str());
    let right_stem = right_path.file_stem().and_then(|value| value.to_str());
    score += 0.2 * common_prefix_ratio(left_stem, right_stem);
    score += 0.2 * common_suffix_ratio(left_stem, right_stem);
    score.min(1.0)
}

fn common_prefix_ratio(left: Option<&str>, right: Option<&str>) -> f32 {
    let (Some(left), Some(right)) = (left, right) else {
        return 0.0;
    };
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let prefix = left
        .chars()
        .zip(right.chars())
        .take_while(|(left, right)| left == right)
        .count();
    prefix as f32 / left.chars().count().max(right.chars().count()) as f32
}

fn common_suffix_ratio(left: Option<&str>, right: Option<&str>) -> f32 {
    let (Some(left), Some(right)) = (left, right) else {
        return 0.0;
    };
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let suffix = left
        .chars()
        .rev()
        .zip(right.chars().rev())
        .take_while(|(left, right)| left == right)
        .count();
    suffix as f32 / left.chars().count().max(right.chars().count()) as f32
}

/// 符号名称集合的 Jaccard；任一侧为空时返回 0（空文件改名不产生提示）。
pub fn symbol_overlap(left: Option<&BTreeSet<String>>, right: Option<&BTreeSet<String>>) -> f32 {
    let (Some(left), Some(right)) = (left, right) else {
        return 0.0;
    };
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let intersection = left.intersection(right).count();
    let union = left.union(right).count();
    intersection as f32 / union as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_similarity_rewards_same_directory_and_prefix() {
        assert!(path_similarity("src/old.ts", "src/old-renamed.ts") >= RENAME_PATH_THRESHOLD);
        assert!(path_similarity("src/dead.ts", "src/new.ts") >= RENAME_PATH_THRESHOLD);
        assert!(path_similarity("src/a.ts", "lib/b.ts") < RENAME_PATH_THRESHOLD);
    }

    #[test]
    fn symbol_overlap_requires_non_empty_sets() {
        let left = ["legacy".to_string()].into_iter().collect();
        let right = ["legacy".to_string()].into_iter().collect();
        assert_eq!(symbol_overlap(Some(&left), Some(&right)), 1.0);
        let empty: BTreeSet<String> = BTreeSet::new();
        assert_eq!(symbol_overlap(Some(&empty), Some(&empty)), 0.0);
    }
}
