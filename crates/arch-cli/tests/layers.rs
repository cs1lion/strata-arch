use std::collections::{BTreeMap, BTreeSet};
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
    workspace_root().join("tests/fixtures/layers")
}

fn run_cli(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_arch-cli"))
        .args(arguments)
        .output()
        .expect("run arch-cli")
}

fn layers(path: &Path) -> serde_json::Value {
    let output = run_cli(&["layers", path.to_str().expect("path")]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "layers failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "stderr must be empty: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("stdout must be JSON")
}

fn module<'a>(json: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    json["modules"]
        .as_array()
        .expect("modules array")
        .iter()
        .find(|module| module["id"].as_str() == Some(id))
        .unwrap_or_else(|| panic!("module missing: {id}"))
}

fn module_layers(json: &serde_json::Value) -> BTreeMap<String, String> {
    json["modules"]
        .as_array()
        .expect("modules array")
        .iter()
        .map(|module| {
            (
                module["id"].as_str().expect("module id").to_string(),
                module["layer"]
                    .as_str()
                    .expect("module must carry a layer")
                    .to_string(),
            )
        })
        .collect()
}

fn unclassified_entries(json: &serde_json::Value) -> Vec<(String, String)> {
    json["unclassified"]
        .as_array()
        .expect("unclassified array")
        .iter()
        .map(|entry| {
            (
                entry["id"].as_str().expect("unclassified id").to_string(),
                entry["reason"]
                    .as_str()
                    .expect("unclassified entry must carry a reason")
                    .to_string(),
            )
        })
        .collect()
}

fn find_edge<'a>(json: &'a serde_json::Value, source: &str, target: &str) -> &'a serde_json::Value {
    json["edges"]
        .as_array()
        .expect("edges array")
        .iter()
        .find(|edge| {
            edge["source"].as_str() == Some(source) && edge["target"].as_str() == Some(target)
        })
        .unwrap_or_else(|| panic!("edge missing: {source} -> {target}"))
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("arch-layers-{tag}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&path).expect("create temp dir");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn layer_order_includes_external_before_unclassified() {
    let json = layers(&fixture_dir());

    let layer_order = json["layerOrder"]
        .as_array()
        .expect("layerOrder array")
        .iter()
        .map(|value| value.as_str().expect("layer id").to_string())
        .collect::<Vec<_>>();
    assert_eq!(
        layer_order,
        vec![
            "cli",
            "commands",
            "components",
            "server",
            "utils",
            "tests",
            "external",
            "unclassified",
        ]
    );

    for (module, layer) in module_layers(&json) {
        assert!(
            layer_order.contains(&layer),
            "module {module} carries a layer outside layerOrder: {layer}"
        );
    }
}

#[test]
fn fixture_assigns_directory_filename_and_workspace_layers() {
    let json = layers(&fixture_dir());

    let expected = [
        ("app/page.tsx", "components"),
        ("cli/main.ts", "cli"),
        ("commands/run.ts", "commands"),
        ("components/Button.tsx", "components"),
        ("components/Card.tsx", "components"),
        ("features/tie-components.test.stories.tsx", "components"),
        ("features/tie-tests.test.stories.tsx", "tests"),
        ("lib/helper.ts", "utils"),
        ("misc/notes.ts", "unclassified"),
        ("misc/plain.ts", "unclassified"),
        ("packages/cli/src/main.ts", "cli"),
        ("packages/web/src/components/Badge.tsx", "components"),
        ("packages/web/src/index.ts", "unclassified"),
        ("packages/web/src/utils/pkg-format.ts", "utils"),
        ("server/api.ts", "server"),
        ("src/components/Widget.tsx", "components"),
        ("src/features/random.ts", "unclassified"),
        ("src/utils/both.test.stories.tsx", "unclassified"),
        ("src/utils/math.test.ts", "tests"),
        ("src/utils/story.stories.tsx", "components"),
        ("tests/format.test.ts", "tests"),
        ("tests/other.test.ts", "tests"),
        ("utils/backdoor.ts", "utils"),
        ("utils/format.ts", "utils"),
    ];
    let layers_by_module = module_layers(&json);
    for (path, layer) in expected {
        assert_eq!(
            layers_by_module.get(path).map(String::as_str),
            Some(layer),
            "wrong layer for {path}"
        );
    }
}

