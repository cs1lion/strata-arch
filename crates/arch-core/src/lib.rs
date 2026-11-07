//! arch-core：解析、图模型与索引产物的共享实现。
//!
//! 当前覆盖 ticket 01 的产物契约与文件发现，以及 ticket 02 的模块级依赖图：
//! 相对导入、`tsconfig` 路径别名、workspace 包与外部包级节点。

pub mod callgraph;
pub(crate) mod config;
pub(crate) mod imports;
pub mod layers;
pub(crate) mod resolve;

use config::{ProjectConfig, WorkspaceLookup};
use imports::{scan_imports, ScannedImport};
use resolve::{relative_module_id, resolve_file_path, FileLookup};
use serde::Serialize;
use std::collections::BTreeSet;
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
    /// `source` 或 `external-package`。外部依赖只保留包级节点。
    pub kind: &'static str,
    pub language: &'static str,
    pub lines: usize,
}

/// 模块级依赖边。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Edge {
    pub source: String,
    pub target: String,
    pub kind: String,
}

/// 未能解析的导入。
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
            ScanError::TooManyFiles { found, limit } => {
                write!(f, "目标仓库源文件数 {found} 超过上限 {limit}，已拒绝索引")
            }
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

    let config = ProjectConfig::load(&root);
    let mut modules = Vec::with_capacity(files.len());
    let mut edges = BTreeSet::new();
    let mut unresolved = BTreeSet::new();
    let mut externals = BTreeSet::new();

    for path in &files {
        let relative = relative_slash_path(&root, path);
        let bytes = fs::read(path)
            .map_err(|error| ScanError::Io(format!("{}（{error}）", display_path(path))))?;

        modules.push(ModuleNode {
            id: relative.clone(),
            path: relative.clone(),
            kind: "source",
            language: language_of(path),
            lines: count_lines_bytes(&bytes),
        });

        let source = String::from_utf8_lossy(&bytes);
        for import in scan_imports(&source) {
            resolve_import(
                &config,
                &root,
                &relative,
                path,
                &import,
                &mut edges,
                &mut unresolved,
                &mut externals,
            );
        }
    }

    for package in &externals {
        modules.push(ModuleNode {
            id: format!("external:{package}"),
            path: package.clone(),
            kind: "external-package",
            language: "external",
            lines: 0,
        });
    }
    modules.sort_by(|left, right| left.id.cmp(&right.id));

    let edges = edges
        .into_iter()
        .map(|(source, target, kind)| Edge {
            source,
            target,
            kind,
        })
        .collect::<Vec<_>>();
    let unresolved = unresolved
        .into_iter()
        .map(|(file, specifier, reason)| UnresolvedItem {
            file,
            specifier,
            reason,
        })
        .collect::<Vec<_>>();

    let is_git = root.join(".git").exists();
    let head = if is_git { git_head(&root) } else { None };

    Ok(IndexArtifact {
        schema_version: SCHEMA_VERSION,
        tool_version: tool_version.to_string(),
        repo: RepoInfo {
            path: display_path(&root),
            head,
            is_git,
        },
        stats: Stats {
            files: files.len(),
            modules: modules.len(),
            edges: edges.len(),
            unresolved: unresolved.len(),
        },
        modules,
        edges,
        unresolved,
    })
}

type EdgeKey = (String, String, String);
type UnresolvedKey = (String, String, String);

