//! ticket 11 截图替代路径（A 档，先写测试）：
//!
//! 1. `arch-view scene tests/fixtures/layers --out <file>` 输出结构化场景描述，退出码 0、stdout 为空。
//! 2. 场景含节点、边、层带、z-order、命中框：`bands[].rect/zIndex`、`nodes[].rect/labelRect/hitRect/zIndex`、
//!    `edges[].from/to/zIndex`。
//! 3. z-order 关系固定：层带 < 边 < 节点，边画在层带之上、节点之下。
//! 4. 命中框等于节点矩形（当前实现不含额外热区）。
//! 5. 同一输入两次运行快照字节一致（确定性断言）。
//! 6. 缺目标仓库时退出码非 0 且给出用法。
//!
//! 本文件只覆盖 IR 快照；OS 窗口捕获是 `eval/capture-window.ps1` 手动脚本，不进 CI 门禁。

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

fn fixture_layers() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .join("tests/fixtures/layers")
}

fn unique_path(prefix: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!("strata-{prefix}-{unique}.json"))
}

fn run_scene(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_arch-view"))
        .args(arguments)
        .output()
        .expect("run arch-view scene")
}

fn scene_json(path: &Path) -> serde_json::Value {
    let text = fs::read_to_string(path).expect("scene artifact");
    serde_json::from_str(&text).expect("scene JSON")
}

#[test]
fn scene_writes_structured_nodes_edges_bands_and_z_order() {
    let out = unique_path("scene");
    let output = run_scene(&[
        "scene",
        fixture_layers().to_str().expect("path"),
        "--out",
        out.to_str().expect("path"),
    ]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "scene failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stdout.is_empty(),
        "--out must not duplicate the scene on stdout"
    );
    let json = scene_json(&out);
    assert_eq!(json["schemaVersion"], 1);
    assert_eq!(json["repo"]["isGit"], false);

    let bands = json["layout"]["bands"].as_array().expect("bands");
    let nodes = json["layout"]["nodes"].as_array().expect("nodes");
    let edges = json["layout"]["edges"].as_array().expect("edges");
    assert!(!bands.is_empty(), "fixture must produce layer bands");
    assert!(!nodes.is_empty(), "fixture must produce nodes");

    for band in bands {
        assert!(band["layer"].is_string(), "band layer: {band}");
        assert!(band["rect"]["width"].as_f64().unwrap() > 0.0);
        assert!(band["zIndex"].as_i64().unwrap() >= 0);
    }

    for node in nodes {
        let id = node["id"].as_str().expect("node id");
        assert!(node["layer"].is_string(), "node layer: {id}");
        assert!(node["rect"]["width"].as_f64().unwrap() > 0.0);
        assert!(node["labelRect"].is_object(), "node label rect: {id}");
        assert!(node["hitRect"].is_object(), "node hit rect: {id}");
        let band_z = bands[0]["zIndex"].as_i64().unwrap();
        assert!(
            node["zIndex"].as_i64().unwrap() > band_z,
            "nodes must sit above bands: {id}"
        );
        assert_eq!(
            node["hitRect"], node["rect"],
            "hit box must match the node rect: {id}"
        );
    }

    for edge in edges {
        assert!(edge["source"].is_string(), "edge source: {edge}");
        assert!(edge["target"].is_string(), "edge target: {edge}");
        assert!(edge["from"]["x"].is_number(), "edge from: {edge}");
        assert!(edge["to"]["y"].is_number(), "edge to: {edge}");
        let edge_z = edge["zIndex"].as_i64().unwrap();
        assert!(
            edge_z < nodes[0]["zIndex"].as_i64().unwrap(),
            "edges below nodes"
        );
        assert!(
            edge_z > bands[0]["zIndex"].as_i64().unwrap(),
            "edges above bands"
        );
    }

    fs::remove_file(&out).ok();
}

#[test]
fn scene_is_byte_deterministic_for_same_input() {
    let first = unique_path("scene-a");
    let second = unique_path("scene-b");
    let repo = fixture_layers();
    let repo = repo.to_str().expect("path");

    for path in [&first, &second] {
        let output = run_scene(&["scene", repo, "--out", path.to_str().expect("path")]);
        assert_eq!(output.status.code(), Some(0));
    }

    let first_bytes = fs::read(&first).expect("first scene");
    let second_bytes = fs::read(&second).expect("second scene");
    assert_eq!(
        first_bytes, second_bytes,
        "scene snapshot must be byte deterministic"
    );

    fs::remove_file(&first).ok();
    fs::remove_file(&second).ok();
}

#[test]
fn scene_without_repo_reports_usage() {
    let output = run_scene(&["scene"]);
    assert!(!output.status.success(), "missing repo must fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("scene 缺少目标仓库路径") && stderr.contains("用法"),
        "stderr must name the missing repo and show usage: {stderr}"
    );
}
