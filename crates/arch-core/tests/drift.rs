//! N1 架构漂移检测的 A 档测试（schema v2）。
//!
//! 判据定义（与 `arch_core::drift` 模块文档一致）：
//! - **未确认的模块变更**：当前模块 ID 集合与基线不一致（新增或删除）。报告按层分组显式
//!   列出 `added` / `removed`；确认方式只能是重录基线。没有数字容差。
//! - **新增反向边**：`directionAdjusted == true` 的 `(source, target, kind)` 在基线中不存在。
//! - **新增循环**：模块级强连通分量（SCC）的成员集合在基线中不存在；已存在的循环不重复计数。
//! - **external 泄漏**：`kind == external-package` 的模块出现在非 `external` 层。
//!
//! 所有断言只看外部可观察产物（`DriftBaseline` / `DriftReport`），不触碰私有实现。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use arch_core::drift::{
    evaluate, record_baseline, DriftBaseline, DriftReport, Violation, DRIFT_SCHEMA_VERSION,
};
use arch_core::layers::build_layers;

const TOOL_VERSION: &str = "test";

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn fixture() -> PathBuf {
    workspace_root().join("tests/fixtures/drift")
}

fn baseline_for(root: &Path) -> DriftBaseline {
    let layers = build_layers(root, TOOL_VERSION).expect("layers");
    record_baseline(&layers, TOOL_VERSION)
}

fn report_for(root: &Path, baseline: &DriftBaseline) -> DriftReport {
    let layers = build_layers(root, TOOL_VERSION).expect("layers");
    evaluate(baseline, &layers, TOOL_VERSION)
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        // 并发测试可能在同一时钟刻度上取到相同 nanos，目录撞名会让用例互相踩踏。
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "arch-drift-{tag}-{}-{nanos}-{seq}",
            std::process::id()
        ));
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

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("create target dir");
    for entry in fs::read_dir(from).expect("read fixture dir") {
        let entry = entry.expect("fixture entry");
        let source = entry.path();
        let target = to.join(entry.file_name());
        if entry.file_type().expect("file type").is_dir() {
            copy_dir(&source, &target);
        } else {
            fs::copy(&source, &target).expect("copy fixture file");
        }
    }
}

fn fixture_copy(tag: &str) -> TempDir {
    let temp = TempDir::new(tag);
    copy_dir(&fixture(), temp.path());
    temp
}

fn append(path: &Path, text: &str) {
    let mut existing = fs::read_to_string(path).unwrap_or_default();
    existing.push_str(text);
    fs::write(path, existing).expect("append fixture source");
}

#[test]
fn positive_baseline_reports_no_drift_in_any_criterion() {
    let baseline = baseline_for(&fixture());

    assert_eq!(baseline.schema_version, DRIFT_SCHEMA_VERSION);
    assert_eq!(baseline.modules.len(), 8);

    let report = report_for(&fixture(), &baseline);

    assert_eq!(report.stats.modules, baseline.modules.len());
    assert!(report.gate.passed);
    assert_eq!(report.gate.checks.module_changes.new_count, 0);
    assert_eq!(report.gate.checks.reverse_edges.new_count, 0);
    assert_eq!(report.gate.checks.cycles.new_count, 0);
    assert_eq!(report.gate.checks.external_leaks.new_count, 0);
    assert_eq!(report.gate.checks.module_changes.resolved_count, 0);
    assert_eq!(report.gate.checks.reverse_edges.resolved_count, 0);
    assert_eq!(report.gate.checks.cycles.resolved_count, 0);
    assert_eq!(report.gate.checks.external_leaks.resolved_count, 0);

    assert!(report.new_violations.is_empty());
    assert!(report.resolved_violations.is_empty());
    assert!(report.modules.added.is_empty());
    assert!(report.modules.removed.is_empty());
    assert!(report.modules.moved.is_empty());
}

