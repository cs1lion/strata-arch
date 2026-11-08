//! 临时规模曲线测量（验证 ticket 09「O(n²)」诊断，跑完即删，不入库）。
//!
//! 用法：
//!   cargo run -p arch-core --example scale_curve -- <tsx 仓库路径> [reps]

use arch_core::history::{build_history, build_history_prefixes, HistoryOptions};
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
    let reps = std::env::args()
        .nth(2)
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(3);

    let times = first_parent_times(root);
    println!("first_parent_commits={}", times.len());

    let sizes = [6usize, 20usize, 100usize, times.len()];
    for &size in &sizes {
        let ticks = &times[..size];

        let mut per_tick_totals = Vec::new();
        for _ in 0..reps {
            let start = Instant::now();
            for time in ticks {
                build_history(
                    root,
                    "scale-curve",
                    HistoryOptions {
                        until: Some(format!("@{}", time.saturating_add(1))),
                        ..Default::default()
                    },
                )
                .expect("per-tick history");
            }
            per_tick_totals.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        let per_tick_median = median(&per_tick_totals);
        println!(
            "per_tick n={} total_ms=[{}] median_ms={:.1} per_tick_ms={:.2}",
            size,
            per_tick_totals
                .iter()
                .map(|value| format!("{value:.1}"))
                .collect::<Vec<_>>()
                .join(","),
            per_tick_median,
            per_tick_median / size as f64
        );

        let mut batch_totals = Vec::new();
        for _ in 0..reps {
            let start = Instant::now();
            build_history_prefixes(root, "scale-curve", HistoryOptions::default(), ticks)
                .expect("batch prefixes");
            batch_totals.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        let batch_median = median(&batch_totals);
        println!(
            "batch n={} total_ms=[{}] median_ms={:.1} per_tick_ms={:.2}",
            size,
            batch_totals
                .iter()
                .map(|value| format!("{value:.1}"))
                .collect::<Vec<_>>()
                .join(","),
            batch_median,
            batch_median / size as f64
        );
    }

    // 单刻度固定成本：同一次 build_history 在不同历史位置上的耗时。
    let probes = [("earliest", 0usize), ("latest", times.len() - 1)];
    for (label, index) in probes {
        let mut samples = Vec::new();
        for _ in 0..reps {
            let start = Instant::now();
            build_history(
                root,
                "scale-curve",
                HistoryOptions {
                    until: Some(format!("@{}", times[index].saturating_add(1))),
                    ..Default::default()
                },
            )
            .expect("probe history");
            samples.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        println!(
            "single_tick {} total_ms=[{}] median_ms={:.1}",
            label,
            samples
                .iter()
                .map(|value| format!("{value:.1}"))
                .collect::<Vec<_>>()
                .join(","),
            median(&samples)
        );
    }
}