fn resolve_import(
    config: &ProjectConfig,
    root: &Path,
    source_id: &str,
    source_path: &Path,
    import: &ScannedImport,
    edges: &mut BTreeSet<EdgeKey>,
    unresolved: &mut BTreeSet<UnresolvedKey>,
    externals: &mut BTreeSet<String>,
) {
    let ScannedImport::Literal { specifier, kind } = import else {
        let ScannedImport::Expression { expression, .. } = import else {
            return;
        };
        unresolved.insert((
            source_id.to_string(),
            expression.clone(),
            "dynamic-import".to_string(),
        ));
        return;
    };
    let edge_kind = kind.edge_kind();

    if specifier.is_empty() {
        unresolved.insert((
            source_id.to_string(),
            specifier.clone(),
            "syntax-error".to_string(),
        ));
        return;
    }

    if specifier.starts_with('.') {
        let base = source_path.parent().unwrap_or(root);
        record_file_lookup(
            resolve_file_path(base, specifier),
            root,
            source_id,
            specifier,
            edge_kind,
            "relative-missing",
            edges,
            unresolved,
        );
        return;
    }

    if specifier.starts_with('/') {
        let relative = specifier.trim_start_matches('/');
        record_file_lookup(
            resolve_file_path(root, relative),
            root,
            source_id,
            specifier,
            edge_kind,
            "relative-missing",
            edges,
            unresolved,
        );
        return;
    }

    if let Some(targets) = config.alias_targets(specifier) {
        for target in targets {
            match resolve_file_path(&config.base_url, &target) {
                FileLookup::Found(file) => {
                    match relative_module_id(root, &file) {
                        Some(target_id) => {
                            edges.insert((source_id.to_string(), target_id, edge_kind.to_string()));
                        }
                        None => {
                            unresolved.insert((
                                source_id.to_string(),
                                specifier.clone(),
                                "alias-missing".to_string(),
                            ));
                        }
                    }
                    return;
                }
                FileLookup::UnsupportedExtension => {
                    unresolved.insert((
                        source_id.to_string(),
                        specifier.clone(),
                        "unsupported-extension".to_string(),
                    ));
                    return;
                }
                FileLookup::Missing => {}
            }
        }
        unresolved.insert((
            source_id.to_string(),
            specifier.clone(),
            "alias-missing".to_string(),
        ));
        return;
    }

    match config.workspace_lookup(specifier) {
        WorkspaceLookup::Resolved(file) => record_file_lookup(
            FileLookup::Found(file),
            root,
            source_id,
            specifier,
            edge_kind,
            "alias-missing",
            edges,
            unresolved,
        ),
        WorkspaceLookup::UnsupportedExtension => {
            unresolved.insert((
                source_id.to_string(),
                specifier.clone(),
                "unsupported-extension".to_string(),
            ));
        }
        WorkspaceLookup::Missing => {
            unresolved.insert((
                source_id.to_string(),
                specifier.clone(),
                "alias-missing".to_string(),
            ));
        }
        WorkspaceLookup::Unknown => {
            if let Some(package) = external_package_name(specifier) {
                externals.insert(package.clone());
                edges.insert((
                    source_id.to_string(),
                    format!("external:{package}"),
                    edge_kind.to_string(),
                ));
            } else {
                unresolved.insert((
                    source_id.to_string(),
                    specifier.clone(),
                    "syntax-error".to_string(),
                ));
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn record_file_lookup(
    lookup: FileLookup,
    root: &Path,
    source_id: &str,
    specifier: &str,
    edge_kind: &str,
    missing_reason: &str,
    edges: &mut BTreeSet<EdgeKey>,
    unresolved: &mut BTreeSet<UnresolvedKey>,
) {
    match lookup {
        FileLookup::Found(target) => match relative_module_id(root, &target) {
            Some(target_id) => {
                edges.insert((source_id.to_string(), target_id, edge_kind.to_string()));
            }
            None => {
                unresolved.insert((
                    source_id.to_string(),
                    specifier.to_string(),
                    missing_reason.to_string(),
                ));
            }
        },
        FileLookup::UnsupportedExtension => {
            unresolved.insert((
                source_id.to_string(),
                specifier.to_string(),
                "unsupported-extension".to_string(),
            ));
        }
        FileLookup::Missing => {
            unresolved.insert((
                source_id.to_string(),
                specifier.to_string(),
                missing_reason.to_string(),
            ));
        }
    }
}

pub(crate) fn external_package_name(specifier: &str) -> Option<String> {
    let mut segments = specifier.split('/');
    let first = segments.next().filter(|value| !value.is_empty())?;
    if first.starts_with('@') {
        let second = segments.next().filter(|value| !value.is_empty())?;
        Some(format!("{first}/{second}"))
    } else {
        Some(first.to_string())
    }
}

pub(crate) fn walk(root: &Path, current: &Path, files: &mut Vec<PathBuf>) -> Result<(), ScanError> {
    let entries = fs::read_dir(current)
        .map_err(|error| ScanError::Io(format!("{}（{error}）", display_path(current))))?;
    for entry in entries {
        let entry = entry.map_err(|error| ScanError::Io(error.to_string()))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| ScanError::Io(format!("{}（{error}）", display_path(&path))))?;

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

fn count_lines_bytes(bytes: &[u8]) -> usize {
    let newlines = bytes.iter().filter(|byte| **byte == b'\n').count();
    if bytes.ends_with(b"\n") {
        newlines
    } else {
        newlines + 1
    }
}

pub(crate) fn relative_slash_path(root: &Path, path: &Path) -> String {
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
