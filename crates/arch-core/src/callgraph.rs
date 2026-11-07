//! ticket 04：符号提取与调用图。
//!
//! 符号 ID 规则：
//! - 命名声明：`<module>#<kind>:<name>`；同名重复声明按出现顺序追加 `@2`、`@3`。
//! - 类方法：`<module>#method:<Class>.<method>`。
//! - 匿名默认导出：`<module>#function:default`。
//! - 每个模块固定有一个 `<module>#module-toplevel`。
//!
//! 调用边只指向已存在的符号；无法归因的调用写入 `call-unattributed`，
//! 不做「强行归到模块顶层」的兜底。

use crate::config::{ProjectConfig, WorkspaceLookup};
use crate::imports::{tokenize, Token, TokenKind};
use crate::resolve::{relative_module_id, resolve_file_path, FileLookup};
use crate::{build_index, walk, RepoInfo, ScanError};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CallGraphArtifact {
    pub schema_version: u32,
    pub tool_version: String,
    pub repo: RepoInfo,
    pub stats: CallGraphStats,
    pub symbols: Vec<SymbolNode>,
    pub edges: Vec<CallEdge>,
    pub module_edges: Vec<ModuleCallEdge>,
    pub unresolved: Vec<CallUnresolved>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CallGraphStats {
    pub files: usize,
    pub symbols: usize,
    pub edges: usize,
    pub module_edges: usize,
    /// 无法归因到仓内符号的调用（`call-unattributed`）。
    pub unattributed_calls: usize,
    /// 导入解析失败（`dynamic-import` 及其他非字面量导入）。
    pub unresolved_imports: usize,
    /// 已知外部包绑定上的成员调用（`external-call`）。
    pub external_calls: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolNode {
    pub id: String,
    pub module: String,
    pub path: String,
    /// `function` / `component` / `class` / `method` / `module-toplevel`。
    pub kind: &'static str,
    pub name: String,
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CallEdge {
    pub source: String,
    pub target: String,
    /// `call` / `jsx` / `dynamic-import` / `require`。
    pub kind: String,
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleCallEdge {
    pub source: String,
    pub target: String,
    /// 投影到该模块对上的符号边种类集合，按字典序去重。
    pub kinds: Vec<String>,
    pub count: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CallUnresolved {
    pub module: String,
    pub expression: String,
    /// `call-unattributed`、`dynamic-import` 或 `external-call`。
    pub reason: &'static str,
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone)]
struct RawSymbol {
    kind: &'static str,
    name: String,
    start_tok: usize,
    end_tok: usize,
}

#[derive(Debug, Clone)]
struct LocalSymbol {
    id: String,
    kind: &'static str,
    name: String,
    start_tok: usize,
    end_tok: usize,
}

#[derive(Debug, Clone)]
enum ImportBinding {
    Named { module: String, imported: String },
    Default { module: String },
    Namespace { module: String },
    External,
}

/// 导入说明符的解析结果：仓内模块、已知外部包、或无法解析。
enum SpecifierTarget {
    Local(String),
    External,
    Unresolved,
}

struct ModuleScan {
    id: String,
    path: PathBuf,
    source: String,
    tokens: Vec<Token>,
    lines: LineIndex,
    symbols: Vec<LocalSymbol>,
    declarations: BTreeSet<usize>,
    imports: BTreeMap<String, ImportBinding>,
}

struct Lookup {
    by_module_name: BTreeMap<(String, String), String>,
    default_by_module: BTreeMap<String, String>,
}

struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    fn new(source: &str) -> Self {
        let mut starts = vec![0];
        for (offset, character) in source.char_indices() {
            if character == '\n' {
                starts.push(offset + 1);
            }
        }
        Self { starts }
    }

    fn position(&self, source: &str, offset: usize) -> (usize, usize) {
        let line = self.starts.partition_point(|start| *start <= offset);
        let line_start = self.starts[line.saturating_sub(1)];
        let column = source[line_start..offset].chars().count() + 1;
        (line, column)
    }
}

/// 构建调用图产物。只读取目标仓库，不写入。
pub fn build_call_graph(root: &Path, tool_version: &str) -> Result<CallGraphArtifact, ScanError> {
    let index = build_index(root, tool_version)?;
    let root = fs::canonicalize(root)
        .map_err(|error| ScanError::NotADirectory(format!("{}（{error}）", root.display())))?;
    let config = ProjectConfig::load(&root);

    let mut files = Vec::new();
    walk(&root, &root, &mut files)?;
    files.sort();
    // 声明文件没有运行时符号，也不应把接口签名计入调用噪声。
    files.retain(|path| !is_declaration_file(path));

    let mut modules = Vec::with_capacity(files.len());
    for path in &files {
        let id = crate::relative_slash_path(&root, path);
        let bytes = fs::read(path)
            .map_err(|error| ScanError::Io(format!("{}（{error}）", path.display())))?;
        let source = String::from_utf8_lossy(&bytes).to_string();
        let tokens = tokenize(&source);
        let lines = LineIndex::new(&source);
        let (raw_symbols, declarations) = scan_symbols(&tokens);
        let imports = parse_imports(&tokens, &source, &config, &root, path, &id);
        modules.push(ModuleScan {
            id,
            path: path.clone(),
            source,
            tokens,
            lines,
            symbols: Vec::new(),
            declarations,
            imports,
        });
        modules.last_mut().expect("module").symbols =
            assign_symbols(raw_symbols, modules.last().expect("module"));
    }

    let mut lookup_by_module_name = BTreeMap::new();
    let mut default_by_module = BTreeMap::new();
    for module in &modules {
        for symbol in &module.symbols {
            lookup_by_module_name
                .entry((module.id.clone(), symbol.name.clone()))
                .or_insert_with(|| symbol.id.clone());
            if symbol.name == "default" {
                default_by_module.insert(module.id.clone(), symbol.id.clone());
            }
        }
    }
    let lookup = Lookup {
        by_module_name: lookup_by_module_name,
        default_by_module,
    };

    let mut edge_keys = BTreeSet::new();
    let mut unresolved_keys = BTreeSet::new();
    for module in &modules {
        scan_calls(
            module,
            &lookup,
            &config,
            &root,
            &mut edge_keys,
            &mut unresolved_keys,
        );
    }

    let symbols = modules
        .iter()
        .flat_map(|module| {
            module
                .symbols
                .iter()
                .map(|symbol| {
                    let (line, column) = module
                        .tokens
                        .get(symbol.start_tok)
                        .map(|token| module.lines.position(&module.source, token.start))
                        .unwrap_or((1, 1));
                    SymbolNode {
                        id: symbol.id.clone(),
                        module: module.id.clone(),
                        path: module.id.clone(),
                        kind: symbol.kind,
                        name: symbol.name.clone(),
                        line,
                        column,
                    }
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    let edges = edge_keys
        .into_iter()
        .map(|(source, target, kind, line, column)| CallEdge {
            source,
            target,
            kind,
            line,
            column,
        })
        .collect::<Vec<_>>();
    let unresolved = unresolved_keys
        .into_iter()
        .map(
            |(module, expression, reason, line, column)| CallUnresolved {
                module,
                expression,
                reason,
                line,
                column,
            },
        )
        .collect::<Vec<_>>();

    let module_edges = aggregate_module_edges(&edges);
    let unattributed_calls = unresolved
        .iter()
        .filter(|entry| entry.reason == "call-unattributed")
        .count();
    let unresolved_imports = unresolved
        .iter()
        .filter(|entry| entry.reason == "dynamic-import")
        .count();
    let external_calls = unresolved
        .iter()
        .filter(|entry| entry.reason == "external-call")
        .count();

    Ok(CallGraphArtifact {
        schema_version: index.schema_version,
        tool_version: index.tool_version,
        repo: index.repo,
        stats: CallGraphStats {
            files: files.len(),
            symbols: symbols.len(),
            edges: edges.len(),
            module_edges: module_edges.len(),
            unattributed_calls,
            unresolved_imports,
            external_calls,
        },
        symbols,
        edges,
        module_edges,
        unresolved,
    })
}

fn is_declaration_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    let name = name.to_ascii_lowercase();
    name.ends_with(".d.ts") || name.ends_with(".d.mts") || name.ends_with(".d.cts")
}

fn assign_symbols(mut raw: Vec<RawSymbol>, module: &ModuleScan) -> Vec<LocalSymbol> {
    raw.sort_by_key(|symbol| symbol.start_tok);
    let mut counts = BTreeMap::<String, usize>::new();
    let mut symbols = Vec::with_capacity(raw.len() + 1);
    for symbol in raw {
        let base = format!("{}#{}:{}", module.id, symbol.kind, symbol.name);
        let count = counts.entry(base.clone()).or_insert(0);
        *count += 1;
        let id = if *count == 1 {
            base
        } else {
            format!("{base}@{}", *count)
        };
        symbols.push(LocalSymbol {
            id,
            kind: symbol.kind,
            name: symbol.name,
            start_tok: symbol.start_tok,
            end_tok: symbol.end_tok,
        });
    }
    symbols.push(LocalSymbol {
        id: format!("{}#module-toplevel", module.id),
        kind: "module-toplevel",
        name: "module".to_string(),
        start_tok: 0,
        end_tok: module.tokens.len().saturating_sub(1),
    });
    symbols
}

fn scan_symbols(tokens: &[Token]) -> (Vec<RawSymbol>, BTreeSet<usize>) {
    let mut symbols = Vec::new();
    let mut declarations = BTreeSet::new();
    let mut index = 0;

    while index < tokens.len() {
        let TokenKind::Ident(word) = &tokens[index].kind else {
            index += 1;
            continue;
        };
        let is_default = ident_at(tokens, index.wrapping_sub(1)) == Some("default");

        match word.as_str() {
            "function" => {
                if let Some((symbol, next)) =
                    parse_function(tokens, index, is_default, &mut declarations)
                {
                    symbols.push(symbol);
                    index = next;
                    continue;
                }
            }
            "class" => {
                if let Some((class, methods, next)) =
                    parse_class(tokens, index, is_default, &mut declarations)
                {
                    symbols.push(class);
                    symbols.extend(methods);
                    index = next;
                    continue;
                }
            }
            "const" | "let" | "var" => {
                if let Some((symbol, next)) = parse_variable(tokens, index) {
                    symbols.push(symbol);
                    index = next;
                    continue;
                }
            }
            "default" if ident_at(tokens, index.wrapping_sub(1)) == Some("export") => {
                if let Some((symbol, next)) = parse_default_arrow(tokens, index) {
                    symbols.push(symbol);
                    index = next;
                    continue;
                }
            }
            _ => {}
        }

        index += 1;
    }

    (symbols, declarations)
}

fn parse_function(
    tokens: &[Token],
    index: usize,
    is_default: bool,
    declarations: &mut BTreeSet<usize>,
) -> Option<(RawSymbol, usize)> {
    let mut cursor = index + 1;
    if punct_at(tokens, cursor, '*') {
        cursor += 1;
    }
    let (name, _position) = match ident_at(tokens, cursor) {
        Some(name) if name != "default" && !is_keyword(name) => {
            declarations.insert(cursor);
            (name.to_string(), cursor)
        }
        _ if is_default => ("default".to_string(), index),
        _ => return None,
    };
    let open = if punct_at(tokens, cursor, '(') {
        Some(cursor)
    } else if punct_at(tokens, cursor + 1, '(') {
        Some(cursor + 1)
    } else {
        None
    }?;
    let close = matching(tokens, open, '(', ')')?;

    let mut body = close + 1;
    while body < tokens.len() && body < close + 120 {
        if punct_at(tokens, body, '{') {
            break;
        }
        if punct_at(tokens, body, ';') {
            return None;
        }
        if matches!(
            ident_at(tokens, body),
            Some("export" | "function" | "class" | "const" | "let" | "var")
        ) {
            return None;
        }
        body += 1;
    }
    if !punct_at(tokens, body, '{') {
        return None;
    }
    let end = matching(tokens, body, '{', '}')?;
    let kind = if is_component_name(&name) && body_has_jsx(tokens, body, end) {
        "component"
    } else {
        "function"
    };
    Some((
        RawSymbol {
            kind,
            name,
            start_tok: declaration_start(tokens, index),
            end_tok: end,
        },
        end + 1,
    ))
}

fn parse_class(
    tokens: &[Token],
    index: usize,
    is_default: bool,
    declarations: &mut BTreeSet<usize>,
) -> Option<(RawSymbol, Vec<RawSymbol>, usize)> {
    let cursor = index + 1;
    let (name, _position) = match ident_at(tokens, cursor) {
        Some(name) if name != "extends" && name != "implements" && !is_keyword(name) => {
            declarations.insert(cursor);
            (name.to_string(), cursor)
        }
        _ if is_default => ("default".to_string(), index),
        _ => return None,
    };

    let mut open = cursor;
    while open < tokens.len() && open < cursor + 80 {
        if punct_at(tokens, open, '{') {
            break;
        }
        if punct_at(tokens, open, ';') {
            return None;
        }
        open += 1;
    }
    if !punct_at(tokens, open, '{') {
        return None;
    }
    let close = matching(tokens, open, '{', '}')?;

    let mut methods = Vec::new();
    let mut depth = 1usize;
    let mut method_cursor = open + 1;
    while method_cursor < close {
        match &tokens[method_cursor].kind {
            TokenKind::Punct('{') => depth += 1,
            TokenKind::Punct('}') => depth = depth.saturating_sub(1),
            TokenKind::Ident(word) if depth == 1 => {
                if matches!(
                    word.as_str(),
                    "async"
                        | "static"
                        | "get"
                        | "set"
                        | "public"
                        | "private"
                        | "protected"
                        | "readonly"
                        | "abstract"
                        | "override"
                ) {
                    method_cursor += 1;
                    continue;
                }
                if is_keyword(word) || !punct_at(tokens, method_cursor + 1, '(') {
                    method_cursor += 1;
                    continue;
                }
                declarations.insert(method_cursor);
                let params_open = method_cursor + 1;
                let params_close = matching(tokens, params_open, '(', ')')?;
                let mut method_body = params_close + 1;
                while method_body < close && !punct_at(tokens, method_body, '{') {
                    method_body += 1;
                }
                let method_end = if punct_at(tokens, method_body, '{') {
                    matching(tokens, method_body, '{', '}')?
                } else {
                    params_close
                };
                methods.push(RawSymbol {
                    kind: "method",
                    name: format!("{name}.{word}"),
                    start_tok: method_cursor,
                    end_tok: method_end,
                });
                method_cursor = method_end + 1;
                continue;
            }
            _ => {}
        }
        method_cursor += 1;
    }

    Some((
        RawSymbol {
            kind: "class",
            name,
            start_tok: declaration_start(tokens, index),
            end_tok: close,
        },
        methods,
        close + 1,
    ))
}

fn parse_variable(tokens: &[Token], index: usize) -> Option<(RawSymbol, usize)> {
    let name_index = index + 1;
    let name = ident_at(tokens, name_index)?;
    if is_keyword(name) {
        return None;
    }
    let mut equals = name_index + 1;
    while equals < tokens.len() && equals < name_index + 30 {
        if punct_at(tokens, equals, '=') {
            break;
        }
        if punct_at(tokens, equals, ';') {
            return None;
        }
        equals += 1;
    }
    if !punct_at(tokens, equals, '=') {
        return None;
    }

    let mut cursor = equals + 1;
    if ident_at(tokens, cursor) == Some("async") {
        cursor += 1;
    }
    if ident_at(tokens, cursor) == Some("function") {
        return None;
    }

    let body_start;
    if punct_at(tokens, cursor, '(') {
        let params_close = matching(tokens, cursor, '(', ')')?;
        if !punct_at(tokens, params_close + 1, '=') || !punct_at(tokens, params_close + 2, '>') {
            return None;
        }
        body_start = params_close + 3;
    } else {
        ident_at(tokens, cursor)?;
        if !punct_at(tokens, cursor + 1, '=') || !punct_at(tokens, cursor + 2, '>') {
            return None;
        }
        body_start = cursor + 3;
    }

    let end = if punct_at(tokens, body_start, '{') {
        matching(tokens, body_start, '{', '}')?
    } else {
        statement_end(tokens, body_start)
    };
    let kind = if is_component_name(name) && body_has_jsx(tokens, body_start, end) {
        "component"
    } else {
        "function"
    };
    Some((
        RawSymbol {
            kind,
            name: name.to_string(),
            start_tok: declaration_start(tokens, index),
            end_tok: end,
        },
        end + 1,
    ))
}

fn parse_default_arrow(tokens: &[Token], index: usize) -> Option<(RawSymbol, usize)> {
    let mut cursor = index + 1;
    let body_start;
    if punct_at(tokens, cursor, '(') {
        let params_close = matching(tokens, cursor, '(', ')')?;
        if !punct_at(tokens, params_close + 1, '=') || !punct_at(tokens, params_close + 2, '>') {
            return None;
        }
        body_start = params_close + 3;
    } else if ident_at(tokens, cursor).is_some() {
        if !punct_at(tokens, cursor + 1, '=') || !punct_at(tokens, cursor + 2, '>') {
            return None;
        }
        cursor += 3;
        body_start = cursor;
    } else {
        return None;
    }

    let end = if punct_at(tokens, body_start, '{') {
        matching(tokens, body_start, '{', '}')?
    } else {
        statement_end(tokens, body_start)
    };
    let kind = if body_has_jsx(tokens, body_start, end) {
        "component"
    } else {
        "function"
    };
    Some((
        RawSymbol {
            kind,
            name: "default".to_string(),
            start_tok: declaration_start(tokens, index),
            end_tok: end,
        },
        end + 1,
    ))
}

fn declaration_start(tokens: &[Token], index: usize) -> usize {
    if ident_at(tokens, index.wrapping_sub(1)) == Some("export") {
        index - 1
    } else if ident_at(tokens, index.wrapping_sub(1)) == Some("default")
        && ident_at(tokens, index.wrapping_sub(2)) == Some("export")
    {
        index - 2
    } else {
        index
    }
}

fn statement_end(tokens: &[Token], start: usize) -> usize {
    let mut depth = 0usize;
    for index in start..tokens.len() {
        match &tokens[index].kind {
            TokenKind::Punct('(' | '[' | '{') => depth += 1,
            TokenKind::Punct(')' | ']' | '}') => depth = depth.saturating_sub(1),
            TokenKind::Punct(';') if depth == 0 => return index,
            TokenKind::Ident(word)
                if depth == 0
                    && (index > start)
                    && matches!(
                        word.as_str(),
                        "export" | "function" | "class" | "const" | "let" | "var"
                    ) =>
            {
                return index - 1;
            }
            _ => {}
        }
    }
    tokens.len().saturating_sub(1)
}

fn body_has_jsx(tokens: &[Token], start: usize, end: usize) -> bool {
    let end = end.min(tokens.len().saturating_sub(1));
    for index in start..end {
        if punct_at(tokens, index, '<') {
            if ident_at(tokens, index + 1).is_some()
                && matches!(
                    token_kind(tokens, index + 2),
                    Some(TokenKind::Punct('>'))
                        | Some(TokenKind::Punct('/'))
                        | Some(TokenKind::Ident(_))
                        | Some(TokenKind::Punct('{'))
                )
            {
                return true;
            }
        }
    }
    false
}

fn parse_imports(
    tokens: &[Token],
    _source: &str,
    config: &ProjectConfig,
    root: &Path,
    source_path: &Path,
    _module_id: &str,
) -> BTreeMap<String, ImportBinding> {
    let mut bindings = BTreeMap::new();
    let mut index = 0;

    while index < tokens.len() {
        let TokenKind::Ident(word) = &tokens[index].kind else {
            index += 1;
            continue;
        };
        if word != "import" || punct_at(tokens, index + 1, '.') || punct_at(tokens, index + 1, '(')
        {
            index += 1;
            continue;
        }

        let mut cursor = index + 1;
        if ident_at(tokens, cursor) == Some("type") {
            cursor += 1;
        }
        if matches!(token_kind(tokens, cursor), Some(TokenKind::Str(_))) {
            index = cursor + 1;
            continue;
        }

        let mut pending = Vec::new();
        if punct_at(tokens, cursor, '*') {
            cursor += 1;
            if ident_at(tokens, cursor) == Some("as") {
                cursor += 1;
            }
            if let Some(local) = ident_at(tokens, cursor) {
                pending.push((local.to_string(), BindingForm::Namespace));
                cursor += 1;
            }
        } else {
            if let Some(local) = ident_at(tokens, cursor) {
                if !is_keyword(local) {
                    pending.push((local.to_string(), BindingForm::Default));
                    cursor += 1;
                    if punct_at(tokens, cursor, ',') {
                        cursor += 1;
                    }
                }
            }
            if punct_at(tokens, cursor, '{') {
                let close = match matching(tokens, cursor, '{', '}') {
                    Some(close) => close,
                    None => break,
                };
                let mut entry = cursor + 1;
                while entry < close {
                    if ident_at(tokens, entry) == Some("type") {
                        entry += 1;
                        continue;
                    }
                    let Some(imported) = ident_at(tokens, entry) else {
                        entry += 1;
                        continue;
                    };
                    let mut local = imported.to_string();
                    entry += 1;
                    if ident_at(tokens, entry) == Some("as") {
                        if let Some(alias) = ident_at(tokens, entry + 1) {
                            local = alias.to_string();
                        }
                        entry += 2;
                    }
                    pending.push((local, BindingForm::Named(imported.to_string())));
                    while entry < close && !punct_at(tokens, entry, ',') {
                        entry += 1;
                    }
                    entry += 1;
                }
                cursor = close + 1;
            }
        }

        let mut specifier = None;
        let mut scan = cursor;
        while scan < tokens.len() && scan < cursor + 400 {
            match &tokens[scan].kind {
                TokenKind::Str(value) => {
                    specifier = Some(value.clone());
                    break;
                }
                TokenKind::Ident(value) if value == "from" => {
                    if let Some(Token {
                        kind: TokenKind::Str(value),
                        ..
                    }) = tokens.get(scan + 1)
                    {
                        specifier = Some(value.clone());
                        break;
                    }
                }
                TokenKind::Punct(';') => break,
                TokenKind::Ident(value)
                    if matches!(
                        value.as_str(),
                        "import" | "export" | "const" | "let" | "var" | "function" | "class"
                    ) =>
                {
                    break;
                }
                _ => {}
            }
            scan += 1;
        }

        if let Some(specifier) = specifier {
            match classify_specifier(config, root, source_path, &specifier) {
                SpecifierTarget::Local(module) => {
                    for (local, form) in pending {
                        let binding = match form {
                            BindingForm::Named(imported) => ImportBinding::Named {
                                module: module.clone(),
                                imported,
                            },
                            BindingForm::Default => ImportBinding::Default {
                                module: module.clone(),
                            },
                            BindingForm::Namespace => ImportBinding::Namespace {
                                module: module.clone(),
                            },
                        };
                        bindings.insert(local, binding);
                    }
                }
                SpecifierTarget::External => {
                    for (local, _) in pending {
                        bindings.insert(local, ImportBinding::External);
                    }
                }
                SpecifierTarget::Unresolved => {}
            }
        }
        index = scan + 1;
    }

    bindings
}

enum BindingForm {
    Named(String),
    Default,
    Namespace,
}

fn classify_specifier(
    config: &ProjectConfig,
    root: &Path,
    source_path: &Path,
    specifier: &str,
) -> SpecifierTarget {
    if specifier.is_empty() {
        return SpecifierTarget::Unresolved;
    }
    if specifier.starts_with('.') {
        let base = source_path.parent().unwrap_or(root);
        return match resolve_file_path(base, specifier) {
            FileLookup::Found(file) => relative_module_id(root, &file)
                .map(SpecifierTarget::Local)
                .unwrap_or(SpecifierTarget::Unresolved),
            _ => SpecifierTarget::Unresolved,
        };
    }
    if specifier.starts_with('/') {
        return match resolve_file_path(root, specifier.trim_start_matches('/')) {
            FileLookup::Found(file) => relative_module_id(root, &file)
                .map(SpecifierTarget::Local)
                .unwrap_or(SpecifierTarget::Unresolved),
            _ => SpecifierTarget::Unresolved,
        };
    }
    if let Some(targets) = config.alias_targets(specifier) {
        for target in targets {
            if let FileLookup::Found(file) = resolve_file_path(&config.base_url, &target) {
                if let Some(id) = relative_module_id(root, &file) {
                    return SpecifierTarget::Local(id);
                }
            }
        }
        return SpecifierTarget::Unresolved;
    }
    match config.workspace_lookup(specifier) {
        WorkspaceLookup::Resolved(file) => relative_module_id(root, &file)
            .map(SpecifierTarget::Local)
            .unwrap_or(SpecifierTarget::Unresolved),
        WorkspaceLookup::Missing | WorkspaceLookup::UnsupportedExtension => {
            SpecifierTarget::Unresolved
        }
        WorkspaceLookup::Unknown => {
            if crate::external_package_name(specifier).is_some() {
                SpecifierTarget::External
            } else {
                SpecifierTarget::Unresolved
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn scan_calls(
    module: &ModuleScan,
    lookup: &Lookup,
    config: &ProjectConfig,
    root: &Path,
    edges: &mut BTreeSet<(String, String, String, usize, usize)>,
    unresolved: &mut BTreeSet<(String, String, &'static str, usize, usize)>,
) {
    let mut index = 0;
    while index < module.tokens.len() {
        if module.declarations.contains(&index) {
            index += 1;
            continue;
        }

        if let TokenKind::Ident(word) = &module.tokens[index].kind {
            if (word == "import" || word == "require") && punct_at(&module.tokens, index + 1, '(') {
                scan_dynamic(module, index, word, config, root, edges, unresolved);
                index += 1;
                continue;
            }

            if is_keyword(word) || !punct_at(&module.tokens, index + 1, '(') {
                index += 1;
                continue;
            }

            let (line, column) = module
                .lines
                .position(&module.source, module.tokens[index].start);
            let source = enclosing_symbol(module, index);

            if punct_at(&module.tokens, index.wrapping_sub(1), '.') {
                let receiver = index.wrapping_sub(2);
                resolve_member(
                    module, index, receiver, source, lookup, line, column, edges, unresolved,
                );
                index += 1;
                continue;
            }

            if let Some(target) = resolve_bare(module, word, lookup) {
                edges.insert((source, target, "call".to_string(), line, column));
            } else {
                unresolved.insert((
                    module.id.clone(),
                    word.clone(),
                    "call-unattributed",
                    line,
                    column,
                ));
            }
        }

        if punct_at(&module.tokens, index, '<') {
            if let Some(name) = ident_at(&module.tokens, index + 1) {
                if is_component_name(name)
                    && matches!(
                        token_kind(&module.tokens, index + 2),
                        Some(TokenKind::Punct('>'))
                            | Some(TokenKind::Punct('/'))
                            | Some(TokenKind::Ident(_))
                            | Some(TokenKind::Punct('{'))
                    )
                {
                    let (line, column) = module
                        .lines
                        .position(&module.source, module.tokens[index].start);
                    let source = enclosing_symbol(module, index);
                    if let Some(target) = resolve_binding(module, name, lookup) {
                        edges.insert((source, target, "jsx".to_string(), line, column));
                    } else {
                        unresolved.insert((
                            module.id.clone(),
                            format!("<{name}>"),
                            "call-unattributed",
                            line,
                            column,
                        ));
                    }
                }
            }
        }

        index += 1;
    }
}

#[allow(clippy::too_many_arguments)]
fn resolve_member(
    module: &ModuleScan,
    index: usize,
    receiver: usize,
    source: String,
    lookup: &Lookup,
    line: usize,
    column: usize,
    edges: &mut BTreeSet<(String, String, String, usize, usize)>,
    unresolved: &mut BTreeSet<(String, String, &'static str, usize, usize)>,
) {
    let method = ident_at(&module.tokens, index).unwrap_or_default();
    let mut external_receiver = false;
    if let Some(receiver_name) = ident_at(&module.tokens, receiver) {
        if receiver_name == "this" {
            let current = enclosing_symbol(module, index);
            if let Some(method_symbol) = module.symbols.iter().find(|symbol| {
                symbol.id == current
                    && symbol.kind == "method"
                    && symbol.name.ends_with(&format!(".{method}"))
            }) {
                edges.insert((
                    current.clone(),
                    method_symbol.id.clone(),
                    "call".to_string(),
                    line,
                    column,
                ));
                return;
            }
        }
        if let Some(ImportBinding::Namespace {
            module: target_module,
        }) = module.imports.get(receiver_name)
        {
            if let Some(target) = lookup
                .by_module_name
                .get(&(target_module.clone(), method.to_string()))
            {
                edges.insert((source, target.clone(), "call".to_string(), line, column));
                return;
            }
        }
        external_receiver = matches!(
            module.imports.get(receiver_name),
            Some(ImportBinding::External)
        );
    }

    let start = module
        .tokens
        .get(receiver)
        .map(|token| token.start)
        .unwrap_or_else(|| module.tokens[index].start);
    let expression = module.source[start..module.tokens[index].end]
        .trim()
        .to_string();
    let reason = if external_receiver {
        "external-call"
    } else {
        "call-unattributed"
    };
    unresolved.insert((module.id.clone(), expression, reason, line, column));
}

fn resolve_bare(module: &ModuleScan, name: &str, lookup: &Lookup) -> Option<String> {
    if let Some(local) = module
        .symbols
        .iter()
        .find(|symbol| symbol.name == name && symbol.kind != "method")
    {
        return Some(local.id.clone());
    }
    resolve_binding(module, name, lookup)
}

fn resolve_binding(module: &ModuleScan, name: &str, lookup: &Lookup) -> Option<String> {
    match module.imports.get(name) {
        Some(ImportBinding::Named {
            module: target_module,
            imported,
        }) => lookup
            .by_module_name
            .get(&(target_module.clone(), imported.clone()))
            .cloned(),
        Some(ImportBinding::Default {
            module: target_module,
        }) => lookup.default_by_module.get(target_module).cloned(),
        Some(ImportBinding::Namespace { .. }) | Some(ImportBinding::External) | None => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn scan_dynamic(
    module: &ModuleScan,
    index: usize,
    keyword: &str,
    config: &ProjectConfig,
    root: &Path,
    edges: &mut BTreeSet<(String, String, String, usize, usize)>,
    unresolved: &mut BTreeSet<(String, String, &'static str, usize, usize)>,
) {
    let (line, column) = module
        .lines
        .position(&module.source, module.tokens[index].start);
    let source = enclosing_symbol(module, index);
    let kind = if keyword == "import" {
        "dynamic-import"
    } else {
        "require"
    };

    let literal = match token_kind(&module.tokens, index + 2) {
        Some(TokenKind::Str(specifier)) => Some(specifier.clone()),
        Some(TokenKind::Template {
            text,
            has_expression: false,
        }) => Some(text.clone()),
        _ => None,
    };

    if let Some(specifier) = literal {
        if let SpecifierTarget::Local(module_id) =
            classify_specifier(config, root, &module.path, &specifier)
        {
            edges.insert((
                source,
                format!("{module_id}#module-toplevel"),
                kind.to_string(),
                line,
                column,
            ));
        }
        return;
    }

    let open = index + 1;
    let close = matching(&module.tokens, open, '(', ')').unwrap_or(open);
    let start = module.tokens.get(open + 1).map(|token| token.start);
    let end = module.tokens.get(close).map(|token| token.start);
    let expression = match (start, end) {
        (Some(start), Some(end)) if start < end => module.source[start..end].trim().to_string(),
        _ => String::new(),
    };
    let reason = if keyword == "import" {
        "dynamic-import"
    } else {
        "call-unattributed"
    };
    unresolved.insert((module.id.clone(), expression, reason, line, column));
}

fn enclosing_symbol(module: &ModuleScan, token_index: usize) -> String {
    let mut best: Option<&LocalSymbol> = None;
    for symbol in &module.symbols {
        if symbol.start_tok <= token_index && token_index <= symbol.end_tok {
            let span = symbol.end_tok.saturating_sub(symbol.start_tok);
            if best.map(|current| span < current.end_tok.saturating_sub(current.start_tok))
                != Some(false)
            {
                best = Some(symbol);
            }
        }
    }
    best.map(|symbol| symbol.id.clone())
        .unwrap_or_else(|| format!("{}#module-toplevel", module.id))
}

fn aggregate_module_edges(edges: &[CallEdge]) -> Vec<ModuleCallEdge> {
    let mut grouped = BTreeMap::<(String, String), (BTreeSet<String>, usize)>::new();
    for edge in edges {
        let source = edge.source.split('#').next().unwrap_or("").to_string();
        let target = edge.target.split('#').next().unwrap_or("").to_string();
        if source == target {
            continue;
        }
        let entry = grouped.entry((source, target)).or_default();
        entry.0.insert(edge.kind.clone());
        entry.1 += 1;
    }
    grouped
        .into_iter()
        .map(|((source, target), (kinds, count))| ModuleCallEdge {
            source,
            target,
            kinds: kinds.into_iter().collect(),
            count,
        })
        .collect()
}

fn token_kind(tokens: &[Token], index: usize) -> Option<&TokenKind> {
    tokens.get(index).map(|token| &token.kind)
}

fn ident_at(tokens: &[Token], index: usize) -> Option<&str> {
    match tokens.get(index).map(|token| &token.kind) {
        Some(TokenKind::Ident(value)) => Some(value.as_str()),
        _ => None,
    }
}

fn punct_at(tokens: &[Token], index: usize, expected: char) -> bool {
    matches!(token_kind(tokens, index), Some(TokenKind::Punct(value)) if *value == expected)
}

fn matching(tokens: &[Token], open: usize, open_char: char, close_char: char) -> Option<usize> {
    if !punct_at(tokens, open, open_char) {
        return None;
    }
    let mut depth = 0usize;
    for index in open..tokens.len() {
        if punct_at(tokens, index, open_char) {
            depth += 1;
        } else if punct_at(tokens, index, close_char) {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some(index);
            }
        }
    }
    None
}

fn is_component_name(name: &str) -> bool {
    name.chars()
        .next()
        .map(|character| character.is_uppercase())
        .unwrap_or(false)
}

fn is_keyword(word: &str) -> bool {
    matches!(
        word,
        "if" | "for"
            | "while"
            | "switch"
            | "catch"
            | "return"
            | "typeof"
            | "void"
            | "delete"
            | "await"
            | "yield"
            | "new"
            | "in"
            | "of"
            | "instanceof"
            | "throw"
            | "case"
            | "do"
            | "else"
            | "super"
            | "this"
            | "try"
            | "finally"
            | "with"
            | "debugger"
            | "function"
            | "class"
            | "extends"
            | "implements"
            | "import"
            | "export"
            | "default"
            | "const"
            | "let"
            | "var"
            | "type"
            | "interface"
            | "enum"
            | "namespace"
    )
}
