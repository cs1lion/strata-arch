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
    workspace_root().join("tests/fixtures/path")
}

fn corpus(name: &str) -> PathBuf {
    let path = workspace_root().join("tests/corpus").join(name);
    assert!(
        path.is_dir(),
        "missing corpus sample at {}; download the pinned commit into tests/corpus/{name}",
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

fn path_json(arguments: &[&str]) -> serde_json::Value {
    let output = run_cli(arguments);
    assert_eq!(
        output.status.code(),
        Some(0),
        "path failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "stderr must be empty: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("stdout must be JSON")
}

fn entries(json: &serde_json::Value) -> &Vec<serde_json::Value> {
    json["entries"].as_array().expect("entries array")
}

fn find_entry<'a>(json: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    entries(json)
        .iter()
        .find(|entry| entry["id"] == id)
        .unwrap_or_else(|| panic!("missing entry {id}: {}", json))
}

fn steps(json: &serde_json::Value) -> &Vec<serde_json::Value> {
    json["steps"].as_array().expect("steps array")
}

fn symbol_sequence(json: &serde_json::Value) -> Vec<String> {
    steps(json)
        .iter()
        .map(|step| step["symbol"].as_str().expect("step symbol").to_string())
        .collect()
}

#[test]
fn entries_map_manifest_targets_to_existing_modules() {
    let json = path_json(&["path", fixture_dir().to_str().expect("path"), "--entries"]);

    assert_eq!(json["schemaVersion"], 1);
    assert_eq!(json["repo"]["isGit"], false);

    let cli = find_entry(&json, "src/cli.ts#module-toplevel");
    assert_eq!(cli["module"], "src/cli.ts");
    assert_eq!(cli["source"], "package.json:bin");
    assert_eq!(cli["layer"], "unclassified");

    let index = find_entry(&json, "src/index.ts#module-toplevel");
    assert_eq!(index["source"], "package.json:main");

    let helper_module = find_entry(&json, "src/utils/helper.ts#module-toplevel");
    assert_eq!(helper_module["source"], "package.json:exports");

    let helper = find_entry(&json, "src/utils/helper.ts#function:helper");
    assert_eq!(helper["source"], "public-export");
    assert_eq!(helper["module"], "src/utils/helper.ts");

    for entry in entries(&json) {
        let module = entry["module"].as_str().expect("entry module");
        let id = entry["id"].as_str().expect("entry id");
        assert!(
            id.starts_with(&format!("{module}#")),
            "entry id must belong to its module: {entry}"
        );
    }
}

#[test]
fn entries_accept_repo_flag_and_out_file() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let output_path = std::env::temp_dir().join(format!("strata-path-entries-{unique}.json"));

    let output = run_cli(&[
        "path",
        "--repo",
        fixture_dir().to_str().expect("path"),
        "--entries",
        "--out",
        output_path.to_str().expect("path"),
    ]);

    assert_eq!(output.status.code(), Some(0));
    assert!(
        output.stdout.is_empty(),
        "--out must not duplicate JSON on stdout"
    );
    let written = fs::read_to_string(&output_path).expect("entries artifact");
    let json: serde_json::Value = serde_json::from_str(&written).expect("entries JSON");
    find_entry(&json, "src/cli.ts#module-toplevel");

    fs::remove_file(&output_path).ok();
}

#[test]
fn path_query_returns_ordered_steps() {
    let json = path_json(&[
        "path",
        fixture_dir().to_str().expect("path"),
        "--from",
        "src/cli.ts#module-toplevel",
        "--to",
        "src/utils/helper.ts#function:helper",
    ]);

    assert_eq!(json["schemaVersion"], 2);
    assert_eq!(json["found"], true);
    assert_eq!(json["length"], 3);
    assert_eq!(
        symbol_sequence(&json),
        vec![
            "src/cli.ts#module-toplevel",
            "src/alt.ts#function:alt",
            "src/utils/helper.ts#function:helper",
        ],
        "tie-break must pick the lexicographically first target at each BFS level"
    );

    let steps = steps(&json);
    assert_eq!(steps[0]["via"], serde_json::Value::Null);
    assert_eq!(steps[1]["via"], "call");
    assert_eq!(steps[2]["via"], "call");
    assert_eq!(steps[0]["layer"], "unclassified");
    assert_eq!(steps[2]["layer"], "utils");
}

#[test]
fn path_accepts_module_id_as_start() {
    let json = path_json(&[
        "path",
        fixture_dir().to_str().expect("path"),
        "--from",
        "src/cli.ts",
        "--to",
        "src/utils/helper.ts#function:helper",
    ]);

    assert_eq!(json["from"], "src/cli.ts#module-toplevel");
    assert_eq!(json["length"], 3);
}

#[test]
fn unreachable_target_exits_4_with_empty_stdout() {
    let output = run_cli(&[
        "path",
        fixture_dir().to_str().expect("path"),
        "--from",
        "src/cli.ts#module-toplevel",
        "--to",
        "src/isolated.ts#function:isolated",
    ]);

    assert_eq!(output.status.code(), Some(4));
    assert!(
        output.stdout.is_empty(),
        "unreachable path must not print a path: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("不可达"),
        "stderr must explain unreachable: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn unknown_target_exits_2() {
    let output = run_cli(&[
        "path",
        fixture_dir().to_str().expect("path"),
        "--from",
        "src/cli.ts#module-toplevel",
        "--to",
        "src/isolated.ts#function:missing",
    ]);

    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("未知符号"),
        "stderr must name the unknown symbol: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn path_output_is_byte_deterministic() {
    let fixture = fixture_dir();
    let arguments = [
        "path",
        fixture.to_str().expect("path"),
        "--from",
        "src/cli.ts#module-toplevel",
        "--to",
        "src/utils/helper.ts#function:helper",
    ];
    let first = run_cli(&arguments);
    let second = run_cli(&arguments);
    assert_eq!(first.status.code(), Some(0));
    assert_eq!(first.stdout, second.stdout);
}

#[test]
fn narrative_flag_prints_text_with_one_line_per_step() {
    let fixture = fixture_dir();
    let arguments = [
        "path",
        fixture.to_str().expect("path"),
        "--from",
        "src/cli.ts#module-toplevel",
        "--to",
        "src/utils/helper.ts#function:helper",
        "--narrative",
    ];
    let output = run_cli(&arguments);

    assert_eq!(output.status.code(), Some(0));
    assert!(
        output.stderr.is_empty(),
        "--narrative must stay silent on stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let first_stdout = output.stdout.clone();
    let text = String::from_utf8(output.stdout).expect("narrative text");
    assert!(
        text.starts_with(
            "路径 src/cli.ts#module-toplevel → src/utils/helper.ts#function:helper：3 步"
        ),
        "narrative header must name both ends and the step count: {text}"
    );
    let body_lines = text
        .lines()
        .filter(|line: &&str| line.starts_with("- "))
        .count();
    assert_eq!(body_lines, 3, "one narrative line per step: {text}");
    assert!(
        !text.trim_start().starts_with('{'),
        "--narrative must not print JSON: {text}"
    );

    let second = run_cli(&arguments);
    assert_eq!(
        first_stdout, second.stdout,
        "narrative text must be byte deterministic"
    );
}

#[test]
fn narrative_flag_writes_text_to_out_file() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let output_path = std::env::temp_dir().join(format!("strata-path-narrative-{unique}.txt"));

    let output = run_cli(&[
        "path",
        fixture_dir().to_str().expect("path"),
        "--from",
        "src/cli.ts#module-toplevel",
        "--to",
        "src/utils/helper.ts#function:helper",
        "--narrative",
        "--out",
        output_path.to_str().expect("path"),
    ]);

    assert_eq!(output.status.code(), Some(0));
    assert!(
        output.stdout.is_empty(),
        "--narrative --out must not duplicate text on stdout"
    );
    let written = fs::read_to_string(&output_path).expect("narrative file");
    assert!(written.contains("src/cli.ts#module-toplevel"));
    assert_eq!(
        written
            .lines()
            .filter(|line: &&str| line.starts_with("- "))
            .count(),
        3
    );

    fs::remove_file(&output_path).ok();
}

#[test]
fn narrative_with_entries_exits_2() {
    let output = run_cli(&[
        "path",
        fixture_dir().to_str().expect("path"),
        "--entries",
        "--narrative",
    ]);

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--narrative") && stderr.contains("--entries") && stderr.contains("不能"),
        "stderr must explain the conflict, not just reject the flag: {stderr}"
    );
}

#[test]
fn zero_entry_candidates_exits_5() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("strata-path-empty-{unique}"));
    fs::create_dir_all(directory.join("src")).expect("create temp dir");
    fs::write(directory.join("src/only.ts"), "export const value = 1;\n").expect("write source");

    let output = run_cli(&["path", directory.to_str().expect("path"), "--entries"]);

    assert_eq!(output.status.code(), Some(5));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("入口"),
        "stderr must explain missing entries: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    fs::remove_dir_all(&directory).ok();
}

