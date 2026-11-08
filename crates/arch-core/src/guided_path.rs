//! ticket 06：入口候选、目标选择与最短路径查询。
//!
//! 路径只在调用图上游走（`call` / `jsx` / `dynamic-import` / `require`），
//! 不做「导入即可达」的推断；同一层多条等长路径按目标符号 ID 字典序、
//! 再按边种类、行、列排序，保证确定性。
//!
//! 入口候选来源（按优先级）：
//! 1. `package.json` 的 `bin` / `main` / `module` / `exports`（构建产物路径按
//!    `dist|build|out|lib` → `src`、去掉输出目录段、补 `src/` 三种规则映射回源码）；
//! 2. 仓库根与各 workspace 包目录下的 `index.*` / `src/index.*`；
//! 3. 上述入口模块的公开导出符号。
//!
//! 零候选时不静默返回空列表，直接报错。

use crate::callgraph::{build_call_graph, CallEdge, SymbolNode};
use crate::imports::{tokenize, TokenKind};
use crate::layers::{build_layers, UNCLASSIFIED};
use crate::{display_path, is_ignored_dir, is_source_file, RepoInfo, ScanError};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

/// `path` 产物的 schema 版本。
///
/// v2 在 v1 的步骤上增加 `evidence` / `layerFrom` / `layerTo` / `conditional` / `testOnly` / `narrative`；
/// v1 字段（`module` / `symbol` / `layer` / `via`）保持不变，属于向后兼容的字段扩展。
pub const PATH_SCHEMA_VERSION: u32 = 2;

/// `path --entries` 产物的 schema 版本，与路径产物分开演进。
///
/// 路径产物 v2 增加的叙述字段与入口候选无关，因此入口产物保持 v1。
pub const ENTRY_SCHEMA_VERSION: u32 = 1;

