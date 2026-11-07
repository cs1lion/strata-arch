//! 文件级解析工具：扩展名补全、TS ESM `.js` -> `.ts` 映射、模块 ID 归一化。

use std::path::{Component, Path, PathBuf};

pub const SOURCE_EXTENSIONS: &[&str] = &["ts", "tsx", "js", "jsx", "mjs", "cjs", "mts", "cts"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileLookup {
    Found(PathBuf),
    Missing,
    UnsupportedExtension,
}

/// 以 `base` 为基准解析说明符。`specifier` 里的 `..` 由 `Path::join` 处理。
pub fn resolve_file_path(base: &Path, specifier: &str) -> FileLookup {
    if specifier.is_empty() {
        return FileLookup::Missing;
    }

    let candidate = base.join(specifier);
    match candidate.extension().and_then(|value| value.to_str()) {
        Some(extension) if is_source_extension(extension) => {
            if candidate.is_file() {
                return FileLookup::Found(candidate);
            }
            // TS/NodeNext 常见的写法：源码里写 `./run.js`，磁盘上是 `run.ts`。
            for replacement in swapped_extensions(extension) {
                let swapped = candidate.with_extension(replacement);
                if swapped.is_file() {
                    return FileLookup::Found(swapped);
                }
            }
            FileLookup::Missing
        }
        Some(_) => {
            // `./data.json` 这类真实存在的非源码文件显式报不支持；
            // `./App.selection` 这类带点的模块名仍按无扩展名补全。
            if candidate.is_file() {
                return FileLookup::UnsupportedExtension;
            }
            complete_extension(&candidate)
        }
        None => complete_extension(&candidate),
    }
}

fn complete_extension(candidate: &Path) -> FileLookup {
    for extension in SOURCE_EXTENSIONS {
        let appended = append_suffix(candidate, &format!(".{extension}"));
        if appended.is_file() {
            return FileLookup::Found(appended);
        }
    }
    let declaration = append_suffix(candidate, ".d.ts");
    if declaration.is_file() {
        return FileLookup::Found(declaration);
    }
    for extension in SOURCE_EXTENSIONS {
        let index = candidate.join(format!("index.{extension}"));
        if index.is_file() {
            return FileLookup::Found(index);
        }
    }
    FileLookup::Missing
}

fn is_source_extension(extension: &str) -> bool {
    SOURCE_EXTENSIONS.contains(&extension)
}

fn swapped_extensions(extension: &str) -> &'static [&'static str] {
    match extension {
        "js" => &["ts", "tsx", "mts", "cts"],
        "jsx" => &["tsx", "ts"],
        "mjs" => &["mts"],
        "cjs" => &["cts"],
        _ => &[],
    }
}

fn append_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_owned();
    value.push(suffix);
    PathBuf::from(value)
}

/// 把绝对文件路径折算为相对仓库根的正斜杠模块 ID；越界或根目录本身返回 `None`。
pub fn relative_module_id(root: &Path, file: &Path) -> Option<String> {
    let relative = file.strip_prefix(root).ok()?;
    let mut parts: Vec<String> = Vec::new();

    for component in relative.components() {
        match component {
            Component::Normal(value) => parts.push(value.to_string_lossy().replace('\\', "/")),
            Component::ParentDir => {
                parts.pop()?;
            }
            Component::CurDir => {}
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensionless_specifiers_append_source_extensions() {
        let root = std::env::temp_dir().join(format!("arch-core-resolve-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("temp dir");
        std::fs::write(root.join("a.ts"), "export const a = 1;\n").expect("a.ts");
        std::fs::write(root.join("b.tsx"), "export const b = 1;\n").expect("b.tsx");
        std::fs::create_dir_all(root.join("nested")).expect("nested");
        std::fs::write(root.join("nested/index.ts"), "export const c = 1;\n").expect("index");

        assert_eq!(
            resolve_file_path(&root, "./a"),
            FileLookup::Found(root.join("a.ts"))
        );
        assert_eq!(
            resolve_file_path(&root, "./nested"),
            FileLookup::Found(root.join("nested/index.ts"))
        );
        assert_eq!(
            resolve_file_path(&root, "./missing.ts"),
            FileLookup::Missing
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn js_specifiers_fall_back_to_typescript_sources() {
        let root =
            std::env::temp_dir().join(format!("arch-core-resolve-js-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("temp dir");
        std::fs::write(root.join("run.ts"), "export const run = 1;\n").expect("run.ts");

        assert_eq!(
            resolve_file_path(&root, "./run.js"),
            FileLookup::Found(root.join("run.ts"))
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn json_specifiers_are_unsupported() {
        let root =
            std::env::temp_dir().join(format!("arch-core-resolve-json-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("temp dir");
        std::fs::write(root.join("data.json"), "{}").expect("data.json");

        assert_eq!(
            resolve_file_path(&root, "./data.json"),
            FileLookup::UnsupportedExtension
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn dotted_module_names_still_append_source_extensions() {
        let root =
            std::env::temp_dir().join(format!("arch-core-resolve-dotted-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("temp dir");
        std::fs::write(root.join("App.selection.ts"), "export const a = 1;\n").expect("App");

        assert_eq!(
            resolve_file_path(&root, "./App.selection"),
            FileLookup::Found(root.join("App.selection.ts"))
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn declaration_files_resolve_for_extensionless_specifiers() {
        let root = std::env::temp_dir().join(format!(
            "arch-core-resolve-declaration-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("temp dir");
        std::fs::write(root.join("global.d.ts"), "export {};\n").expect("global");

        assert_eq!(
            resolve_file_path(&root, "./global"),
            FileLookup::Found(root.join("global.d.ts"))
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn module_ids_normalize_parent_components() {
        let root = Path::new("/repo");
        assert_eq!(
            relative_module_id(root, Path::new("/repo/src/util.ts")),
            Some("src/util.ts".to_string())
        );
        assert_eq!(
            relative_module_id(root, Path::new("/repo/src/../util.ts")),
            Some("util.ts".to_string())
        );
        assert_eq!(relative_module_id(root, Path::new("/other/util.ts")), None);
    }
}
