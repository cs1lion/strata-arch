//! 轻量 TS/JS 导入扫描：跳过注释与字符串字面量，只提取可静态识别的导入说明符。
//!
//! 这里不做语法分析，目标是稳定地找到 `import` / `export ... from` / `import()` /
//! `require()`，并区分字面量与无法静态求值的表达式。表达式类导入由调用方写入
//! `unresolved`，不做猜测。

/// 导入语法形态；同时决定依赖边的 `kind`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportKind {
    Static,
    Dynamic,
    Require,
}

impl ImportKind {
    pub fn edge_kind(self) -> &'static str {
        match self {
            ImportKind::Static => "import",
            ImportKind::Dynamic => "dynamic-import",
            ImportKind::Require => "require",
        }
    }
}

/// 扫描结果：字面量可直接解析，表达式只能记入 `unresolved`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScannedImport {
    Literal {
        specifier: String,
        kind: ImportKind,
    },
    Expression {
        expression: String,
        kind: ImportKind,
    },
}

#[derive(Debug, Clone)]
enum TokenKind {
    Ident(String),
    Str(String),
    Template { text: String, has_expression: bool },
    Punct(char),
    Other,
}

#[derive(Debug, Clone)]
struct Token {
    kind: TokenKind,
    start: usize,
    end: usize,
}

/// 扫描源码中的导入。注释、字符串、模板字面量与正则字面量内的文本不会被误判。
pub fn scan_imports(source: &str) -> Vec<ScannedImport> {
    let tokens = tokenize(source);
    let mut imports = Vec::new();
    let mut index = 0;

    while index < tokens.len() {
        let TokenKind::Ident(word) = &tokens[index].kind else {
            index += 1;
            continue;
        };

        match word.as_str() {
            "import" => {
                if matches!(token_kind(&tokens, index + 1), Some(TokenKind::Punct('.'))) {
                    // `import.meta`
                    index += 1;
                    continue;
                }
                if matches!(token_kind(&tokens, index + 1), Some(TokenKind::Punct('('))) {
                    let (import, next) =
                        parse_call(source, &tokens, index + 1, ImportKind::Dynamic);
                    imports.push(import);
                    index = next;
                    continue;
                }
                if let Some((specifier, next)) = find_import_source(&tokens, index + 1) {
                    imports.push(ScannedImport::Literal {
                        specifier,
                        kind: ImportKind::Static,
                    });
                    index = next;
                    continue;
                }
            }
            "export" => {
                if let Some((specifier, next)) = find_import_source(&tokens, index + 1) {
                    imports.push(ScannedImport::Literal {
                        specifier,
                        kind: ImportKind::Static,
                    });
                    index = next;
                    continue;
                }
            }
            "require" => {
                if matches!(token_kind(&tokens, index + 1), Some(TokenKind::Punct('('))) {
                    let (import, next) =
                        parse_call(source, &tokens, index + 1, ImportKind::Require);
                    imports.push(import);
                    index = next;
                    continue;
                }
            }
            _ => {}
        }

        index += 1;
    }

    imports
}

fn token_kind(tokens: &[Token], index: usize) -> Option<&TokenKind> {
    tokens.get(index).map(|token| &token.kind)
}

/// 从 `from` 或直接字符串中取得导入说明符，遇到语句边界即停止。
fn find_import_source(tokens: &[Token], start: usize) -> Option<(String, usize)> {
    if let Some(Token {
        kind: TokenKind::Str(specifier),
        ..
    }) = tokens.get(start)
    {
        return Some((specifier.clone(), start + 1));
    }

    let limit = (start + 400).min(tokens.len());
    for index in start..limit {
        match &tokens[index].kind {
            TokenKind::Punct(';') => return None,
            TokenKind::Ident(word)
                if matches!(
                    word.as_str(),
                    "import" | "export" | "const" | "let" | "var" | "function" | "class" | "return"
                ) =>
            {
                return None;
            }
            TokenKind::Ident(word) if word == "from" => {
                if let Some(Token {
                    kind: TokenKind::Str(specifier),
                    ..
                }) = tokens.get(index + 1)
                {
                    return Some((specifier.clone(), index + 2));
                }
            }
            _ => {}
        }
    }
    None
}