const SOURCE_EXTENSIONS: [&str; 6] = ["ts", "tsx", "js", "jsx", "mjs", "cjs"];
const OUTPUT_DIRECTORIES: [&str; 4] = ["dist", "build", "out", "lib"];
const NESTED_OUTPUT_DIRECTORIES: [&str; 5] = ["prod", "dev", "esm", "cjs", "lib"];
const EXPORT_KEYWORDS: [&str; 8] = [
    "default", "const", "let", "var", "function", "class", "async", "type",
];

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryCandidatesArtifact {
    pub schema_version: u32,
    pub tool_version: String,
    pub repo: RepoInfo,
    pub entries: Vec<EntryCandidate>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryCandidate {
    /// 入口符号 ID（`<module>#module-toplevel` 或公开导出符号）。
    pub id: String,
    pub module: String,
    pub kind: &'static str,
    pub layer: &'static str,
    /// `package.json:bin` / `package.json:main` / `package.json:module` /
    /// `package.json:exports` / `index` / `src-index` / `public-export`。
    pub source: String,
    /// 来源 `package.json` 相对仓库根的路径；由 `index` 推导的候选为 null。
    pub package_json: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuidedPathArtifact {
    pub schema_version: u32,
    pub tool_version: String,
    pub repo: RepoInfo,
    pub from: String,
    pub to: String,
    pub found: bool,
    pub length: usize,
    pub steps: Vec<PathStep>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PathStep {
    pub module: String,
    pub symbol: String,
    pub layer: &'static str,
    /// 进入该步的边种类；首步为 null。
    pub via: Option<String>,
    /// 进入该步的边证据（上一符号所在文件与边行列）；首步为 null。
    pub evidence: Option<EdgeEvidence>,
    /// 上一步的层；首步为 null。
    pub layer_from: Option<&'static str>,
    /// 本步的层，与 `layer` 相同，便于叙述模板直接引用。
    pub layer_to: &'static str,
    /// 边来自 `dynamic-import` / `require` 时标记为条件执行，不承诺一定发生。
    pub conditional: bool,
    /// 路径经过 `tests` 层（任一端）时标记为仅测试。
    pub test_only: bool,
    /// 固定模板生成的单步叙述，不引入 LLM。
    pub narrative: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EdgeEvidence {
    pub file: String,
    pub line: usize,
    pub column: usize,
}

#[derive(Debug)]
pub enum PathError {
    Scan(ScanError),
    /// 没有任何可用入口候选。
    NoEntries(String),
    UnknownSymbol(String),
    Unreachable {
        from: String,
        to: String,
    },
    Io(String),
}

impl std::fmt::Display for PathError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PathError::Scan(error) => write!(formatter, "{error}"),
            PathError::NoEntries(path) => {
                write!(formatter, "目标仓库没有可识别的入口候选: {path}")
            }
            PathError::UnknownSymbol(symbol) => write!(formatter, "未知符号: {symbol}"),
            PathError::Unreachable { from, to } => {
                write!(formatter, "路径不可达: {from} -> {to}")
            }
            PathError::Io(message) => write!(formatter, "读取目标仓库失败: {message}"),
        }
    }
}

impl std::error::Error for PathError {}

impl From<ScanError> for PathError {
    fn from(error: ScanError) -> Self {
        PathError::Scan(error)
    }
}

#[derive(Debug, Clone)]
struct EntryOrigin {
    source: String,
    package_json: Option<String>,
}

/// 列出候选入口。零候选时返回 [`PathError::NoEntries`]。
pub fn list_entries(root: &Path, tool_version: &str) -> Result<EntryCandidatesArtifact, PathError> {
    let callgraph = build_call_graph(root, tool_version)?;
    let layers = build_layers(root, tool_version)?;
    let layer_map = layer_map(&layers);

    let modules: BTreeSet<String> = callgraph
        .symbols
        .iter()
        .map(|symbol| symbol.module.clone())
        .collect();
    let symbols_by_module = symbols_by_module(&callgraph.symbols);

    let origins = discover_entry_origins(root, &modules);
    if origins.is_empty() {
        return Err(PathError::NoEntries(display_path(root)));
    }

    let mut entries: BTreeMap<String, EntryCandidate> = BTreeMap::new();

    for (module, origin) in &origins {
        if let Some(symbol) = symbols_by_module.get(module).and_then(|symbols| {
            symbols
                .iter()
                .find(|symbol| symbol.kind == "module-toplevel")
        }) {
            entries.entry(symbol.id.clone()).or_insert_with(|| {
                candidate(
                    symbol,
                    origin.source.clone(),
                    origin.package_json.clone(),
                    &layer_map,
                )
            });
        }

        let source_path = root.join(module);
        let Ok(source) = fs::read_to_string(&source_path) else {
            continue;
        };
        let exported = exported_names(&source);
        let Some(symbols) = symbols_by_module.get(module) else {
            continue;
        };
        for symbol in symbols {
            if !matches!(symbol.kind, "function" | "component" | "class") {
                continue;
            }
            if !exported.contains(&symbol.name) {
                continue;
            }
            entries.entry(symbol.id.clone()).or_insert_with(|| {
                candidate(symbol, "public-export".to_string(), None, &layer_map)
            });
        }
    }

    let entries = entries
        .into_values()
        .map(|mut entry| {
            if entry.source == "public-export" {
                if let Some(origin) = origins.get(&entry.module) {
                    entry.package_json = origin.package_json.clone();
                }
            }
            entry
        })
        .collect();

    Ok(EntryCandidatesArtifact {
        schema_version: ENTRY_SCHEMA_VERSION,
        tool_version: tool_version.to_string(),
        repo: callgraph.repo,
        entries,
    })
}

/// 从入口符号到目标符号的最短路径。
///
/// `from` 可以是符号 ID，也可以是模块 ID（自动补 `#module-toplevel`）。
pub fn build_path(
    root: &Path,
    tool_version: &str,
    from: &str,
    to: &str,
) -> Result<GuidedPathArtifact, PathError> {
    let callgraph = build_call_graph(root, tool_version)?;
    let layers = build_layers(root, tool_version)?;
    let layer_map = layer_map(&layers);

    let symbols: BTreeMap<&str, &SymbolNode> = callgraph
        .symbols
        .iter()
        .map(|symbol| (symbol.id.as_str(), symbol))
        .collect();

    let from =
        resolve_symbol(&symbols, from).ok_or_else(|| PathError::UnknownSymbol(from.to_string()))?;
    let to =
        resolve_symbol(&symbols, to).ok_or_else(|| PathError::UnknownSymbol(to.to_string()))?;

    let mut adjacency: BTreeMap<&str, Vec<&CallEdge>> = BTreeMap::new();
    for edge in &callgraph.edges {
        adjacency
            .entry(edge.source.as_str())
            .or_default()
            .push(edge);
    }
    for edges in adjacency.values_mut() {
        edges.sort_by(|left, right| {
            (
                left.target.as_str(),
                left.kind.as_str(),
                left.line,
                left.column,
            )
                .cmp(&(
                    right.target.as_str(),
                    right.kind.as_str(),
                    right.line,
                    right.column,
                ))
        });
    }

    let mut visited: BTreeSet<String> = BTreeSet::from([from.clone()]);
    let mut previous: BTreeMap<String, &CallEdge> = BTreeMap::new();
    let mut queue: VecDeque<String> = VecDeque::from([from.clone()]);
    let mut reached = false;

    while let Some(node) = queue.pop_front() {
        if node == to {
            reached = true;
            break;
        }
        let Some(edges) = adjacency.get(node.as_str()) else {
            continue;
        };
        for edge in edges {
            if visited.contains(&edge.target) {
                continue;
            }
            visited.insert(edge.target.clone());
            previous.insert(edge.target.clone(), edge);
            if edge.target == to {
                reached = true;
                break;
            }
            queue.push_back(edge.target.clone());
        }
        if reached {
            break;
        }
    }

    if !reached {
        return Err(PathError::Unreachable { from, to });
    }

    let mut sequence = vec![to.clone()];
    let mut cursor = to.clone();
    while cursor != from {
        let Some(edge) = previous.get(&cursor) else {
            return Err(PathError::Unreachable { from, to });
        };
        sequence.push(edge.source.clone());
        cursor = edge.source.clone();
    }
    sequence.reverse();

    let mut steps = Vec::with_capacity(sequence.len());
    for (index, symbol_id) in sequence.iter().enumerate() {
        let symbol = symbols
            .get(symbol_id.as_str())
            .ok_or_else(|| PathError::UnknownSymbol(symbol_id.clone()))?;
        let layer = layer_for(&layer_map, &symbol.module);

        let (via, evidence, layer_from, conditional) = if index == 0 {
            (None, None, None, false)
        } else {
            let edge = previous
                .get(symbol_id)
                .ok_or_else(|| PathError::Unreachable {
                    from: from.clone(),
                    to: to.clone(),
                })?;
            let predecessor_id = &sequence[index - 1];
            let predecessor = symbols
                .get(predecessor_id.as_str())
                .ok_or_else(|| PathError::UnknownSymbol(predecessor_id.clone()))?;
            let predecessor_layer = layer_for(&layer_map, &predecessor.module);
            (
                Some(edge.kind.clone()),
                Some(EdgeEvidence {
                    file: predecessor.path.clone(),
                    line: edge.line,
                    column: edge.column,
                }),
                Some(predecessor_layer),
                matches!(edge.kind.as_str(), "dynamic-import" | "require"),
            )
        };
        let test_only = layer == "tests" || layer_from == Some("tests");

        let narrative = match (&evidence, &via, layer_from) {
            (Some(evidence), Some(kind), Some(layer_from)) => {
                let predecessor = &sequence[index - 1];
                let mut text = format!(
                    "{} 通过 {} 到达 {}（{}:{}:{}；层：{} → {}）",
                    predecessor,
                    kind,
                    symbol.id,
                    evidence.file,
                    evidence.line,
                    evidence.column,
                    layer_from,
                    layer,
                );
                if conditional {
                    text.push_str("，条件执行");
                }
                if test_only {
                    text.push_str("，仅测试");
                }
                text
            }
            _ => format!(
                "{} 是起点（层：{}，模块：{}）",
                symbol.id, layer, symbol.module
            ),
        };

        steps.push(PathStep {
            module: symbol.module.clone(),
            symbol: symbol.id.clone(),
            layer,
            via,
            evidence,
            layer_from,
            layer_to: layer,
            conditional,
            test_only,
            narrative,
        });
    }

    Ok(GuidedPathArtifact {
        schema_version: PATH_SCHEMA_VERSION,
        tool_version: tool_version.to_string(),
        repo: callgraph.repo,
        from: from.clone(),
        to: to.clone(),
        found: true,
        length: steps.len(),
        steps,
    })
}

/// 把路径产物渲染成固定模板的纯文本，用于 `--narrative`。
pub fn render_narrative(artifact: &GuidedPathArtifact) -> String {
    let mut text = format!(
        "路径 {} → {}：{} 步\n",
        artifact.from, artifact.to, artifact.length
    );
    for step in &artifact.steps {
        text.push_str("- ");
        text.push_str(&step.narrative);
        text.push('\n');
    }
    text
}

fn layer_map(layers: &crate::layers::LayersArtifact) -> BTreeMap<String, &'static str> {
    layers
        .modules
        .iter()
        .map(|module| (module.id.clone(), module.layer))
        .collect()
}