#[test]
fn added_module_is_reported_as_unconfirmed_change_by_layer() {
    let baseline = baseline_for(&fixture());
    let repo = fixture_copy("added");
    fs::write(
        repo.path().join("misc/extra.ts"),
        "export const extra = 1;\n",
    )
    .expect("write extra module");

    let report = report_for(repo.path(), &baseline);

    assert!(!report.gate.passed);
    assert_eq!(report.gate.checks.module_changes.new_count, 1);
    assert_eq!(report.gate.checks.reverse_edges.new_count, 0);
    assert_eq!(report.gate.checks.cycles.new_count, 0);
    assert_eq!(report.gate.checks.external_leaks.new_count, 0);
    assert_eq!(
        report.new_violations,
        vec![Violation::ModuleChange {
            layer: "unclassified".to_string(),
            added: vec!["misc/extra.ts".to_string()],
            removed: vec![],
        }],
        "module changes must be grouped by layer and name the modules"
    );
    assert_eq!(
        report.modules.added,
        vec!["misc/extra.ts".to_string()],
        "the detail layer must locate the new module"
    );
}

#[test]
fn removed_module_is_reported_as_unconfirmed_change_by_layer() {
    let baseline = baseline_for(&fixture());
    let repo = fixture_copy("removed");
    fs::remove_file(repo.path().join("server/api.ts")).expect("remove module");

    let report = report_for(repo.path(), &baseline);

    assert!(!report.gate.passed);
    assert_eq!(report.gate.checks.module_changes.new_count, 1);
    assert_eq!(
        report.new_violations,
        vec![Violation::ModuleChange {
            layer: "server".to_string(),
            added: vec![],
            removed: vec!["server/api.ts".to_string()],
        }],
        "deletions also need explicit confirmation, not a count threshold"
    );
    assert_eq!(report.modules.removed, vec!["server/api.ts".to_string()]);
}

#[test]
fn added_and_removed_modules_produce_one_violation_per_layer() {
    let baseline = baseline_for(&fixture());
    let repo = fixture_copy("two-layers");
    fs::write(
        repo.path().join("utils/extra.ts"),
        "export const extra = 1;\n",
    )
    .expect("write utils module");
    fs::remove_file(repo.path().join("server/api.ts")).expect("remove server module");

    let report = report_for(repo.path(), &baseline);

    assert!(!report.gate.passed);
    assert_eq!(
        report.gate.checks.module_changes.new_count, 2,
        "one grouped record per affected layer"
    );
    assert_eq!(
        report.new_violations,
        vec![
            Violation::ModuleChange {
                layer: "server".to_string(),
                added: vec![],
                removed: vec!["server/api.ts".to_string()],
            },
            Violation::ModuleChange {
                layer: "utils".to_string(),
                added: vec!["utils/extra.ts".to_string()],
                removed: vec![],
            },
        ],
        "layer groups must be deterministically sorted"
    );
}

#[test]
fn baseline_schema_v2_removes_the_numeric_tolerance_and_layer_counts() {
    let baseline = baseline_for(&fixture());
    let json = serde_json::to_string(&baseline).expect("serialize baseline");

    assert!(
        !json.contains("populationTolerance"),
        "numeric tolerance must be gone, so it cannot be tuned after the fact"
    );
    assert!(
        !json.contains("\"layers\""),
        "layer counts are removed: module identity is the gate, not a count"
    );
}

#[test]
fn a_new_legal_forward_edge_is_not_drift() {
    let baseline = baseline_for(&fixture());
    let repo = fixture_copy("forward");
    append(
        &repo.path().join("cli/main.ts"),
        "\nimport { format } from \"../utils/format.js\";\n",
    );

    let report = report_for(repo.path(), &baseline);

    assert!(
        report.gate.passed,
        "cli -> utils follows the layer direction and must not be drift"
    );
    assert_eq!(report.gate.checks.module_changes.new_count, 0);
    assert_eq!(report.gate.checks.reverse_edges.new_count, 0);
    assert_eq!(report.gate.checks.cycles.new_count, 0);
    assert_eq!(report.gate.checks.external_leaks.new_count, 0);
    assert!(report.new_violations.is_empty());
    assert!(report.modules.added.is_empty());
}

