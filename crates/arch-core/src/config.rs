//! 项目配置读取：`tsconfig`/`jsconfig` 的 `baseUrl` + `paths`、npm/yarn 与 pnpm workspace。

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::resolve::{resolve_file_path, FileLookup};

#[derive(Debug, Clone)]
pub struct PathMapping {
    pub pattern: String,
    pub targets: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceLookup {
    Unknown,
    Resolved(PathBuf),
    UnsupportedExtension,
    Missing,
}

#[derive(Debug)]
pub struct ProjectConfig {
    pub base_url: PathBuf,
    pub paths: Vec<PathMapping>,
    pub workspaces: BTreeMap<String, PathBuf>,
}

impl ProjectConfig {
    pub fn load(root: &Path) -> Self {
        let mut base_url = root.to_path_buf();
        let mut paths = Vec::new();

        if let Some(config_path) = ["tsconfig.json", "jsconfig.json"]
            .iter()
            .map(|name| root.join(name))
            .find(|path| path.is_file())
        {
            if let Some(value) = read_json(&config_path) {
                if let Some(options) = value.get("compilerOptions") {
                    if let Some(url) = options.get("baseUrl").and_then(Value::as_str) {
                        base_url = config_path.parent().unwrap_or(root).join(url);
                    }
                    if let Some(map) = options.get("paths").and_then(Value::as_object) {
                        for (pattern, targets) in map {
                            let Some(list) = targets.as_array() else {
                                continue;
                            };
                            let targets = list
                                .iter()
                                .filter_map(Value::as_str)
                                .map(str::to_string)
                                .collect::<Vec<_>>();
                            if !targets.is_empty() {
                                paths.push(PathMapping {
                                    pattern: pattern.clone(),
                                    targets,
                                });
                            }
                        }
                    }
                }
            }
        }
        paths.sort_by(|left, right| left.pattern.cmp(&right.pattern));

        let mut patterns = Vec::new();
        if let Some(value) = read_json(&root.join("package.json")) {
            match value.get("workspaces") {
                Some(Value::Array(items)) => {
                    patterns.extend(items.iter().filter_map(Value::as_str).map(str::to_string));
                }
                Some(Value::Object(object)) => {
                    if let Some(Value::Array(items)) = object.get("packages") {
                        patterns.extend(items.iter().filter_map(Value::as_str).map(str::to_string));
                    }
                }
                _ => {}
            }
        }
        patterns.extend(read_pnpm_workspace_patterns(
            &root.join("pnpm-workspace.yaml"),
        ));

        let mut workspace_dirs = Vec::new();
        for pattern in patterns {
            expand_workspace(root, &pattern, &mut workspace_dirs);
        }
        workspace_dirs.sort();
        workspace_dirs.dedup();

        let mut workspaces = BTreeMap::new();
        for directory in workspace_dirs {
            let Some(value) = read_json(&directory.join("package.json")) else {
                continue;
            };
            let Some(name) = value.get("name").and_then(Value::as_str) else {
                continue;
            };
            if !name.is_empty() {
                workspaces
                    .entry(name.to_string())
                    .or_insert_with(|| directory.clone());
            }
        }

        Self {
            base_url,
            paths,
            workspaces,
        }
    }

    /// 返回匹配的 `paths` 目标（已替换通配符）。`None` 表示没有任何模式命中。
    pub fn alias_targets(&self, specifier: &str) -> Option<Vec<String>> {
        for mapping in &self.paths {
            if mapping.pattern == specifier {
                return Some(mapping.targets.clone());
            }
        }

        let mut best: Option<(usize, &PathMapping, String)> = None;
        for mapping in &self.paths {
            let Some(star) = mapping.pattern.find('*') else {
                continue;
            };
            let prefix = &mapping.pattern[..star];
            let suffix = &mapping.pattern[star + 1..];
            if !specifier.starts_with(prefix) || !specifier.ends_with(suffix) {
                continue;
            }
            if specifier.len() < prefix.len() + suffix.len() {
                continue;
            }
            let capture = &specifier[prefix.len()..specifier.len() - suffix.len()];
            if best
                .as_ref()
                .map(|(length, _, _)| prefix.len() > *length)
                .unwrap_or(true)
            {
                best = Some((prefix.len(), mapping, capture.to_string()));
            }
        }

        best.map(|(_, mapping, capture)| {
            mapping
                .targets
                .iter()
                .map(|target| target.replace('*', &capture))
                .collect()
        })
    }