fn layer_for(layer_map: &BTreeMap<String, &'static str>, module: &str) -> &'static str {
    layer_map.get(module).copied().unwrap_or(UNCLASSIFIED)
}

fn candidate(
    symbol: &SymbolNode,
    source: String,
    package_json: Option<String>,
    layer_map: &BTreeMap<String, &'static str>,
) -> EntryCandidate {
    EntryCandidate {
        id: symbol.id.clone(),
        module: symbol.module.clone(),
        kind: symbol.kind,
        layer: layer_for(layer_map, &symbol.module),
        source,
        package_json,
    }
}

fn symbols_by_module(symbols: &[SymbolNode]) -> BTreeMap<String, Vec<&SymbolNode>> {
    let mut map: BTreeMap<String, Vec<&SymbolNode>> = BTreeMap::new();
    for symbol in symbols {
        map.entry(symbol.module.clone()).or_default().push(symbol);
    }
    map
}

fn resolve_symbol(symbols: &BTreeMap<&str, &SymbolNode>, requested: &str) -> Option<String> {
    if symbols.contains_key(requested) {
        return Some(requested.to_string());
    }
    let toplevel = format!("{requested}#module-toplevel");
    if symbols.contains_key(toplevel.as_str()) {
        Some(toplevel)
    } else {
        None
    }
}

fn discover_entry_origins(
    root: &Path,
    modules: &BTreeSet<String>,
) -> BTreeMap<String, EntryOrigin> {
    let mut directories = Vec::new();
    if collect_package_directories(root, &mut directories).is_err() {
        directories.clear();
    }
    directories.insert(0, root.to_path_buf());
    directories.sort_by_key(|directory| relative_directory(root, directory));
    directories.dedup();

    let mut origins: BTreeMap<String, EntryOrigin> = BTreeMap::new();

    for directory in directories {
        let package_directory = relative_directory(root, &directory);
        let manifest_path = directory.join("package.json");
        let package_json = if package_directory.is_empty() {
            "package.json".to_string()
        } else {
            format!("{package_directory}/package.json")
        };

        if let Ok(text) = fs::read_to_string(&manifest_path) {
            if let Ok(manifest) = serde_json::from_str::<Value>(&text) {
                for (source, specifier) in manifest_specifiers(&manifest) {
                    let Some(module) = resolve_specifier(root, &package_directory, &specifier)
                    else {
                        continue;
                    };
                    if !modules.contains(&module) {
                        continue;
                    }
                    origins.entry(module).or_insert_with(|| EntryOrigin {
                        source,
                        package_json: Some(package_json.clone()),
                    });
                }
            }
        }

        if let Some((module, source)) = index_module(root, &package_directory) {
            if modules.contains(&module) {
                origins.entry(module).or_insert_with(|| EntryOrigin {
                    source: source.to_string(),
                    package_json: None,
                });
            }
        }
    }

    origins
}

fn collect_package_directories(directory: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if directory.join("package.json").is_file() {
        out.push(directory.to_path_buf());
    }
    let mut children = Vec::new();
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        if path.is_dir() {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            if is_ignored_dir(name) || name.starts_with('.') {
                continue;
            }
            children.push(path);
        }
    }
    children.sort();
    for child in children {
        collect_package_directories(&child, out)?;
    }
    Ok(())
}