/// 解析 `import(...)` / `require(...)` 的参数。
fn parse_call(
    source: &str,
    tokens: &[Token],
    open: usize,
    kind: ImportKind,
) -> (ScannedImport, usize) {
    match tokens.get(open + 1) {
        Some(Token {
            kind: TokenKind::Str(specifier),
            ..
        }) => (
            ScannedImport::Literal {
                specifier: specifier.clone(),
                kind,
            },
            open + 2,
        ),
        Some(Token {
            kind:
                TokenKind::Template {
                    text,
                    has_expression: false,
                },
            ..
        }) => (
            ScannedImport::Literal {
                specifier: text.clone(),
                kind,
            },
            open + 2,
        ),
        Some(Token { start, end, .. }) => {
            let mut depth = 1usize;
            let mut index = open + 1;
            while index < tokens.len() {
                match &tokens[index].kind {
                    TokenKind::Punct('(') => depth += 1,
                    TokenKind::Punct(')') => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                index += 1;
            }

            let expression_end = if index < tokens.len() {
                tokens[index].start
            } else {
                *end
            };
            let expression = source[*start..expression_end].trim().to_string();
            let next = if index < tokens.len() {
                index + 1
            } else {
                index
            };
            (ScannedImport::Expression { expression, kind }, next)
        }
        None => (
            ScannedImport::Expression {
                expression: String::new(),
                kind,
            },
            open + 1,
        ),
    }
}

fn tokenize(source: &str) -> Vec<Token> {
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0;

    while index < bytes.len() {
        let byte = bytes[index];

        if byte.is_ascii_whitespace() {
            index += 1;
            continue;
        }

        if byte == b'/' {
            if index + 1 < bytes.len() && bytes[index + 1] == b'/' {
                index += 2;
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
                continue;
            }
            if index + 1 < bytes.len() && bytes[index + 1] == b'*' {
                let mut cursor = index + 2;
                while cursor + 1 < bytes.len()
                    && !(bytes[cursor] == b'*' && bytes[cursor + 1] == b'/')
                {
                    cursor += 1;
                }
                index = if cursor + 1 < bytes.len() {
                    cursor + 2
                } else {
                    bytes.len()
                };
                continue;
            }
            if regex_allowed(tokens.last()) {
                let end = skip_regex(bytes, index);
                if end > index + 1 {
                    tokens.push(Token {
                        kind: TokenKind::Other,
                        start: index,
                        end,
                    });
                    index = end;
                    continue;
                }
            }
            tokens.push(Token {
                kind: TokenKind::Punct('/'),
                start: index,
                end: index + 1,
            });
            index += 1;
            continue;
        }

        if byte == b'\'' || byte == b'"' {
            // JSX 文本里的撇号（如 `Don't`）紧跟在字母后，按普通文本处理，
            // 避免被当成字符串起点而吞掉后续代码。
            if byte == b'\'' && index > 0 && is_ident_tail(bytes[index - 1]) {
                tokens.push(Token {
                    kind: TokenKind::Punct('\''),
                    start: index,
                    end: index + 1,
                });
                index += 1;
                continue;
            }
            let (text, end) = parse_string(source, index);
            tokens.push(Token {
                kind: TokenKind::Str(text),
                start: index,
                end,
            });
            index = end;
            continue;
        }

        if byte == b'`' {
            let (text, has_expression, end) = parse_template(source, index);
            tokens.push(Token {
                kind: TokenKind::Template {
                    text,
                    has_expression,
                },
                start: index,
                end,
            });
            index = end;
            continue;
        }

        if is_ident_start(byte) {
            let mut end = index + 1;
            while end < bytes.len() && is_ident_tail(bytes[end]) {
                end += 1;
            }
            tokens.push(Token {
                kind: TokenKind::Ident(source[index..end].to_string()),
                start: index,
                end,
            });
            index = end;
            continue;
        }

        if byte.is_ascii_digit() {
            let mut end = index + 1;
            while end < bytes.len()
                && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'.' || bytes[end] == b'_')
            {
                end += 1;
            }
            tokens.push(Token {
                kind: TokenKind::Other,
                start: index,
                end,
            });
            index = end;
            continue;
        }

        let character = source[index..].chars().next().unwrap_or('\u{fffd}');
        let end = index + character.len_utf8();
        tokens.push(Token {
            kind: TokenKind::Punct(character),
            start: index,
            end,
        });
        index = end;
    }

    tokens
}

