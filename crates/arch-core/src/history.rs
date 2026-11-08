//! git 历史采样与模块演化指标（ticket 08）。
//!
//! 只读 git 对象库：不 checkout、不建工作树、不改动目标仓库。
//! 默认沿 first-parent 链取最近 500 个提交；合并提交的第二父分支改动不计入模块指标。

use crate::{display_path, is_ignored_dir, is_source_file};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// `history` JSON 产物的 schema 版本。
pub const HISTORY_SCHEMA_VERSION: u32 = 1;

/// 默认窗口：最近 500 个 first-parent 提交。
pub const DEFAULT_MAX_COMMITS: usize = 500;

#[derive(Debug, Clone)]
pub struct HistoryOptions {
    pub max_commits: usize,
    pub since: Option<String>,
    pub until: Option<String>,
}

impl Default for HistoryOptions {
    fn default() -> Self {
        Self {
            max_commits: DEFAULT_MAX_COMMITS,
            since: None,
            until: None,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryArtifact {
    pub schema_version: u32,
    pub tool_version: String,
    pub repo: HistoryRepo,
    pub window: HistoryWindow,
    /// 是否使用采样；当前实现不做采样，恒为 false。
    pub sampled: bool,
    /// 采样步长；未采样时为 null。
    pub sample_step: Option<usize>,
    /// 窗口内提交数超过 `maxCommits` 时为 true。
    pub truncated: bool,
    /// 目标仓库是浅克隆；为 true 时直接失败，不输出该产物。
    pub shallow: bool,
    pub stats: HistoryStats,
    pub modules: Vec<HistoryModule>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRepo {
    /// 仓库根（`git rev-parse --show-toplevel`）。
    pub path: String,
    pub head: String,
    pub is_git: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryWindow {
    pub max_commits: usize,
    pub since: Option<String>,
    pub until: Option<String>,
    pub first_parent: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryStats {
    pub commits_scanned: usize,
    pub commits_available: usize,
    pub modules: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryModule {
    /// 模块谱系的最终路径（相对仓库根，正斜杠）。
    pub id: String,
    /// 窗口内最早触及该谱系的提交时间（Unix epoch 秒）。
    pub first_seen: u64,
    /// 窗口内最晚触及该谱系的提交时间（Unix epoch 秒）。
    pub last_modified: u64,
    /// 窗口内触及该谱系的 first-parent 提交数（含重命名提交，合并提交不计）。
    pub commit_count: usize,
    pub churn: Churn,
    /// `lastModified - firstSeen`（秒）。
    pub active_span: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Churn {
    pub additions: u64,
    pub deletions: u64,
    pub total: u64,
}

#[derive(Debug)]
pub enum HistoryError {
    NotGit(String),
    NotDirectory(String),
    NoCommits(String),
    Shallow(String),
    Git(String),
    Io(String),
}

impl std::fmt::Display for HistoryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HistoryError::NotGit(path) => write!(formatter, "目标不是 git 仓库: {path}"),
            HistoryError::NotDirectory(path) => {
                write!(formatter, "目标路径不存在或不是目录: {path}")
            }
            HistoryError::NoCommits(path) => write!(formatter, "仓库没有提交: {path}"),
            HistoryError::Shallow(path) => write!(formatter, "浅克隆仓库无法给出完整历史: {path}"),
            HistoryError::Git(message) => write!(formatter, "git 调用失败: {message}"),
            HistoryError::Io(message) => write!(formatter, "读取目标仓库失败: {message}"),
        }
    }
}

impl std::error::Error for HistoryError {}

#[derive(Debug)]
struct Commit {
    parents: Vec<String>,
    time: u64,
    changes: Vec<(u64, u64, String)>,
}

#[derive(Debug, Default)]
struct ModuleStats {
    first_seen: u64,
    last_modified: u64,
    commit_count: usize,
    additions: u64,
    deletions: u64,
}

struct RepoContext {
    toplevel_text: String,
    head: String,
}

fn open_repo(root: &Path) -> Result<RepoContext, HistoryError> {
    let requested = PathBuf::from(root);
    if !requested.is_dir() {
        return Err(HistoryError::NotDirectory(display_path(&requested)));
    }

    // 正常路径合并为单次 rev-parse：仓库根、浅克隆标记、HEAD 一次取回。
    // 只在失败时退回逐项查询，保留 NotGit / NoCommits 的精确错误分类。
    let combined = git_optional(
        &requested,
        &[
            "rev-parse",
            "--show-toplevel",
            "--is-shallow-repository",
            "HEAD",
        ],
    )?;
    if let Some(text) = combined {
        let mut lines = text.lines().map(str::trim);
        if let (Some(toplevel), Some(shallow), Some(head)) =
            (lines.next(), lines.next(), lines.next())
        {
            if !toplevel.is_empty()
                && !head.is_empty()
                && head.chars().all(|c| c.is_ascii_hexdigit())
            {
                let toplevel = PathBuf::from(toplevel);
                let toplevel_text = display_path(&toplevel);
                if shallow == "true" {
                    return Err(HistoryError::Shallow(toplevel_text));
                }
                return Ok(RepoContext {
                    toplevel_text,
                    head: head.to_string(),
                });
            }
        }
    }

    let Some(toplevel) = git_optional(&requested, &["rev-parse", "--show-toplevel"])? else {
        return Err(HistoryError::NotGit(display_path(&requested)));
    };
    let toplevel = PathBuf::from(toplevel.trim());
    let toplevel_text = display_path(&toplevel);

    let shallow = git_optional(&toplevel, &["rev-parse", "--is-shallow-repository"])?
        .map(|value| value.trim() == "true")
        .unwrap_or(false);
    if shallow {
        return Err(HistoryError::Shallow(toplevel_text));
    }

    let Some(head) = git_optional(&toplevel, &["rev-parse", "HEAD"])? else {
        return Err(HistoryError::NoCommits(toplevel_text));
    };
    let head = head.trim().to_string();

    Ok(RepoContext {
        toplevel_text,
        head,
    })
}

/// 仓库元信息与提交列表是两条独立的 git 调用，并发执行以摊掉进程启动成本。
/// 两者都失败时优先返回 `open_repo` 的错误（NotGit / NoCommits / Shallow 分类更精确）。
fn load_repo_and_commits(
    root: &Path,
    since: Option<&str>,
    until: Option<&str>,
) -> Result<(RepoContext, Vec<Commit>), HistoryError> {
    let (context, commits) = std::thread::scope(|scope| {
        let context = scope.spawn(|| open_repo(root));
        let commits = scope.spawn(|| load_commits(root, since, until));
        (
            context.join().expect("open_repo thread"),
            commits.join().expect("load_commits thread"),
        )
    });
    Ok((context?, commits?))
}

fn load_commits(
    toplevel: &Path,
    since: Option<&str>,
    until: Option<&str>,
) -> Result<Vec<Commit>, HistoryError> {
    let mut log_arguments = vec![
        "log",
        "--first-parent",
        "--format=%x01%H%x09%P%x09%ct",
        "--numstat",
        "-M",
    ];
    if let Some(since) = since {
        log_arguments.push("--since");
        log_arguments.push(since);
    }
    if let Some(until) = until {
        log_arguments.push("--until");
        log_arguments.push(until);
    }
    log_arguments.push("HEAD");
    let log = git(toplevel, &log_arguments)?;
    Ok(parse_log(&log))
}

/// 把已过滤好的提交聚合为产物；调用方负责保证 `commits` 已是该刻度的前缀集合。
fn aggregate(
    commits: &[&Commit],
    context: &RepoContext,
    tool_version: &str,
    max_commits: usize,
    since: Option<String>,
    until: Option<String>,
) -> HistoryArtifact {
    let available = commits.len();
    let truncated = available > max_commits;
    let scanned = available.min(max_commits);

    let mut stats: HashMap<String, ModuleStats> = HashMap::new();
    let mut aliases: HashMap<String, String> = HashMap::new();

    for commit in commits.iter().take(scanned).rev() {
        if commit.parents.len() > 1 {
            // 合并提交：第二父分支改动不计入指标；first-parent 侧差异也已由主线提交记录。
            continue;
        }
        for (additions, deletions, path) in &commit.changes {
            let (module_path, renamed_from) = match split_renamed_path(path) {
                Some((old, new)) => (new, Some(old)),
                None => (path.clone(), None),
            };
            if !is_history_module(&module_path) {
                continue;
            }
            if let Some(old) = renamed_from {
                aliases.insert(old, module_path.clone());
            }
            let entry = stats.entry(module_path).or_default();
            if entry.commit_count == 0 || commit.time < entry.first_seen {
                entry.first_seen = commit.time;
            }
            if entry.commit_count == 0 || commit.time > entry.last_modified {
                entry.last_modified = commit.time;
            }
            entry.commit_count += 1;
            entry.additions += *additions;
            entry.deletions += *deletions;
        }
    }

    let mut aggregated: BTreeMap<String, ModuleStats> = BTreeMap::new();
    for (path, entry) in stats {
        let lineage = resolve_lineage(&path, &aliases);
        let target = aggregated.entry(lineage).or_default();
        if target.commit_count == 0 || entry.first_seen < target.first_seen {
            target.first_seen = entry.first_seen;
        }
        if entry.last_modified > target.last_modified {
            target.last_modified = entry.last_modified;
        }
        target.commit_count += entry.commit_count;
        target.additions += entry.additions;
        target.deletions += entry.deletions;
    }

    let modules = aggregated
        .into_iter()
        .map(|(id, entry)| HistoryModule {
            id,
            first_seen: entry.first_seen,
            last_modified: entry.last_modified,
            commit_count: entry.commit_count,
            churn: Churn {
                additions: entry.additions,
                deletions: entry.deletions,
                total: entry.additions + entry.deletions,
            },
            active_span: entry.last_modified.saturating_sub(entry.first_seen),
        })
        .collect::<Vec<_>>();

    HistoryArtifact {
        schema_version: HISTORY_SCHEMA_VERSION,
        tool_version: tool_version.to_string(),
        repo: HistoryRepo {
            path: context.toplevel_text.clone(),
            head: context.head.clone(),
            is_git: true,
        },
        window: HistoryWindow {
            max_commits,
            since,
            until,
            first_parent: true,
        },
        sampled: false,
        sample_step: None,
        truncated,
        shallow: false,
        stats: HistoryStats {
            commits_scanned: scanned,
            commits_available: available,
            modules: modules.len(),
        },
        modules,
    }
}

pub fn build_history(
    root: &Path,
    tool_version: &str,
    options: HistoryOptions,
) -> Result<HistoryArtifact, HistoryError> {
    let (context, commits) =
        load_repo_and_commits(root, options.since.as_deref(), options.until.as_deref())?;
    let refs = commits.iter().collect::<Vec<_>>();
    Ok(aggregate(
        &refs,
        &context,
        tool_version,
        options.max_commits,
        options.since,
        options.until,
    ))
}

/// 一次 git 遍历生成多个「截至各刻度」的前缀产物（ticket 09 性能修复入口）。
///
/// `tick_times` 为各刻度的提交时间（Unix epoch 秒）；每个刻度的 `until` 仍按旧口径
/// 记为 `@<time + 1>`，产物与逐刻度调用 [`build_history`] 逐字节一致。
/// `options.until` 被 `tick_times` 覆盖，调用方不应同时传入。
pub fn build_history_prefixes(
    root: &Path,
    tool_version: &str,
    options: HistoryOptions,
    tick_times: &[u64],
) -> Result<Vec<HistoryArtifact>, HistoryError> {
    debug_assert!(
        options.until.is_none(),
        "build_history_prefixes 忽略 options.until，请改用 tick_times"
    );
    // 一次遍历取全量 first-parent 提交；每个刻度只在内存里按同一个 `--until` 语义过滤。
    let (context, commits) = load_repo_and_commits(root, options.since.as_deref(), None)?;

    let mut artifacts = Vec::with_capacity(tick_times.len());
    for time in tick_times {
        // 与旧实现一致：刻度 T 的 until 为 `@T+1`，按 committer date 过滤。
        let bound = time.saturating_add(1);
        let filtered = commits
            .iter()
            .filter(|commit| commit.time <= bound)
            .collect::<Vec<_>>();
        artifacts.push(aggregate(
            &filtered,
            &context,
            tool_version,
            options.max_commits,
            options.since.clone(),
            Some(format!("@{bound}")),
        ));
    }
    Ok(artifacts)
}

fn git(repo: &Path, arguments: &[&str]) -> Result<String, HistoryError> {
    let output = run_git(repo, arguments)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(HistoryError::Git(format!(
            "git {} 失败: {}",
            arguments.join(" "),
            stderr.trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn git_optional(repo: &Path, arguments: &[&str]) -> Result<Option<String>, HistoryError> {
    let output = run_git(repo, arguments)?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(Some(String::from_utf8_lossy(&output.stdout).to_string()))
}

fn run_git(repo: &Path, arguments: &[&str]) -> Result<Output, HistoryError> {
    Command::new("git")
        .arg("-C")
        .arg(repo)
        .arg("-c")
        .arg("core.quotepath=false")
        .args(arguments)
        .output()
        .map_err(|error| HistoryError::Git(format!("无法启动 git: {error}")))
}

fn parse_log(output: &str) -> Vec<Commit> {
    let mut commits: Vec<Commit> = Vec::new();
    for line in output.lines() {
        if let Some(rest) = line.strip_prefix('\u{1}') {
            let mut fields = rest.split('\t');
            let sha = fields.next().unwrap_or("").trim().to_string();
            if sha.is_empty() {
                continue;
            }
            let parents = fields
                .next()
                .unwrap_or("")
                .split_whitespace()
                .map(str::to_string)
                .collect::<Vec<_>>();
            let time = fields
                .next()
                .and_then(|value| value.trim().parse().ok())
                .unwrap_or(0);
            commits.push(Commit {
                parents,
                time,
                changes: Vec::new(),
            });
            continue;
        }
        if let Some(commit) = commits.last_mut() {
            if let Some((additions, deletions, path)) = parse_numstat(line) {
                commit.changes.push((additions, deletions, path));
            }
        }
    }
    commits
}

fn parse_numstat(line: &str) -> Option<(u64, u64, String)> {
    let mut fields = line.splitn(3, '\t');
    let additions = fields.next()?;
    let deletions = fields.next()?;
    let path = fields.next()?.to_string();
    if path.is_empty() {
        return None;
    }
    let additions = additions.parse::<u64>().unwrap_or(0);
    let deletions = deletions.parse::<u64>().unwrap_or(0);
    Some((additions, deletions, path))
}

fn split_renamed_path(path: &str) -> Option<(String, String)> {
    if let (Some(open), Some(close)) = (path.find('{'), path.rfind('}')) {
        if open < close {
            let inner = &path[open + 1..close];
            if let Some(index) = inner.find(" => ") {
                let prefix = &path[..open];
                let suffix = &path[close + 1..];
                let old = format!("{prefix}{}{suffix}", &inner[..index]);
                let new = format!("{prefix}{}{suffix}", &inner[index + 4..]);
                return Some((old, new));
            }
        }
    }
    let index = path.find(" => ")?;
    Some((path[..index].to_string(), path[index + 4..].to_string()))
}

fn is_history_module(path: &str) -> bool {
    if path.split('/').any(is_ignored_dir) {
        return false;
    }
    is_source_file(Path::new(path))
}

fn resolve_lineage(path: &str, aliases: &HashMap<String, String>) -> String {
    let mut current = path.to_string();
    for _ in 0..64 {
        match aliases.get(&current) {
            Some(next) if next != &current => current = next.clone(),
            _ => return current,
        }
    }
    current
}