fn relative_directory(root: &Path, directory: &Path) -> String {
    let relative = directory.strip_prefix(root).unwrap_or(directory);
    relative
        .to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_string()
}

fn manifest_specifiers(manifest: &Value) -> Vec<(String, String)> {
    let mut specifiers = Vec::new();

    if let Some(bin) = manifest.get("bin") {
        match bin {
            Value::String(value) => {
                specifiers.push(("package.json:bin".to_string(), value.clone()))
            }
            Value::Object(map) => {
                for (_, value) in map {
                    if let Some(value) = value.as_str() {
                        specifiers.push(("package.json:bin".to_string(), value.to_string()));
                    }
                }
            }
            _ => {}
        }
    }

    for (field, source) in [
        ("main", "package.json:main"),
        ("module", "package.json:module"),
    ] {
        if let Some(value) = manifest.get(field).and_then(Value::as_str) {
            specifiers.push((source.to_string(), value.to_string()));
        }
    }

    if let Some(exports) = manifest.get("exports") {
        let mut targets = Vec::new();
        collect_export_targets(exports, &mut targets);
        for target in targets {
            specifiers.push(("package.json:exports".to_string(), target));
        }
    }

    let mut seen = BTreeSet::new();
    specifiers.retain(|specifier| seen.insert(specifier.clone()));
    specifiers
}

