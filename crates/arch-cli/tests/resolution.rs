use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn fixture_dir() -> PathBuf {
    workspace_root().join("tests/fixtures/resolution")
}

fn corpus_dir(name: &str) -> PathBuf {
    let path = workspace_root().join("tests/corpus").join(name);
    assert!(
        path.is_dir(),
        "sample corpus missing: {}; fetch pinned archives before running this test",
        path.display()
    );
    path
}

fn run_cli(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_arch-cli"))
        .args(arguments)
        .output()
        .expect("run arch-cli")
}

fn index(path: &Path) -> serde_json::Value {
    let output = run_cli(&["index", path.to_str().expect("path")]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "index failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("stdout must be JSON")
}

fn edges(json: &serde_json::Value) -> Vec<(String, String, String)> {
    json["edges"]
        .as_array()
        .expect("edges array")
        .iter()
        .map(|edge| {
            (
                edge["source"].as_str().expect("edge source").to_string(),
                edge["target"].as_str().expect("edge target").to_string(),
                edge["kind"].as_str().expect("edge kind").to_string(),
            )
        })
        .collect()
}

fn unresolved(json: &serde_json::Value) -> Vec<(String, String, String)> {
    json["unresolved"]
        .as_array()
        .expect("unresolved array")
        .iter()
        .map(|item| {
            (
                item["file"].as_str().expect("unresolved file").to_string(),
                item["specifier"]
                    .as_str()
                    .expect("unresolved specifier")
                    .to_string(),
                item["reason"]
                    .as_str()
                    .expect("unresolved reason")
                    .to_string(),
            )
        })
        .collect()
}

fn assert_edge(json: &serde_json::Value, source: &str, target: &str, kind: &str) {
    let actual = edges(json);
    assert!(
        actual.contains(&(source.to_string(), target.to_string(), kind.to_string())),
        "missing edge {source} -> {target} ({kind}); actual edges: {actual:?}"
    );
}

#[test]
fn fixture_resolves_relative_paths_aliases_workspaces_and_externals() {
    let json = index(&fixture_dir());

    assert_edge(&json, "src/main.ts", "src/util.ts", "import");
    assert_edge(&json, "src/main.ts", "src/types.ts", "import");
    assert_edge(&json, "src/main.ts", "packages/lib/src/index.ts", "import");
    assert_edge(&json, "src/main.ts", "src/helper.ts", "import");
    assert_edge(&json, "src/main.ts", "external:react", "import");
    assert_edge(&json, "src/main.ts", "src/lazy.ts", "dynamic-import");
    assert_edge(&json, "src/main.ts", "src/legacy.js", "require");
    assert_edge(&json, "src/main.ts", "src/side-effect.ts", "import");
    assert_edge(&json, "src/main.ts", "src/reexport.ts", "import");
    assert_edge(&json, "src/main.ts", "packages/lib/src/util.ts", "import");
    assert_edge(
        &json,
        "packages/lib/src/index.ts",
        "packages/lib/src/util.ts",
        "import",
    );

    let unresolved = unresolved(&json);
    assert!(
        unresolved.contains(&(
            "src/main.ts".to_string(),
            "./missing".to_string(),
            "relative-missing".to_string()
        )),
        "missing relative import must be reported: {unresolved:?}"
    );
    assert!(
        unresolved.contains(&(
            "src/main.ts".to_string(),
            "./data.json".to_string(),
            "unsupported-extension".to_string()
        )),
        "unsupported extension must be reported: {unresolved:?}"
    );
    assert!(
        unresolved.contains(&(
            "src/main.ts".to_string(),
            "@app/missing".to_string(),
            "alias-missing".to_string()
        )),
        "alias target miss must be reported: {unresolved:?}"
    );
    assert!(
        unresolved
            .iter()
            .any(|(file, _, reason)| { file == "src/main.ts" && reason == "dynamic-import" }),
        "non-literal dynamic import must be reported: {unresolved:?}"
    );

    let modules = json["modules"].as_array().expect("modules array");
    let external = modules
        .iter()
        .find(|module| module["id"] == "external:react")
        .expect("external package node");
    assert_eq!(external["kind"], "external-package");
    assert_eq!(external["path"], "react");
    assert!(
        modules.iter().all(|module| !module["path"]
            .as_str()
            .unwrap_or_default()
            .contains("node_modules")),
        "node_modules must not be expanded into modules"
    );
}

#[test]
fn tsx_internal_relative_imports_all_resolve() {
    let json = index(&corpus_dir("tsx"));

    assert_edge(&json, "src/cli.ts", "src/run.ts", "import");
    let unresolved = unresolved(&json);
    assert!(
        unresolved
            .iter()
            .all(|(_, _, reason)| reason != "relative-missing"),
        "tsx has unresolved relative imports: {unresolved:?}"
    );
    assert!(
        json["modules"]
            .as_array()
            .expect("modules")
            .iter()
            .any(|module| module["kind"] == "external-package"),
        "tsx should have package-level external nodes"
    );
}

#[test]
fn excalidraw_aliases_and_workspaces_resolve() {
    let json = index(&corpus_dir("excalidraw"));

    assert_edge(
        &json,
        "excalidraw-app/App.tsx",
        "packages/excalidraw/index.tsx",
        "import",
    );
    let unresolved = unresolved(&json);
    // 根 tsconfig 的 include 是 ["packages", "excalidraw-app"]；examples/dev-docs
    // 有各自构建配置且会引用构建产物 CSS，不在本票的解析范围内。
    assert!(
        unresolved.iter().all(|(file, specifier, reason)| {
            !(reason == "alias-missing"
                && specifier.starts_with("@excalidraw/")
                && (file.starts_with("excalidraw-app/") || file.starts_with("packages/")))
        }),
        "excalidraw aliases left unresolved: {unresolved:?}"
    );
    assert!(
        json["modules"]
            .as_array()
            .expect("modules")
            .iter()
            .any(|module| module["id"] == "external:react"),
        "excalidraw should have an external:react node"
    );

    let modules = json["modules"].as_array().expect("modules");
    assert!(
        modules.iter().all(|module| !module["path"]
            .as_str()
            .unwrap_or_default()
            .contains("node_modules")),
        "node_modules must not be expanded into modules"
    );
}

#[test]
fn resolution_output_is_deterministic() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let temp = std::env::temp_dir().join(format!("strata-resolution-{nanos}"));
    fs::create_dir_all(&temp).expect("temp dir");
    let first = temp.join("a.json");
    let second = temp.join("b.json");

    for output in [&first, &second] {
        let result = run_cli(&[
            "index",
            fixture_dir().to_str().expect("fixture path"),
            "--out",
            output.to_str().expect("output path"),
        ]);
        assert_eq!(result.status.code(), Some(0));
    }

    let first_bytes = fs::read(&first).expect("first artifact");
    let second_bytes = fs::read(&second).expect("second artifact");
    assert_eq!(first_bytes, second_bytes);
    let _ = fs::remove_dir_all(&temp);
}
