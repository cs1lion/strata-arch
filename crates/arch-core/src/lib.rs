//! arch-core：解析、图模型与索引产物的共享实现。
//!
//! 本阶段（ticket 01）只提供索引产物契约与 TS/JS 源文件发现；
//! 模块解析、符号/调用图与历史指标由后续 ticket 扩展。

use serde::Serialize;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// 索引 JSON 产物的 schema 版本。
pub const SCHEMA_VERSION: u32 = 1;

/// 目标仓库源文件数上限（Q6：≤5k 文件）。
pub const DEFAULT_MAX_FILES: usize = 5000;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexArtifact {
    pub schema_version: u32,
    pub tool_version: String,
    pub repo: RepoInfo,
    pub stats: Stats,
    pub modules: Vec<ModuleNode>,
    pub edges: Vec<Edge>,
    pub unresolved: Vec<UnresolvedItem>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoInfo {
    pub path: String,
    pub head: Option<String>,
    pub is_git: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub files: usize,
    pub modules: usize,
    pub edges: usize,
    pub unresolved: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleNode {
    pub id: String,
    pub path: String,
    pub language: &'static str,
    pub lines: usize,
}

/// 依赖/调用边。ticket 01 只保留契约，边集合为空。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Edge {
    pub source: String,
    pub target: String,
    pub kind: String,
}

/// 未能解析的导入。ticket 01 只保留契约，集合为空。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnresolvedItem {
    pub file: String,
    pub specifier: String,
    pub reason: String,
}

#[derive(Debug)]
pub enum ScanError {
    NotADirectory(String),
    TooManyFiles { found: usize, limit: usize },
    Io(String),
}

impl fmt::Display for ScanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScanError::NotADirectory(path) => write!(f, "目标路径不存在或不是目录: {path}"),
            ScanError::TooManyFiles { found, limit } => write!(
                f,
                "目标仓库源文件数 {found} 超过上限 {limit}，已拒绝索引"
            ),
            ScanError::Io(message) => write!(f, "读取目标仓库失败: {message}"),
        }
    }
}

impl std::error::Error for ScanError {}

/// 从目标仓库构建索引产物。只读取源文件，不写入目标仓库。
pub fn build_index(root: &Path, tool_version: &str) -> Result<IndexArtifact, ScanError> {
    let root = fs::canonicalize(root)
        .map_err(|error| ScanError::NotADirectory(format!("{}（{error}）", root.display())))?;
    if !root.is_dir() {
        return Err(ScanError::NotADirectory(display_path(&root)));
    }

    let mut files = Vec::new();
    walk(&root, &root, &mut files)?;
    if files.len() > DEFAULT_MAX_FILES {
        return Err(ScanError::TooManyFiles {
            found: files.len(),
            limit: DEFAULT_MAX_FILES,
        });
    }

    files.sort();

    let modules = files
        .iter()
        .map(|path| {
            let relative = relative_slash_path(&root, path);
            ModuleNode {
                id: relative.clone(),
                path: relative,
                language: language_of(path),
                lines: count_lines(path),
            }
        })
        .collect::<Vec<_>>();

    let is_git = root.join(".git").exists();
    let head = if is_git {
        git_head(&root)
    } else {
        None
    };

    Ok(IndexArtifact {
        schema_version: SCHEMA_VERSION,
        tool_version: tool_version.to_string(),
        repo: RepoInfo {
            path: display_path(&root),
            head,
            is_git,
        },
        stats: Stats {
            files: modules.len(),
            modules: modules.len(),
            edges: 0,
            unresolved: 0,
        },
        modules,
        edges: Vec::new(),
        unresolved: Vec::new(),
    })
}

fn walk(root: &Path, current: &Path, files: &mut Vec<PathBuf>) -> Result<(), ScanError> {
    let entries = fs::read_dir(current).map_err(|error| {
        ScanError::Io(format!("{}（{error}）", display_path(current)))
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| ScanError::Io(error.to_string()))?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|error| {
            ScanError::Io(format!("{}（{error}）", display_path(&path)))
        })?;

        // 跳过符号链接，避免目录环。
        if file_type.is_symlink() {
            continue;
        }

        if file_type.is_dir() {
            if is_ignored_dir(entry.file_name().to_string_lossy().as_ref()) {
                continue;
            }
            walk(root, &path, files)?;
            continue;
        }

        if file_type.is_file() && is_source_file(&path) {
            files.push(path);
        }
    }
    Ok(())
}

fn is_ignored_dir(name: &str) -> bool {
    matches!(
        name,
        "node_modules"
            | ".git"
            | "target"
            | "dist"
            | "build"
            | "out"
            | "coverage"
            | ".next"
            | ".turbo"
            | ".cache"
    )
}

fn is_source_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|value| value.to_str()),
        Some("ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs")
    )
}

fn language_of(path: &Path) -> &'static str {
    match path.extension().and_then(|value| value.to_str()) {
        Some("ts" | "tsx") => "typescript",
        _ => "javascript",
    }
}

fn count_lines(path: &Path) -> usize {
    // 源文件按字节读取，避免编码差异影响索引是否成功。
    fs::read(path)
        .map(|bytes| {
            let newlines = bytes.iter().filter(|byte| **byte == b'\n').count();
            if bytes.ends_with(b"\n") {
                newlines
            } else {
                newlines + 1
            }
        })
        .unwrap_or(0)
}

fn relative_slash_path(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy().replace('\\', "/"))
        .collect::<Vec<_>>()
        .join("/")
}

fn display_path(path: &Path) -> String {
    let text = path.to_string_lossy().to_string();
    text.strip_prefix(r"\\?\").unwrap_or(&text).to_string()
}

fn git_head(root: &Path) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("rev-parse")
        .arg("HEAD")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let head = String::from_utf8(output.stdout).ok()?;
    let head = head.trim();
    if head.is_empty() {
        None
    } else {
        Some(head.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_extensions_are_recognized() {
        assert!(is_source_file(Path::new("a.ts")));
        assert!(is_source_file(Path::new("a.tsx")));
        assert!(is_source_file(Path::new("a.mjs")));
        assert!(!is_source_file(Path::new("a.json")));
        assert!(!is_source_file(Path::new("a.md")));
    }

    #[test]
    fn ignored_directories_cover_dependency_and_build_outputs() {
        for name in ["node_modules", ".git", "target", "dist", "out"] {
            assert!(is_ignored_dir(name), "{name} should be ignored");
        }
        assert!(!is_ignored_dir("src"));
    }

    #[test]
    fn module_ids_use_forward_slashes() {
        let root = Path::new("repo");
        let path = root.join("src").join("main.ts");
        assert_eq!(relative_slash_path(root, &path), "src/main.ts");
    }
}
