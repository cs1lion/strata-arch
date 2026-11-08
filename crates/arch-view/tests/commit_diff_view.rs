//! ticket 17：结构 diff 视图模型的 B 档断言。
//!
//! 断言只依赖公开模型：三色块、行序不归并改名、淡入最终态恒定、关闭态与静态逐位一致。

use arch_core::commit_diff::{
    CommitDiffArtifact, CommitDiffStats, CommitIdentity, FileChange, ModifiedFile, RenameHint,
};
use arch_view::diff::{
    packed_static, DiffFade, DiffKind, DiffModel, DIFF_ADDED_TINT, DIFF_LABEL, DIFF_MODIFIED_TINT,
    DIFF_REMOVED_TINT, FADE_FINAL_ALPHA,
};

fn sample_artifact() -> CommitDiffArtifact {
    CommitDiffArtifact {
        schema_version: 1,
        tool_version: "test".to_string(),
        from: CommitIdentity {
            commit: "from".to_string(),
            tree: "tree-from".to_string(),
        },
        to: CommitIdentity {
            commit: "to".to_string(),
            tree: "tree-to".to_string(),
        },
        stats: CommitDiffStats {
            added: 3,
            removed: 2,
            modified: 1,
            renames: 1,
        },
        added: vec![
            FileChange {
                path: "src/blank.ts".to_string(),
                symbol_count: 1,
                dependency_edges: 0,
                rename_hint: None,
            },
            FileChange {
                path: "src/legacy.ts".to_string(),
                symbol_count: 2,
                dependency_edges: 0,
                rename_hint: Some("src/old.ts".to_string()),
            },
            FileChange {
                path: "src/new.ts".to_string(),
                symbol_count: 2,
                dependency_edges: 0,
                rename_hint: None,
            },
        ],
        removed: vec![
            FileChange {
                path: "src/dead.ts".to_string(),
                symbol_count: 2,
                dependency_edges: 0,
                rename_hint: None,
            },
            FileChange {
                path: "src/old.ts".to_string(),
                symbol_count: 2,
                dependency_edges: 0,
                rename_hint: Some("src/legacy.ts".to_string()),
            },
        ],
        modified: vec![ModifiedFile {
            path: "src/a.ts".to_string(),
            symbol_count_from: 2,
            symbol_count_to: 3,
            dependency_edges_from: 0,
            dependency_edges_to: 0,
            reasons: vec!["symbol-count".to_string()],
        }],
        renames: vec![RenameHint {
            from: "src/old.ts".to_string(),
            to: "src/legacy.ts".to_string(),
            path_similarity: 0.6,
            symbol_overlap: 1.0,
        }],
    }
}

#[test]
fn model_orders_three_kinds_and_keeps_rename_rows() {
    let model = DiffModel::from_artifact(&sample_artifact());
    assert_eq!(model.added, 3);
    assert_eq!(model.removed, 2);
    assert_eq!(model.modified, 1);
    assert_eq!(model.renames, 1);
    assert_eq!(model.rows.len(), 6, "改名不得合并成一行");

    let kinds = model.rows.iter().map(|row| row.kind).collect::<Vec<_>>();
    assert_eq!(
        kinds,
        [
            DiffKind::Added,
            DiffKind::Added,
            DiffKind::Added,
            DiffKind::Removed,
            DiffKind::Removed,
            DiffKind::Modified,
        ]
    );

    let added = model
        .rows
        .iter()
        .find(|row| row.path == "src/legacy.ts")
        .expect("改名目标行");
    assert_eq!(added.rename_hint.as_deref(), Some("src/old.ts"));

    let removed = model
        .rows
        .iter()
        .find(|row| row.path == "src/old.ts")
        .expect("改名源行");
    assert_eq!(removed.rename_hint.as_deref(), Some("src/legacy.ts"));

    for (index, row) in model.rows.iter().enumerate() {
        assert_eq!(row.z_index, index as i32);
    }
}

#[test]
fn three_kinds_have_distinct_tints() {
    let tints = [DIFF_ADDED_TINT, DIFF_REMOVED_TINT, DIFF_MODIFIED_TINT];
    assert_eq!(
        tints
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        3,
        "三类底色必须互不相同"
    );
    assert!(tints.iter().all(|tint| *tint != DIFF_LABEL));
}

#[test]
fn fade_final_state_is_constant_across_frames() {
    let fade = DiffFade::new(true);
    let samples = [0.0_f32, 0.25, 0.5, 0.75, 1.0, 1.0, 1.0, 1.2, 2.0];

    let mut previous = 0.0_f32;
    for progress in samples {
        let alpha = fade.alpha_at(progress);
        assert!(
            alpha >= previous,
            "淡入必须单调，不得回退：{progress} -> {alpha} < {previous}"
        );
        previous = alpha;
    }

    let final_alpha = arch_view::playback::opacity_alpha_byte(fade.alpha_at(1.0));
    assert_eq!(final_alpha, FADE_FINAL_ALPHA, "最终态 alpha 必须到 255");

    for progress in [1.0_f32, 1.0, 2.0] {
        for kind in [DiffKind::Added, DiffKind::Removed, DiffKind::Modified] {
            assert_eq!(
                fade.packed_at(kind, progress),
                packed_static(kind),
                "progress={progress} 的最终色值必须恒定"
            );
        }
    }
}

#[test]
fn fade_off_matches_static_rendering_bitwise() {
    let fade = DiffFade::new(false);
    for progress in [0.0_f32, 0.1, 0.5, 0.9, 1.0, 2.0] {
        for kind in [DiffKind::Added, DiffKind::Removed, DiffKind::Modified] {
            assert_eq!(
                fade.packed_at(kind, progress),
                packed_static(kind),
                "关闭淡入后 progress={progress} 必须与静态渲染逐位一致"
            );
        }
    }
}

#[test]
fn view_sources_do_not_query_wall_clock_or_randomness() {
    let source_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    collect_rust_files(&source_root, &mut files);
    assert!(!files.is_empty(), "必须扫描到视图源码");

    let banned = [
        "Instant::now",
        "SystemTime::now",
        "Date::now",
        "rand::",
        "thread_rng",
    ];
    for file in files {
        let text = std::fs::read_to_string(&file).expect("read view source");
        for pattern in banned {
            assert!(
                !text.contains(pattern),
                "{} 命中禁用时间/随机源 {}",
                file.display(),
                pattern
            );
        }
    }
}

fn collect_rust_files(directory: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rust_files(&path, files);
        } else if path.extension().and_then(|value| value.to_str()) == Some("rs") {
            files.push(path);
        }
    }
}