#[test]
fn a_new_reverse_edge_is_reported_with_both_layers() {
    let baseline = baseline_for(&fixture());
    let repo = fixture_copy("reverse");
    append(
        &repo.path().join("utils/text.ts"),
        "\nimport { run } from \"../commands/run.js\";\n",
    );

    let report = report_for(repo.path(), &baseline);

    assert!(!report.gate.passed);
    assert_eq!(report.gate.checks.reverse_edges.new_count, 1);
    assert_eq!(report.gate.checks.module_changes.new_count, 0);
    assert_eq!(report.gate.checks.cycles.new_count, 0);
    assert_eq!(report.gate.checks.external_leaks.new_count, 0);
    assert_eq!(
        report.new_violations,
        vec![Violation::ReverseEdge {
            source: "utils/text.ts".to_string(),
            target: "commands/run.ts".to_string(),
            edge_kind: "import".to_string(),
            source_layer: "utils".to_string(),
            target_layer: "commands".to_string(),
        }],
        "a reverse edge must name the edge and both endpoint layers"
    );
}

#[test]
fn a_new_cycle_is_reported_once_and_existing_cycles_are_not_recounted() {
    let repo = fixture_copy("cycle");
    append(
        &repo.path().join("utils/format.ts"),
        "\nimport { text } from \"./text.js\";\n",
    );
    append(
        &repo.path().join("utils/text.ts"),
        "\nimport { format } from \"./format.js\";\n",
    );

    let pristine = baseline_for(&fixture());
    let first = report_for(repo.path(), &pristine);
    assert!(!first.gate.passed);
    assert_eq!(first.gate.checks.cycles.new_count, 1);
    assert_eq!(first.gate.checks.reverse_edges.new_count, 0);
    assert_eq!(first.gate.checks.module_changes.new_count, 0);
    assert_eq!(
        first.new_violations,
        vec![Violation::Cycle {
            modules: vec!["utils/format.ts".to_string(), "utils/text.ts".to_string()],
        }],
        "one cycle must be one SCC record, not one record per traversed edge"
    );

    let cycled_baseline = baseline_for(repo.path());
    let second = report_for(repo.path(), &cycled_baseline);
    assert!(second.gate.passed);
    assert_eq!(
        second.gate.checks.cycles.new_count, 0,
        "a cycle already recorded in the baseline must not be recounted"
    );
    assert!(second.resolved_violations.is_empty());
}

#[test]
fn a_renamed_module_is_reported_as_added_and_removed() {
    let baseline = baseline_for(&fixture());
    let repo = fixture_copy("rename");
    fs::rename(
        repo.path().join("misc/note.ts"),
        repo.path().join("misc/note-renamed.ts"),
    )
    .expect("rename module");

    let report = report_for(repo.path(), &baseline);

    assert!(
        !report.gate.passed,
        "a rename is an unconfirmed module identity change and must fail the gate"
    );
    assert_eq!(report.gate.checks.module_changes.new_count, 1);
    assert_eq!(
        report.new_violations,
        vec![Violation::ModuleChange {
            layer: "unclassified".to_string(),
            added: vec!["misc/note-renamed.ts".to_string()],
            removed: vec!["misc/note.ts".to_string()],
        }],
        "renames must be explicit, never merged into no change"
    );
    assert_eq!(
        report.modules.added,
        vec!["misc/note-renamed.ts".to_string()]
    );
    assert_eq!(report.modules.removed, vec!["misc/note.ts".to_string()]);
    assert!(report.modules.moved.is_empty());
}

