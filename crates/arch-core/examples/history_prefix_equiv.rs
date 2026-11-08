//! 临时逐字节等价抽查（真实 tsx 历史，跑完即删）。
//!
//! 用法：cargo run -q -p arch-core --example prefix_equiv -- <tsx 仓库路径>

use arch_core::history::{build_history, build_history_prefixes, HistoryOptions};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::process::Command;

fn first_parent_times(root: &Path) -> Vec<u64> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["log", "--first-parent", "--reverse", "--format=%ct"])
        .output()
        .expect("git log");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| line.trim().parse::<u64>().expect("epoch"))
        .collect()
}

fn sha256(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn main() {
    let root = std::env::args().nth(1).expect("仓库路径");
    let root = Path::new(&root);
    let sizes = std::env::args()
        .skip(2)
        .map(|value| value.parse::<usize>().expect("刻度数"))
        .collect::<Vec<_>>();
    let sizes = if sizes.is_empty() { vec![6, 20] } else { sizes };
    let times = first_parent_times(root);
    for size in sizes {
        let ticks = &times[..size];
        let batch = build_history_prefixes(root, "scale-curve", HistoryOptions::default(), ticks)
            .expect("batch");
        let mut all_equal = true;
        for (index, time) in ticks.iter().enumerate() {
            let legacy = build_history(
                root,
                "scale-curve",
                HistoryOptions {
                    until: Some(format!("@{}", time.saturating_add(1))),
                    ..Default::default()
                },
            )
            .expect("legacy");
            let legacy_json = serde_json::to_string(&legacy).expect("json");
            let batch_json = serde_json::to_string(&batch[index]).expect("json");
            let equal = legacy_json == batch_json;
            all_equal &= equal;
            println!(
                "n={} tick={} equal={} legacy_sha={} batch_sha={}",
                size,
                index,
                equal,
                sha256(&legacy_json),
                sha256(&batch_json)
            );
        }
        println!("n={} all_equal={}", size, all_equal);
    }
}