#[test]
fn filename_conventions_win_over_directory_and_are_traceable() {
    let json = layers(&fixture_dir());

    let test_file = module(&json, "src/utils/math.test.ts");
    assert_eq!(test_file["layer"].as_str(), Some("tests"));
    assert_eq!(
        test_file["rule"].as_str(),
        Some("file:test-suffix"),
        "test suffix must be recorded as the winning rule"
    );
    assert_eq!(test_file["reason"].as_str(), None);

    let story_file = module(&json, "src/utils/story.stories.tsx");
    assert_eq!(story_file["layer"].as_str(), Some("components"));
    assert_eq!(
        story_file["rule"].as_str(),
        Some("file:stories-suffix"),
        "stories suffix must win over the utils directory"
    );
}

#[test]
fn external_packages_form_their_own_layer_and_are_explained() {
    let json = layers(&fixture_dir());

    let external = module(&json, "external:react");
    assert_eq!(external["kind"].as_str(), Some("external-package"));
    assert_eq!(external["layer"].as_str(), Some("external"));
    assert_eq!(external["reason"].as_str(), Some("external"));
    assert_eq!(
        external["rule"].as_str(),
        Some("kind:external-package"),
        "external assignment must be traceable"
    );

    let entries = unclassified_entries(&json);
    assert!(
        !entries.iter().any(|(id, _)| id == "external:react"),
        "external packages must not appear in the unclassified list"
    );

    for module in json["modules"].as_array().expect("modules array") {
        if module["kind"].as_str() == Some("external-package") {
            assert_eq!(module["layer"].as_str(), Some("external"));
        }
    }
}

#[test]
fn unclassified_entries_carry_reason_codes() {
    let json = layers(&fixture_dir());

    let entries = unclassified_entries(&json);
    assert_eq!(
        entries,
        vec![
            (
                "features/tie-none.test.stories.tsx".to_string(),
                "ambiguous".to_string()
            ),
            ("misc/notes.ts".to_string(), "no-rule-match".to_string()),
            ("misc/plain.ts".to_string(), "no-rule-match".to_string()),
            (
                "packages/web/src/index.ts".to_string(),
                "no-rule-match".to_string()
            ),
            (
                "src/features/random.ts".to_string(),
                "no-rule-match".to_string()
            ),
            (
                "src/utils/both.test.stories.tsx".to_string(),
                "ambiguous".to_string()
            ),
        ]
    );

    for (id, reason) in &entries {
        assert!(
            reason == "no-rule-match" || reason == "ambiguous",
            "unclassified {id} must carry an unclassified reason, got {reason}"
        );
        let module = module(&json, id);
        assert_eq!(module["layer"].as_str(), Some("unclassified"));
        assert_eq!(module["reason"].as_str(), Some(reason.as_str()));
        assert_eq!(
            module["rule"].as_str(),
            None,
            "unclassified {id} must not claim a winning rule"
        );
    }
    assert_eq!(json["unclassified"].as_array().unwrap().len(), 6);
}

#[test]
fn reverse_cases_stay_unclassified_without_generalizing() {
    let json = layers(&fixture_dir());
    let layers_by_module = module_layers(&json);

    for path in [
        "misc/notes.ts",
        "misc/plain.ts",
        "src/features/random.ts",
        "packages/web/src/index.ts",
    ] {
        assert_eq!(
            layers_by_module.get(path).map(String::as_str),
            Some("unclassified"),
            "reverse case {path} must stay unclassified"
        );
    }

    assert_eq!(
        module(&json, "misc/notes.ts")["reason"].as_str(),
        Some("no-rule-match")
    );
    assert_eq!(
        module(&json, "src/features/random.ts")["reason"].as_str(),
        Some("no-rule-match"),
        "src/ prefix alone must not act as a rule"
    );
    assert_eq!(
        module(&json, "packages/web/src/index.ts")["reason"].as_str(),
        Some("no-rule-match"),
        "workspace package name alone must not act as a rule"
    );
    assert_eq!(
        module(&json, "src/utils/both.test.stories.tsx")["reason"].as_str(),
        Some("ambiguous"),
        "test+stories conflict without topology evidence must stay ambiguous"
    );
}