#[test]
fn an_external_package_outside_the_external_layer_is_a_leak() {
    let baseline = baseline_for(&fixture());
    let mut layers = build_layers(&fixture(), TOOL_VERSION).expect("layers");

    // 交换两个模块的层归属，保持每层计数不变，从而把 external 泄漏单独暴露出来。
    for module in &mut layers.modules {
        match module.id.as_str() {
            "external:react" => module.layer = "components",
            "components/Button.tsx" => module.layer = "external",
            _ => {}
        }
    }

    let report = evaluate(&baseline, &layers, TOOL_VERSION);

    assert!(!report.gate.passed);
    assert_eq!(report.gate.checks.external_leaks.new_count, 1);
    assert_eq!(
        report.gate.checks.module_changes.new_count, 0,
        "the swap keeps module identities stable, so the leak must be attributable to the leak criterion"
    );
    assert_eq!(
        report.new_violations,
        vec![Violation::ExternalLeak {
            module: "external:react".to_string(),
            layer: "components".to_string(),
            expected_layer: "external".to_string(),
        }],
        "an external leak must name the module and the polluted layer"
    );
}

#[test]
fn resolved_violations_are_counted_separately_from_new_ones() {
    let repo = fixture_copy("resolved");
    let text = repo.path().join("utils/text.ts");
    let original = fs::read_to_string(&text).expect("read text module");
    append(&text, "\nimport { run } from \"../commands/run.js\";\n");

    let baseline = baseline_for(repo.path());
    fs::write(&text, original).expect("restore text module");

    let report = report_for(repo.path(), &baseline);

    assert!(report.gate.passed, "a resolved violation must not block");
    assert_eq!(report.gate.checks.module_changes.new_count, 0);
    assert_eq!(report.gate.checks.reverse_edges.new_count, 0);
    assert_eq!(report.gate.checks.reverse_edges.resolved_count, 1);
    assert!(report.new_violations.is_empty());
    assert_eq!(
        report.resolved_violations,
        vec![Violation::ReverseEdge {
            source: "utils/text.ts".to_string(),
            target: "commands/run.ts".to_string(),
            edge_kind: "import".to_string(),
            source_layer: "utils".to_string(),
            target_layer: "commands".to_string(),
        }]
    );
}

#[test]
fn baseline_and_report_are_byte_deterministic_and_roundtrip() {
    let baseline = baseline_for(&fixture());
    let second_baseline = baseline_for(&fixture());
    let first_json = serde_json::to_string_pretty(&baseline).expect("serialize baseline");
    let second_json = serde_json::to_string_pretty(&second_baseline).expect("serialize baseline");
    assert_eq!(first_json, second_json, "baseline bytes must be stable");

    let decoded: DriftBaseline =
        serde_json::from_str(&first_json).expect("baseline must roundtrip through JSON");
    assert_eq!(
        serde_json::to_string_pretty(&decoded).expect("re-serialize baseline"),
        first_json,
        "deserializing and re-serializing must not reorder or drop fields"
    );

    let first_report =
        serde_json::to_string_pretty(&report_for(&fixture(), &baseline)).expect("serialize report");
    let second_report =
        serde_json::to_string_pretty(&report_for(&fixture(), &baseline)).expect("serialize report");
    assert_eq!(first_report, second_report, "report bytes must be stable");
}

#[test]
fn drift_schema_version_is_independent_from_other_artifacts() {
    assert_eq!(DRIFT_SCHEMA_VERSION, 2);

    let baseline = baseline_for(&fixture());
    assert_eq!(baseline.schema_version, DRIFT_SCHEMA_VERSION);

    let report = report_for(&fixture(), &baseline);
    assert_eq!(report.schema_version, DRIFT_SCHEMA_VERSION);

    let layers = build_layers(&fixture(), TOOL_VERSION).expect("layers");
    assert_eq!(
        layers.schema_version,
        arch_core::SCHEMA_VERSION,
        "drift must not reuse or overwrite the index/layers schema version"
    );
}
