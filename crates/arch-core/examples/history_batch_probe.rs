//! 临时采样对照探针（debug vs release，617 档；跑完可保留作复现工具）。
//!
//! 用法：history_batch_probe <仓库> [刻度数=617] [重复=3]

use arch_core::history::{build_history_prefixes, HistoryOptions};
use std::path::Path;
use std::process::Command;
use std::time::Instant;

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

fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).expect("finite"));
    sorted[sorted.len() / 2]
}

fn main() {
    let root = std::env::args().nth(1).expect("仓库路径");
    let root = Path::new(&root);
    let size = std::env::args()
        .nth(2)
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(617);
    let reps = std::env::args()
        .nth(3)
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(3);

    let times = first_parent_times(root);
    let ticks = &times[..size.min(times.len())];
    let mut samples = Vec::new();
    for _ in 0..reps {
        let start = Instant::now();
        build_history_prefixes(root, "batch-probe", HistoryOptions::default(), ticks)
            .expect("batch prefixes");
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    println!(
        "batch n={} total_ms=[{}] median_ms={:.1}",
        ticks.len(),
        samples
            .iter()
            .map(|value| format!("{value:.1}"))
            .collect::<Vec<_>>()
            .join(","),
        median(&samples)
    );
}