#[test]
fn topology_only_breaks_filename_conflicts_and_is_traceable() {
    let json = layers(&fixture_dir());

    let tests_tie = module(&json, "features/tie-tests.test.stories.tsx");
    assert_eq!(
        tests_tie["layer"].as_str(),
        Some("tests"),
        "two tests importers must break the conflict toward tests"
    );
    assert_eq!(
        tests_tie["rule"].as_str(),
        Some("topology:tests:support=2"),
        "topology tie-break must record evidence"
    );

    let components_tie = module(&json, "features/tie-components.test.stories.tsx");
    assert_eq!(components_tie["layer"].as_str(), Some("components"));
    assert_eq!(
        components_tie["rule"].as_str(),
        Some("topology:components:support=2")
    );

    let unresolved = module(&json, "features/tie-none.test.stories.tsx");
    assert_eq!(unresolved["layer"].as_str(), Some("unclassified"));
    assert_eq!(unresolved["reason"].as_str(), Some("ambiguous"));

    let notes = module(&json, "misc/notes.ts");
    assert_eq!(
        notes["layer"].as_str(),
        Some("unclassified"),
        "topology must not assign layers when naming produced no candidate"
    );
}

#[test]
fn layers_fixture_marks_direction_rule_violations_without_dropping_edges() {
    let json = layers(&fixture_dir());

    let allowed = find_edge(&json, "cli/main.ts", "commands/run.ts");
    assert_eq!(
        allowed["directionAdjusted"].as_bool(),
        Some(false),
        "cli -> commands should follow direction and stay normal confidence"
    );

    let violation = find_edge(&json, "utils/backdoor.ts", "commands/run.ts");
    assert_eq!(
        violation["directionAdjusted"].as_bool(),
        Some(true),
        "utils -> commands must be marked as direction-adjusted"
    );
    assert!(
        find_edge(&json, "commands/run.ts", "utils/format.ts")["directionAdjusted"]
            .as_bool()
            .is_some(),
        "original edges must not be silently removed"
    );

    let external_edge = find_edge(&json, "components/Card.tsx", "external:react");
    assert_eq!(
        external_edge["directionAdjusted"].as_bool(),
        Some(false),
        "external layer has no direction rank and must not be adjusted"
    );
}

#[test]
fn layers_artifact_is_byte_deterministic() {
    let first = run_cli(&["layers", fixture_dir().to_str().expect("path")]);
    let second = run_cli(&["layers", fixture_dir().to_str().expect("path")]);
    assert_eq!(first.status.code(), Some(0));
    assert_eq!(second.status.code(), Some(0));
    assert_eq!(
        first.stdout, second.stdout,
        "layers output must be byte-identical across runs"
    );
}

#[test]
fn layers_command_writes_out_file_and_keeps_stdout_clean() {
    let temp = TempDir::new("out");
    let path = temp.path().join("layers.json");
    let output = run_cli(&[
        "layers",
        fixture_dir().to_str().expect("path"),
        "--out",
        path.to_str().expect("out path"),
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "layers failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stdout.is_empty(),
        "stdout must be empty when --out is used"
    );
    let json: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).expect("read out file")).expect("out JSON");
    assert_eq!(
        json["stats"]["modules"].as_u64(),
        Some(26),
        "fixture must expose 25 source modules plus one external package"
    );

    let ids = json["modules"]
        .as_array()
        .expect("modules array")
        .iter()
        .map(|module| module["id"].as_str().expect("module id").to_string())
        .collect::<BTreeSet<_>>();
    assert_eq!(ids.len(), 26, "module ids must be unique and complete");
}