fn is_ident_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte == b'$'
}

fn is_ident_tail(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$'
}

/// 只有出现在表达式起点位置时 `/` 才可能是正则字面量。
fn regex_allowed(last: Option<&Token>) -> bool {
    match last.map(|token| &token.kind) {
        None => true,
        Some(TokenKind::Punct(character)) => matches!(
            character,
            '(' | ','
                | '='
                | ':'
                | ';'
                | '!'
                | '&'
                | '|'
                | '?'
                | '+'
                | '-'
                | '*'
                | '%'
                | '{'
                | '['
                | '}'
                | ']'
                | '~'
                | '^'
        ),
        Some(TokenKind::Ident(word)) => matches!(
            word.as_str(),
            "return"
                | "case"
                | "delete"
                | "typeof"
                | "void"
                | "in"
                | "of"
                | "new"
                | "else"
                | "do"
                | "yield"
                | "await"
                | "throw"
        ),
        _ => false,
    }
}

fn skip_regex(bytes: &[u8], start: usize) -> usize {
    let mut index = start + 1;
    let mut in_class = false;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'[' => {
                in_class = true;
                index += 1;
            }
            b']' => {
                in_class = false;
                index += 1;
            }
            b'/' if !in_class => {
                index += 1;
                while index < bytes.len() && bytes[index].is_ascii_alphabetic() {
                    index += 1;
                }
                return index;
            }
            b'\n' => return start + 1,
            _ => index += 1,
        }
    }
    start + 1
}

fn parse_string(source: &str, start: usize) -> (String, usize) {
    let quote = source.as_bytes()[start] as char;
    let mut text = String::new();
    let mut chars = source[start + 1..].char_indices();

    while let Some((offset, character)) = chars.next() {
        let absolute = start + 1 + offset;
        if character == quote {
            return (text, absolute + character.len_utf8());
        }
        if character == '\\' {
            match chars.next() {
                Some((_, escaped)) => match escaped {
                    'n' => text.push('\n'),
                    'r' => text.push('\r'),
                    't' => text.push('\t'),
                    'b' => text.push('\u{0008}'),
                    'f' => text.push('\u{000c}'),
                    other => text.push(other),
                },
                None => break,
            }
            continue;
        }
        if character == '\n' {
            return (text, absolute);
        }
        text.push(character);
    }

    (text, source.len())
}

fn parse_template(source: &str, start: usize) -> (String, bool, usize) {
    let mut text = String::new();
    let mut has_expression = false;
    let bytes = source.as_bytes();
    let mut index = start + 1;

    while index < bytes.len() {
        match bytes[index] {
            b'\\' => {
                if index + 1 < bytes.len() {
                    text.push(bytes[index + 1] as char);
                    index += 2;
                } else {
                    index += 1;
                }
            }
            b'`' => return (text, has_expression, index + 1),
            b'$' if index + 1 < bytes.len() && bytes[index + 1] == b'{' => {
                has_expression = true;
                index = skip_template_expression(source, index + 2);
            }
            _ => {
                let character = source[index..].chars().next().unwrap_or('\u{fffd}');
                text.push(character);
                index += character.len_utf8();
            }
        }
    }

    (text, has_expression, source.len())
}

