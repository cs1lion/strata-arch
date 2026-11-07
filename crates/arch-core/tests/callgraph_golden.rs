use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use arch_core::callgraph::{build_call_graph, build_call_graph_with_options, CallGraphOptions};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn fixture_dir() -> PathBuf {
    workspace_root().join("tests/fixtures/callgraph")
}

fn graph() -> arch_core::callgraph::CallGraphArtifact {
    build_call_graph(&fixture_dir(), "test").expect("call graph")
}

fn symbol_map(
    artifact: &arch_core::callgraph::CallGraphArtifact,
) -> BTreeMap<String, &arch_core::callgraph::SymbolNode> {
    artifact
        .symbols
        .iter()
        .map(|symbol| (symbol.id.clone(), symbol))
        .collect()
}

#[test]
fn golden_symbols_use_stable_ids_and_positions() {
    let artifact = graph();
    let symbols = symbol_map(&artifact);

    assert_eq!(artifact.schema_version, 2, "call graph schema version");
    assert_eq!(artifact.symbols.len(), 32, "fixture symbol baseline");
    assert_eq!(artifact.edges.len(), 6, "fixture call edge baseline");
    assert_eq!(
        artifact.module_edges.len(),
        3,
        "fixture module edge baseline"
    );
    assert_eq!(
        artifact.unresolved.len(),
        7,
        "default artifact must aggregate call-unattributed entries"
    );

    for id in [
        "src/util.ts#function:helper",
        "src/a.ts#function:alpha",
        "src/a.ts#function:beta",
        "src/a.ts#class:Service",
        "src/a.ts#method:Service.run",
        "src/a.ts#module-toplevel",
        "src/components.tsx#component:Widget",
        "src/b.ts#function:caller",
        "src/b.ts#function:unattributed",
        "src/b.ts#function:load",
        "src/b.ts#function:loadDynamic",
        "src/default.ts#function:NamedDefault",
        "src/default.ts#function:duplicateSameName",
        "src/other.ts#function:duplicateSameName",
        "src/anonymous.ts#function:default",
        "src/overload.ts#function:overloaded",
        "src/external.ts#function:useMemoValue",
        "src/external.ts#function:readIt",
        "src/external-bare.ts#function:classNames",
        "src/external-bare.ts#function:lint",
        "src/commonjs.js#function:cjsMember",
        "src/commonjs.js#function:cjsBare",
    ] {
        assert!(symbols.contains_key(id), "missing symbol {id}");
    }

    let alpha = symbols["src/a.ts#function:alpha"];
    assert_eq!(alpha.kind, "function");
    assert_eq!(alpha.name, "alpha");
    assert_eq!(alpha.line, 3);
    assert_eq!(alpha.column, 1);

    let service = symbols["src/a.ts#class:Service"];
    assert_eq!(service.kind, "class");
    let run = symbols["src/a.ts#method:Service.run"];
    assert_eq!(run.kind, "method");
    assert_eq!(run.line, 10);

    let widget = symbols["src/components.tsx#component:Widget"];
    assert_eq!(widget.kind, "component");

    assert_eq!(
        artifact
            .symbols
            .iter()
            .filter(|symbol| symbol.name == "overloaded")
            .count(),
        1,
        "overload signatures must collapse to the implementation symbol"
    );
}

#[test]
fn golden_cross_module_call_chain_and_jsx_edges_exist() {
    let artifact = graph();
    let edges = artifact
        .edges
        .iter()
        .map(|edge| {
            (
                edge.source.as_str(),
                edge.target.as_str(),
                edge.kind.as_str(),
            )
        })
        .collect::<BTreeSet<_>>();

    assert!(
        edges.contains(&(
            "src/b.ts#function:caller",
            "src/a.ts#function:alpha",
            "call"
        )),
        "caller -> alpha chain missing"
    );
    assert!(
        edges.contains(&(
            "src/a.ts#function:alpha",
            "src/util.ts#function:helper",
            "call"
        )),
        "alpha -> helper chain missing"
    );
    assert!(
        edges.contains(&(
            "src/b.ts#function:caller",
            "src/components.tsx#component:Widget",
            "jsx"
        )),
        "JSX component usage edge missing"
    );
    assert!(
        edges.contains(&(
            "src/a.ts#method:Service.run",
            "src/a.ts#function:alpha",
            "call"
        )),
        "method call edge missing"
    );
}

#[test]
fn golden_dynamic_import_edge_and_unresolved_records_coexist() {
    let artifact = graph();

    let dynamic_edge = artifact
        .edges
        .iter()
        .find(|edge| edge.kind == "dynamic-import")
        .expect("literal dynamic import edge");
    assert_eq!(dynamic_edge.source, "src/b.ts#function:load");
    assert_eq!(dynamic_edge.target, "src/a.ts#module-toplevel");

    let unattributed = artifact
        .unresolved
        .iter()
        .find(|entry| entry.reason == "call-unattributed");
    assert!(
        unattributed.is_none(),
        "call-unattributed must be aggregated into stats by default"
    );
    assert_eq!(
        artifact.stats.unattributed_calls, 1,
        "aggregate count must still include unattributed calls"
    );

    let dynamic_unresolved = artifact
        .unresolved
        .iter()
        .find(|entry| entry.reason == "dynamic-import")
        .expect("non-literal dynamic import must be recorded");
    assert!(
        dynamic_unresolved.expression.contains("dynamicTarget"),
        "unresolved expression should carry the dynamic target"
    );
}

