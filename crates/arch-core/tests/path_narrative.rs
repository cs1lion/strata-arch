//! N2 叙述式路径解释（A 档，先写测试）。
//!
//! 断言清单：
//! 1. 路径产物 schemaVersion 升到 2，每步带 evidence(file/line/column)、layerFrom/layerTo、conditional、testOnly、narrative。
//! 2. 首步没有入边证据；后续每步证据指向上一符号所在文件与边行列。
//! 3. `dynamic-import` / `require` 边必须标 conditional=true，叙述文本带「条件执行」。
//! 4. `render_narrative` 文本包含每一步与固定模板，不含「一定」「必然」「证明」等无证据措辞。
//! 5. 同一输入两次构建的 JSON 与叙述文本字节一致。

use std::path::{Path, PathBuf};

use arch_core::guided_path::{build_path, render_narrative, PATH_SCHEMA_VERSION};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn fixture(name: &str) -> PathBuf {
    workspace_root().join("tests/fixtures").join(name)
}

#[test]
fn every_step_carries_evidence_layers_and_narrative() {
    let artifact = build_path(
        &fixture("path"),
        "test",
        "src/cli.ts#module-toplevel",
        "src/utils/helper.ts#function:helper",
    )
    .expect("path");

    assert_eq!(artifact.schema_version, 2);
    assert_eq!(PATH_SCHEMA_VERSION, 2);
    assert_eq!(artifact.steps.len(), 3);

    let first = &artifact.steps[0];
    assert!(first.evidence.is_none(), "首步不应有入边证据");
    assert!(first.layer_from.is_none(), "首步不应有 from 层");
    assert_eq!(first.layer_to, first.layer);
    assert!(!first.narrative.is_empty());

    for (index, step) in artifact.steps.iter().enumerate().skip(1) {
        let previous = &artifact.steps[index - 1];
        let evidence = step.evidence.as_ref().expect("后续步必须有证据");
        assert_eq!(evidence.file, previous.module);
        assert!(evidence.line > 0);
        assert!(evidence.column > 0);
        assert_eq!(step.layer_from, Some(previous.layer));
        assert_eq!(step.layer_to, step.layer);
        assert!(step.narrative.contains(step.via.as_deref().unwrap()));
        assert!(step.narrative.contains(&evidence.file));
        assert!(format!("{}:{}", evidence.line, evidence.column).len() > 2);
    }

    let last = artifact.steps.last().unwrap();
    assert_eq!(last.layer, "utils");
    assert!(last.narrative.contains("unclassified"));
    assert!(last.narrative.contains("utils"));
}

#[test]
fn dynamic_import_edge_is_marked_conditional() {
    let artifact = build_path(
        &fixture("callgraph"),
        "test",
        "src/b.ts#function:load",
        "src/a.ts#module-toplevel",
    )
    .expect("dynamic path");

    assert_eq!(artifact.steps.len(), 2);
    let step = &artifact.steps[1];
    assert_eq!(step.via.as_deref(), Some("dynamic-import"));
    assert!(step.conditional, "dynamic-import 必须标条件执行");
    assert!(!step.test_only);
    let evidence = step.evidence.as_ref().expect("evidence");
    assert_eq!(evidence.file, "src/b.ts");
    assert_eq!(evidence.line, 14);
    assert!(step.narrative.contains("条件执行"));
}

#[test]
fn narrative_text_is_deterministic_and_avoids_unsupported_claims() {
    let first = build_path(
        &fixture("path"),
        "test",
        "src/cli.ts#module-toplevel",
        "src/utils/helper.ts#function:helper",
    )
    .expect("path");
    let second = build_path(
        &fixture("path"),
        "test",
        "src/cli.ts#module-toplevel",
        "src/utils/helper.ts#function:helper",
    )
    .expect("path");

    let first_json = serde_json::to_vec(&first).expect("json");
    let second_json = serde_json::to_vec(&second).expect("json");
    assert_eq!(first_json, second_json, "JSON 必须字节一致");

    let first_text = render_narrative(&first);
    let second_text = render_narrative(&second);
    assert_eq!(first_text, second_text, "叙述文本必须字节一致");

    for banned in ["一定", "必然", "证明"] {
        assert!(!first_text.contains(banned), "叙述含无证据措辞: {banned}");
    }
}

#[test]
fn narrative_text_lists_every_step() {
    let artifact = build_path(
        &fixture("path"),
        "test",
        "src/cli.ts#module-toplevel",
        "src/utils/helper.ts#function:helper",
    )
    .expect("path");
    let text = render_narrative(&artifact);

    assert!(text.starts_with("路径 "));
    assert!(text.contains(&artifact.from));
    assert!(text.contains(&artifact.to));
    for step in &artifact.steps {
        assert!(text.contains(&step.symbol), "缺步骤: {}", step.symbol);
    }
    let body_lines = text
        .lines()
        .filter(|line: &&str| line.starts_with('-'))
        .count();
    assert_eq!(body_lines, artifact.steps.len());
}