fn collect_export_targets(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(target) => {
            if !is_declaration_or_asset(target) && !target.contains('*') {
                out.push(target.clone());
            }
        }
        Value::Object(map) => {
            for (key, value) in map {
                if matches!(key.as_str(), "types" | "typings") {
                    continue;
                }
                collect_export_targets(value, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_export_targets(item, out);
            }
        }
        _ => {}
    }
}

fn is_declaration_or_asset(target: &str) -> bool {
    let lower = target.to_ascii_lowercase();
    lower.ends_with(".css")
        || lower.ends_with(".json")
        || lower.ends_with(".map")
        || lower.ends_with(".d.ts")
        || lower.ends_with(".d.mts")
        || lower.ends_with(".d.cts")
}

fn index_module(root: &Path, package_directory: &str) -> Option<(String, &'static str)> {
    let mut bases = vec![(package_directory.to_string(), "index")];
    let src = if package_directory.is_empty() {
        "src".to_string()
    } else {
        format!("{package_directory}/src")
    };
    bases.push((src, "src-index"));

    for (base, source) in bases {
        for extension in SOURCE_EXTENSIONS {
            let module = if base.is_empty() {
                format!("index.{extension}")
            } else {
                format!("{base}/index.{extension}")
            };
            if root.join(&module).is_file() {
                return Some((module, source));
            }
        }
    }
    None
}