/// 跳过模板字面量 `${...}` 内的表达式，处理嵌套模板、字符串与注释。
fn skip_template_expression(source: &str, start: usize) -> usize {
    let bytes = source.as_bytes();
    let mut index = start;
    let mut depth = 1usize;

    while index < bytes.len() {
        match bytes[index] {
            b'\'' | b'"' => index = parse_string(source, index).1,
            b'`' => index = parse_template(source, index).2,
            b'/' if index + 1 < bytes.len() && bytes[index + 1] == b'/' => {
                index += 2;
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'/' if index + 1 < bytes.len() && bytes[index + 1] == b'*' => {
                let mut cursor = index + 2;
                while cursor + 1 < bytes.len()
                    && !(bytes[cursor] == b'*' && bytes[cursor + 1] == b'/')
                {
                    cursor += 1;
                }
                index = if cursor + 1 < bytes.len() {
                    cursor + 2
                } else {
                    bytes.len()
                };
            }
            b'{' => {
                depth += 1;
                index += 1;
            }
            b'}' => {
                depth -= 1;
                index += 1;
                if depth == 0 {
                    return index;
                }
            }
            b'\\' => index += 2,
            _ => {
                let character = source[index..].chars().next().unwrap_or('\u{fffd}');
                index += character.len_utf8();
            }
        }
    }

    source.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_static_dynamic_and_require_imports() {
        let source = r#"
            import a from "./a";
            import type { B } from "./b";
            import "./side";
            export { c } from "./c";
            const d = () => import("./d");
            const e = require("./e");
        "#;
        let imports = scan_imports(source);
        assert_eq!(
            imports,
            vec![
                ScannedImport::Literal {
                    specifier: "./a".into(),
                    kind: ImportKind::Static
                },
                ScannedImport::Literal {
                    specifier: "./b".into(),
                    kind: ImportKind::Static
                },
                ScannedImport::Literal {
                    specifier: "./side".into(),
                    kind: ImportKind::Static
                },
                ScannedImport::Literal {
                    specifier: "./c".into(),
                    kind: ImportKind::Static
                },
                ScannedImport::Literal {
                    specifier: "./d".into(),
                    kind: ImportKind::Dynamic
                },
                ScannedImport::Literal {
                    specifier: "./e".into(),
                    kind: ImportKind::Require
                },
            ]
        );
    }

    #[test]
    fn ignores_imports_in_comments_strings_templates_and_regexes() {
        let source = r#"
            // import x from "./comment"
            /* import y from "./block" */
            const text = "import z from './string'";
            const template = `import w from "./template"`;
            const pattern = /import v from ".\/regex"/;
            import real from "./real";
        "#;
        let imports = scan_imports(source);
        assert_eq!(
            imports,
            vec![ScannedImport::Literal {
                specifier: "./real".into(),
                kind: ImportKind::Static,
            }]
        );
    }

    #[test]
    fn non_literal_dynamic_imports_are_reported_as_expressions() {
        let source = r#"const target = "./x"; const value = import(target);"#;
        let imports = scan_imports(source);
        assert_eq!(
            imports,
            vec![ScannedImport::Expression {
                expression: "target".into(),
                kind: ImportKind::Dynamic,
            }]
        );
    }

    #[test]
    fn nested_template_literals_do_not_leak_imports() {
        let source = r#"
            const generated = `
                import fake from "./fake";
                ${condition ? `import nested from "./nested"` : ""}
            `;
            import real from "./real";
        "#;
        let imports = scan_imports(source);
        assert_eq!(
            imports,
            vec![ScannedImport::Literal {
                specifier: "./real".into(),
                kind: ImportKind::Static,
            }]
        );
    }
}