#[test]
fn tsx_entries_and_golden_path() {
    let corpus = corpus("tsx");
    let entries_json = path_json(&["path", corpus.to_str().expect("path"), "--entries"]);
    let cli = find_entry(&entries_json, "src/cli.ts#module-toplevel");
    assert_eq!(cli["source"], "package.json:bin");
    assert_eq!(cli["module"], "src/cli.ts");

    let loader = find_entry(&entries_json, "src/loader.ts#module-toplevel");
    assert_eq!(loader["source"], "package.json:exports");

    let json = path_json(&[
        "path",
        corpus.to_str().expect("path"),
        "--from",
        "src/cli.ts#module-toplevel",
        "--to",
        "src/utils/ipc/get-pipe-path.ts#function:getPipePath",
    ]);

    assert_eq!(json["length"], 3);
    assert_eq!(
        symbol_sequence(&json),
        vec![
            "src/cli.ts#module-toplevel",
            "src/utils/ipc/server.ts#function:createIpcServer",
            "src/utils/ipc/get-pipe-path.ts#function:getPipePath",
        ]
    );
    assert_eq!(steps(&json)[1]["via"], "call");
    assert_eq!(steps(&json)[2]["via"], "call");
}

#[test]
fn excalidraw_golden_path() {
    let corpus = corpus("excalidraw");
    let json = path_json(&[
        "path",
        corpus.to_str().expect("path"),
        "--from",
        "packages/excalidraw/index.tsx#component:ExcalidrawBase",
        "--to",
        "packages/excalidraw/i18n.ts#function:setLanguage",
    ]);

    assert_eq!(json["length"], 3);
    assert_eq!(
        symbol_sequence(&json),
        vec![
            "packages/excalidraw/index.tsx#component:ExcalidrawBase",
            "packages/excalidraw/components/InitializeApp.tsx#component:InitializeApp",
            "packages/excalidraw/i18n.ts#function:setLanguage",
        ]
    );
    assert_eq!(steps(&json)[1]["via"], "jsx");
    assert_eq!(steps(&json)[2]["via"], "call");
}