fn resolve_specifier(root: &Path, package_directory: &str, specifier: &str) -> Option<String> {
    let specifier = specifier.trim();
    if specifier.is_empty() || specifier.contains('*') {
        return None;
    }
    let relative = specifier.strip_prefix("./")?;
    let joined = join_and_normalize(package_directory, relative)?;

    for base in path_variants(&joined) {
        for file in file_variants(&base) {
            let candidate = root.join(&file);
            if candidate.is_file() && is_source_file(&candidate) {
                return Some(file);
            }
        }
    }
    None
}

fn join_and_normalize(package_directory: &str, relative: &str) -> Option<String> {
    let mut segments: Vec<&str> = package_directory
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    for segment in relative.split(['/', '\\']) {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop()?;
            }
            other => segments.push(other),
        }
    }
    Some(segments.join("/"))
}

fn path_variants(base: &str) -> Vec<String> {
    let mut variants = vec![base.to_string()];
    let segments: Vec<&str> = base.split('/').collect();
    let Some(first) = segments.first() else {
        return variants;
    };
    if OUTPUT_DIRECTORIES.contains(first) {
        let mut replaced = segments.clone();
        replaced[0] = "src";
        variants.push(replaced.join("/"));

        let mut remainder = segments[1..].to_vec();
        if let Some(next) = remainder.first() {
            if NESTED_OUTPUT_DIRECTORIES.contains(next) {
                remainder.remove(0);
            }
        }
        let dropped = remainder.join("/");
        variants.push(dropped.clone());
        if !dropped.is_empty() {
            variants.push(format!("src/{dropped}"));
        }
    }
    variants.sort();
    variants.dedup();
    variants
}

fn file_variants(path: &str) -> Vec<String> {
    let mut variants = Vec::new();
    let Some((stem, extension)) = split_extension(path) else {
        for extension in SOURCE_EXTENSIONS {
            variants.push(format!("{path}.{extension}"));
        }
        for extension in SOURCE_EXTENSIONS {
            variants.push(format!("{path}/index.{extension}"));
        }
        return variants;
    };

    if SOURCE_EXTENSIONS.contains(&extension) {
        variants.push(path.to_string());
    }
    if matches!(extension, "js" | "jsx" | "mjs" | "cjs") {
        variants.push(format!("{stem}.ts"));
        variants.push(format!("{stem}.tsx"));
    }

    variants
}

fn split_extension(path: &str) -> Option<(&str, &str)> {
    let file = path.rsplit('/').next()?;
    let dot = file.rfind('.')?;
    if dot == 0 {
        return None;
    }
    let stem_end = path.len() - (file.len() - dot);
    Some((&path[..stem_end], &path[stem_end + 1..]))
}

fn exported_names(source: &str) -> BTreeSet<String> {
    let tokens = tokenize(source);
    let mut names = BTreeSet::new();
    let mut index = 0;

    while index < tokens.len() {
        let TokenKind::Ident(word) = &tokens[index].kind else {
            index += 1;
            continue;
        };
        if word != "export" {
            index += 1;
            continue;
        }

        let mut cursor = index + 1;
        if matches!(
            tokens.get(cursor).map(|token| &token.kind),
            Some(TokenKind::Punct('{'))
        ) {
            cursor += 1;
            let mut depth = 1;
            while cursor < tokens.len() {
                match &tokens[cursor].kind {
                    TokenKind::Punct('{') => depth += 1,
                    TokenKind::Punct('}') => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    TokenKind::Ident(name) => {
                        if !matches!(name.as_str(), "as" | "type") {
                            names.insert(name.clone());
                        }
                    }
                    _ => {}
                }
                cursor += 1;
            }
        } else {
            while cursor < tokens.len() {
                match &tokens[cursor].kind {
                    TokenKind::Punct('{') | TokenKind::Punct(';') => break,
                    TokenKind::Ident(name) => {
                        if !EXPORT_KEYWORDS.contains(&name.as_str()) {
                            names.insert(name.clone());
                        }
                    }
                    _ => {}
                }
                cursor += 1;
            }
        }

        index = cursor.max(index + 1);
    }

    names
}