    pub fn workspace_lookup(&self, specifier: &str) -> WorkspaceLookup {
        let Some((name, subpath)) = package_parts(specifier) else {
            return WorkspaceLookup::Unknown;
        };
        let Some(directory) = self.workspaces.get(&name) else {
            return WorkspaceLookup::Unknown;
        };

        if subpath.is_empty() {
            return match package_entry(directory) {
                Some(path) => WorkspaceLookup::Resolved(path),
                None => WorkspaceLookup::Missing,
            };
        }

        for base in [directory.clone(), directory.join("src")] {
            match resolve_file_path(&base, &subpath) {
                FileLookup::Found(path) => return WorkspaceLookup::Resolved(path),
                FileLookup::UnsupportedExtension => return WorkspaceLookup::UnsupportedExtension,
                FileLookup::Missing => {}
            }
        }
        WorkspaceLookup::Missing
    }
}

fn package_entry(directory: &Path) -> Option<PathBuf> {
    if let Some(value) = read_json(&directory.join("package.json")) {
        for field in ["source", "module", "main"] {
            if let Some(relative) = value.get(field).and_then(Value::as_str) {
                if let FileLookup::Found(path) = resolve_file_path(directory, relative) {
                    return Some(path);
                }
            }
        }
    }

    for relative in [
        "src/index.ts",
        "src/index.tsx",
        "index.ts",
        "index.tsx",
        "index.js",
        "index.jsx",
    ] {
        let candidate = directory.join(relative);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn package_parts(specifier: &str) -> Option<(String, String)> {
    let mut segments = specifier.split('/');
    let first = segments.next().filter(|value| !value.is_empty())?;

    if first.starts_with('@') {
        let second = segments.next().filter(|value| !value.is_empty())?;
        Some((
            format!("{first}/{second}"),
            segments.collect::<Vec<_>>().join("/"),
        ))
    } else {
        Some((first.to_string(), segments.collect::<Vec<_>>().join("/")))
    }
}

fn expand_workspace(root: &Path, pattern: &str, output: &mut Vec<PathBuf>) {
    let cleaned = pattern.trim().trim_matches('/');
    if cleaned.is_empty() {
        return;
    }
    let parts = cleaned
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect::<Vec<_>>();
    expand_parts(root, &parts, output);
}

fn expand_parts(base: &Path, parts: &[&str], output: &mut Vec<PathBuf>) {
    let Some(part) = parts.first().copied() else {
        if base.join("package.json").is_file() {
            output.push(base.to_path_buf());
        }
        return;
    };

    if part == "*" || part == "**" {
        let Ok(entries) = fs::read_dir(base) else {
            return;
        };
        let mut children = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect::<Vec<_>>();
        children.sort();
        for child in children {
            let name = child
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default();
            if name == "node_modules" || name.starts_with('.') {
                continue;
            }
            expand_parts(&child, &parts[1..], output);
        }
        return;
    }

    if part.contains('*') {
        // 只支持整段通配，避免隐式引入复杂的 glob 语义。
        return;
    }
    expand_parts(&base.join(part), &parts[1..], output);
}

fn read_pnpm_workspace_patterns(path: &Path) -> Vec<String> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut patterns = Vec::new();
    let mut in_packages = false;

    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or_default().trim_end();
        let trimmed = line.trim_start();
        if trimmed.starts_with("packages:") {
            in_packages = true;
            continue;
        }
        if !in_packages {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("- ") {
            let value = rest
                .trim()
                .trim_matches(|character| character == '\'' || character == '"');
            if !value.is_empty() {
                patterns.push(value.to_string());
            }
        } else if !trimmed.is_empty() {
            let indented = raw.chars().next().is_some_and(char::is_whitespace);
            if !indented {
                break;
            }
        }
    }
    patterns
}

/// 读取 JSONC（tsconfig 常见）并去掉注释与尾逗号后解析。
pub fn read_json(path: &Path) -> Option<Value> {
    let text = fs::read_to_string(path).ok()?;
    let cleaned = strip_trailing_commas(&strip_comments(&text));
    serde_json::from_str(&cleaned).ok()
}

fn strip_comments(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string: Option<char> = None;
    let mut escaped = false;

    while let Some(character) = chars.next() {
        if let Some(quote) = in_string {
            output.push(character);
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == quote {
                in_string = None;
            }
            continue;
        }

        if character == '"' || character == '\'' {
            in_string = Some(character);
            output.push(character);
            continue;
        }

        if character == '/' {
            if matches!(chars.peek(), Some('/')) {
                chars.next();
                for next in chars.by_ref() {
                    if next == '\n' {
                        output.push('\n');
                        break;
                    }
                }
                continue;
            }
            if matches!(chars.peek(), Some('*')) {
                chars.next();
                let mut previous = '\0';
                for next in chars.by_ref() {
                    if previous == '*' && next == '/' {
                        break;
                    }
                    previous = next;
                }
                continue;
            }
        }

        output.push(character);
    }
    output
}

fn strip_trailing_commas(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string: Option<char> = None;
    let mut escaped = false;

    while let Some(character) = chars.next() {
        if let Some(quote) = in_string {
            output.push(character);
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == quote {
                in_string = None;
            }
            continue;
        }

        if character == '"' || character == '\'' {
            in_string = Some(character);
            output.push(character);
            continue;
        }

        if character == ',' {
            let mut lookahead = chars.clone().skip_while(|next| next.is_whitespace());
            if matches!(lookahead.next(), Some('}') | Some(']')) {
                continue;
            }
        }
        output.push(character);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jsonc_comments_and_trailing_commas_are_stripped() {
        let input = r#"
            {
              // line comment
              "compilerOptions": {
                "baseUrl": ".", /* block comment */
                "paths": { "@app/*": ["src/*"], },
              },
            }
        "#;
        let value: Value = serde_json::from_str(&strip_trailing_commas(&strip_comments(input)))
            .expect("jsonc should parse");
        assert_eq!(value["compilerOptions"]["baseUrl"], ".");
    }

    #[test]
    fn longest_wildcard_prefix_wins() {
        let config = ProjectConfig {
            base_url: PathBuf::from("/repo"),
            paths: vec![
                PathMapping {
                    pattern: "@app/*".into(),
                    targets: vec!["src/*".into()],
                },
                PathMapping {
                    pattern: "@app/deep/*".into(),
                    targets: vec!["src/deep/*".into()],
                },
            ],
            workspaces: BTreeMap::new(),
        };
        assert_eq!(
            config.alias_targets("@app/deep/value"),
            Some(vec!["src/deep/value".to_string()])
        );
        assert_eq!(
            config.alias_targets("@app/value"),
            Some(vec!["src/value".to_string()])
        );
        assert_eq!(config.alias_targets("@other/value"), None);
    }
}
