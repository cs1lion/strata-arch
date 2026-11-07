//! ticket 02 的样本仓回归基线。
//!
//! 两个样本仓固定在指定 commit（见 `tests/corpus/README` 或下载说明），
//! 这里锁定读数并检查产物不变量。读数变化必须显式修改本文件并在提交信息中说明。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use arch_core::{build_index, IndexArtifact};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn corpus(name: &str) -> PathBuf {
    let path = workspace_root().join("tests").join("corpus").join(name);
    assert!(
        path.is_dir(),
        "missing corpus sample at {}; download the pinned commit into tests/corpus/{name}",
        path.display()
    );
    path
}

fn index(name: &str) -> IndexArtifact {
    build_index(&corpus(name), "golden").expect("index should succeed")
}

fn count_modules(artifact: &IndexArtifact, kind: &str) -> usize {
    artifact
        .modules
        .iter()
        .filter(|module| module.kind == kind)
        .count()
}

fn count_edges(artifact: &IndexArtifact, kind: &str) -> usize {
    artifact
        .edges
        .iter()
        .filter(|edge| edge.kind == kind)
        .count()
}

fn count_unresolved(artifact: &IndexArtifact, reason: &str) -> usize {
    artifact
        .unresolved
        .iter()
        .filter(|item| item.reason == reason)
        .count()
}

fn assert_invariants(artifact: &IndexArtifact) {
    let ids = artifact
        .modules
        .iter()
        .map(|module| module.id.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        ids.len(),
        artifact.modules.len(),
        "module ids must be unique"
    );

    let sorted_ids = artifact
        .modules
        .iter()
        .map(|module| module.id.clone())
        .collect::<Vec<_>>();
    assert!(
        sorted_ids.windows(2).all(|pair| pair[0] <= pair[1]),
        "modules must already be sorted by id"
    );

    let edge_keys = artifact
        .edges
        .iter()
        .map(|edge| (&edge.source, &edge.target, &edge.kind))
        .collect::<Vec<_>>();
    assert!(
        edge_keys.windows(2).all(|pair| pair[0] <= pair[1]),
        "edges must already be sorted by (source, target, kind)"
    );

    for edge in &artifact.edges {
        assert!(
            ids.contains(&edge.source),
            "edge source {} missing from modules",
            edge.source
        );
        assert!(
            ids.contains(&edge.target),
            "edge target {} missing from modules",
            edge.target
        );
    }

    let unresolved_keys = artifact
        .unresolved
        .iter()
        .map(|item| (&item.file, &item.specifier, &item.reason))
        .collect::<Vec<_>>();
    assert!(
        unresolved_keys.windows(2).all(|pair| pair[0] <= pair[1]),
        "unresolved must already be sorted by (file, specifier, reason)"
    );
    for item in &artifact.unresolved {
        assert!(
            ids.contains(&item.file),
            "unresolved file {} missing from modules",
            item.file
        );
    }

    for module in &artifact.modules {
        assert!(
            !module.path.contains("node_modules"),
            "external dependencies must stay at package level: {}",
            module.path
        );
        assert!(
            !module.path.contains('\\'),
            "module paths must use forward slashes: {}",
            module.path
        );
    }

    assert_eq!(artifact.stats.files, count_modules(artifact, "source"));
    assert_eq!(artifact.stats.modules, artifact.modules.len());
    assert_eq!(artifact.stats.edges, artifact.edges.len());
    assert_eq!(artifact.stats.unresolved, artifact.unresolved.len());
}

fn assert_idempotent(name: &str) {
    let first = serde_json::to_vec(&index(name)).expect("serialize first artifact");
    let second = serde_json::to_vec(&index(name)).expect("serialize second artifact");
    assert_eq!(
        first, second,
        "{name} artifact must be byte-identical across runs"
    );
}

#[test]
fn tsx_golden_baseline() {
    let artifact = index("tsx");

    assert_eq!(artifact.stats.files, 95);
    assert_eq!(artifact.stats.modules, 136);
    assert_eq!(artifact.stats.edges, 379);
    assert_eq!(artifact.stats.unresolved, 10);
    assert_eq!(count_modules(&artifact, "source"), 95);
    assert_eq!(count_modules(&artifact, "external-package"), 41);
    assert_eq!(count_edges(&artifact, "import"), 376);
    assert_eq!(count_edges(&artifact, "dynamic-import"), 2);
    assert_eq!(count_edges(&artifact, "require"), 1);
    assert_eq!(count_unresolved(&artifact, "dynamic-import"), 5);
    assert_eq!(count_unresolved(&artifact, "unsupported-extension"), 5);
    assert_eq!(count_unresolved(&artifact, "relative-missing"), 0);

    assert!(
        artifact.edges.iter().any(|edge| {
            edge.source == "src/cli.ts" && edge.target == "src/run.ts" && edge.kind == "import"
        }),
        "src/cli.ts must resolve ./run.js to src/run.ts"
    );

    assert_invariants(&artifact);
    assert_idempotent("tsx");
}

#[test]
fn excalidraw_golden_baseline() {
    let artifact = index("excalidraw");

    assert_eq!(artifact.stats.files, 718);
    assert_eq!(artifact.stats.modules, 802);
    assert_eq!(artifact.stats.edges, 4363);
    assert_eq!(artifact.stats.unresolved, 334);
    assert_eq!(count_modules(&artifact, "source"), 718);
    assert_eq!(count_modules(&artifact, "external-package"), 84);
    assert_eq!(count_edges(&artifact, "import"), 4292);
    assert_eq!(count_edges(&artifact, "dynamic-import"), 26);
    assert_eq!(count_edges(&artifact, "require"), 45);
    assert_eq!(count_unresolved(&artifact, "unsupported-extension"), 323);
    assert_eq!(count_unresolved(&artifact, "dynamic-import"), 8);

    assert!(
        artifact.edges.iter().any(|edge| {
            edge.source == "excalidraw-app/App.tsx"
                && edge.target == "packages/excalidraw/index.tsx"
                && edge.kind == "import"
        }),
        "excalidraw-app/App.tsx must resolve @excalidraw/excalidraw"
    );
    assert!(
        artifact
            .modules
            .iter()
            .any(|module| module.id == "external:react"),
        "excalidraw must expose a package-level react node"
    );

    // 根 tsconfig 的 include 是 ["packages", "excalidraw-app"]；
    // 该范围内的 @excalidraw/ 别名必须全部命中。
    let scoped_alias_misses = artifact
        .unresolved
        .iter()
        .filter(|item| {
            item.reason == "alias-missing"
                && item.specifier.starts_with("@excalidraw/")
                && (item.file.starts_with("excalidraw-app/") || item.file.starts_with("packages/"))
        })
        .collect::<Vec<_>>();
    assert!(
        scoped_alias_misses.is_empty(),
        "alias misses inside tsconfig include scope: {scoped_alias_misses:?}"
    );

    assert_invariants(&artifact);
    assert_idempotent("excalidraw");
}