#[test]
fn golden_edges_never_dangle_and_module_edges_project_symbol_edges() {
    let artifact = graph();
    let ids = artifact
        .symbols
        .iter()
        .map(|symbol| symbol.id.as_str())
        .collect::<BTreeSet<_>>();

    for edge in &artifact.edges {
        assert!(
            ids.contains(edge.source.as_str()),
            "dangling source: {}",
            edge.source
        );
        assert!(
            ids.contains(edge.target.as_str()),
            "dangling target: {}",
            edge.target
        );
    }

    for module_edge in &artifact.module_edges {
        let projected = artifact
            .edges
            .iter()
            .filter(|edge| {
                edge.source.split('#').next() == Some(module_edge.source.as_str())
                    && edge.target.split('#').next() == Some(module_edge.target.as_str())
            })
            .collect::<Vec<_>>();
        assert_eq!(
            module_edge.count,
            projected.len(),
            "module edge {0} -> {1} count must project symbol edges",
            module_edge.source,
            module_edge.target
        );
        let mut kinds = projected
            .iter()
            .map(|edge| edge.kind.clone())
            .collect::<Vec<_>>();
        kinds.sort();
        kinds.dedup();
        assert_eq!(module_edge.kinds, kinds);
    }

    let b_to_a = artifact
        .module_edges
        .iter()
        .find(|edge| edge.source == "src/b.ts" && edge.target == "src/a.ts")
        .expect("module edge b -> a");
    assert_eq!(b_to_a.kinds, vec!["call", "dynamic-import"]);
    assert_eq!(b_to_a.count, 2);
}

#[test]
fn golden_include_unattributed_restores_detail_without_changing_counts() {
    let default = graph();
    let full = build_call_graph_with_options(
        &fixture_dir(),
        "test",
        CallGraphOptions {
            include_unattributed: true,
        },
    )
    .expect("call graph with unattributed detail");

    assert_eq!(
        default.unresolved.len(),
        7,
        "default artifact aggregates call-unattributed"
    );
    assert_eq!(
        full.unresolved.len(),
        8,
        "diagnostic artifact keeps call-unattributed detail"
    );
    let detail = full
        .unresolved
        .iter()
        .find(|entry| entry.reason == "call-unattributed")
        .expect("unattributed detail is restored");
    assert_eq!(detail.module, "src/b.ts");
    assert!(detail.expression.contains("unknownMethod"));

    assert_eq!(
        default.stats.unattributed_calls,
        full.stats.unattributed_calls
    );
    assert_eq!(default.stats.external_calls, full.stats.external_calls);
    assert_eq!(default.stats.edges, full.stats.edges);
}

#[test]
fn golden_output_is_deterministic() {
    let first = serde_json::to_vec(&graph()).expect("serialize first");
    let second = serde_json::to_vec(&graph()).expect("serialize second");
    assert_eq!(first, second, "call graph must be byte deterministic");
}

#[test]
fn golden_noise_is_split_and_declaration_files_are_not_scanned() {
    let artifact = graph();
    let stats = serde_json::to_value(&artifact).expect("serialize")["stats"].clone();

    assert_eq!(
        stats["files"], 11,
        "declaration file must not count as scanned"
    );
    assert_eq!(stats["unattributedCalls"], 1, "unknown call baseline");
    assert_eq!(stats["unresolvedImports"], 1, "dynamic import baseline");
    assert_eq!(stats["externalCalls"], 6, "external call baseline");

    assert!(
        !artifact
            .unresolved
            .iter()
            .any(|entry| entry.reason == "call-unattributed"),
        "default artifact must not expose call-unattributed entries"
    );

    let external = artifact
        .unresolved
        .iter()
        .filter(|entry| entry.reason == "external-call")
        .collect::<Vec<_>>();
    assert_eq!(external.len(), 6, "external calls must be tagged");
    assert!(
        external
            .iter()
            .any(|entry| entry.expression.contains("React.useMemo")),
        "default external import member call must be tagged: {:?}",
        external
            .iter()
            .map(|entry| &entry.expression)
            .collect::<Vec<_>>()
    );
    assert!(
        external
            .iter()
            .any(|entry| entry.expression.contains("fs.readFileSync")),
        "namespace external import member call must be tagged"
    );

    let bare = external
        .iter()
        .filter(|entry| entry.module == "src/external-bare.ts")
        .collect::<Vec<_>>();
    assert_eq!(
        bare.len(),
        2,
        "bare external calls (clsx, new CLIEngine) must be tagged: {:?}",
        bare.iter()
            .map(|entry| &entry.expression)
            .collect::<Vec<_>>()
    );
    assert!(bare.iter().any(|entry| entry.expression.contains("clsx")));
    assert!(bare
        .iter()
        .any(|entry| entry.expression.contains("CLIEngine")));

    let commonjs = external
        .iter()
        .filter(|entry| entry.module == "src/commonjs.js")
        .collect::<Vec<_>>();
    assert_eq!(
        commonjs.len(),
        2,
        "CommonJS require bindings must be treated like ES imports: {:?}",
        commonjs
            .iter()
            .map(|entry| &entry.expression)
            .collect::<Vec<_>>()
    );
    assert!(commonjs
        .iter()
        .any(|entry| entry.expression.contains("React.useMemo")));
    assert!(commonjs
        .iter()
        .any(|entry| entry.expression.contains("readFileSync")));

    assert!(
        !artifact
            .symbols
            .iter()
            .any(|symbol| symbol.module.ends_with(".d.ts")),
        "declaration files must not produce symbols"
    );
    assert!(
        !artifact
            .unresolved
            .iter()
            .any(|entry| entry.module.ends_with(".d.ts")),
        "declaration files must not produce unresolved calls"
    );
}
