//! ticket 18：P3 时间线离散播放的状态断言。
//!
//! 播放按 commit 刻度 dwell，不做插值：任何一帧的热力向量都必须完整等于某个刻度的
//! 实测值。判据与固定样本见 `docs/tickets/arch-tool/18-timeline-discrete-playback.md`。

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Command;

use arch_core::history::{build_history_prefixes, HistoryOptions};
use arch_view::timeline::{
    degradation_label, heat_for_churn, Heat, TimelineEntry, TimelineFlags, TimelinePlayback,
    TimelineState, DEGRADED_LABEL, PLAYBACK_DWELL_MS, PLAYBACK_SPEEDS, SHALLOW_LABEL,
};

fn fixture_repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/history")
}

fn tick_times(repo: &PathBuf) -> Vec<u64> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["log", "--reverse", "--first-parent", "--format=%ct"])
        .output()
        .expect("git log");
    assert!(output.status.success(), "git log 失败");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| line.trim().parse::<u64>().expect("epoch"))
        .collect()
}

fn timeline_from(repo: &PathBuf, times: &[u64]) -> TimelineState {
    let artifacts =
        build_history_prefixes(repo, "test", HistoryOptions::default(), times).expect("history");
    let entries = artifacts
        .into_iter()
        .enumerate()
        .map(|(index, artifact)| TimelineEntry {
            hash: format!("hash{index}"),
            time: times[index],
            subject: format!("c{index}"),
            artifact,
        })
        .collect::<Vec<_>>();
    TimelineState::new(entries)
}

fn fixture_timeline() -> TimelineState {
    let repo = fixture_repo();
    let times = tick_times(&repo);
    assert_eq!(times.len(), 6, "fixture 必须保持 6 个刻度");
    timeline_from(&repo, &times)
}

fn union_ids(timeline: &TimelineState) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    for entry in timeline.entries() {
        for module in &entry.artifact.modules {
            ids.insert(module.id.clone());
        }
    }
    ids
}

/// 某个刻度应有且仅有的热力向量；缺失模块用中性灰。
fn expected_heat(timeline: &TimelineState, index: usize, ids: &BTreeSet<String>) -> Vec<Heat> {
    let entry = &timeline.entries()[index];
    ids.iter()
        .map(|id| {
            match entry
                .artifact
                .modules
                .iter()
                .find(|module| &module.id == id)
            {
                Some(module) => heat_for_churn(module.churn.total, module.commit_count),
                None => Heat {
                    color: arch_view::timeline::HEAT_NEUTRAL,
                    size_scale_percent: 100,
                    neutral: true,
                },
            }
        })
        .collect()
}

fn displayed_heat(timeline: &mut TimelineState, playback: &TimelinePlayback) -> Vec<Heat> {
    assert!(timeline.scrub_to(playback.current()));
    let ids = union_ids(timeline);
    ids.iter()
        .map(|id| timeline.heat(id))
        .collect::<Vec<_>>()
        .into_iter()
        .collect()
}

/// 判据 1：同一步多帧热力向量恒定，切步时整向量离散跳变，绝不混合。
#[test]
fn dwell_keeps_heat_constant_and_switches_discretely() {
    let mut timeline = fixture_timeline();
    let ids = union_ids(&timeline);
    let mut playback = TimelinePlayback::new(timeline.ticks());
    playback.on_scrub(0);
    assert!(playback.play(), "6 个刻度应可播放");

    let mut frame = 0;
    let mut switches = 0;
    let mut frames_per_tick = 0;
    let mut dwell_readings = Vec::new();
    while playback.is_playing() && frame < 500 {
        let before = playback.current();
        let changed = playback.on_frame(16.0);
        frame += 1;
        frames_per_tick += 1;
        if changed {
            assert_eq!(
                playback.current(),
                before + 1,
                "每帧最多前进一个刻度，不允许跳刻度"
            );
            switches += 1;
            dwell_readings.push(frames_per_tick);
            frames_per_tick = 0;
        }

        // 每一帧的画面对应且仅对应当前刻度的实测热力。
        let observed = displayed_heat(&mut timeline, &playback);
        assert_eq!(
            observed,
            expected_heat(&timeline, playback.current(), &ids),
            "第 {frame} 帧出现非该刻度的热力状态（插值/混合）"
        );
        assert_eq!(
            timeline.current_index(),
            playback.current(),
            "视图索引必须与播放索引同步"
        );
        assert!(playback.current() < timeline.ticks());
    }

    println!(
        "dwell 帧数（16ms/帧, 1x, 每段到切换为止）: {dwell_readings:?}; 热力向量维度 {}",
        ids.len()
    );
    assert_eq!(switches, 5, "6 个刻度应恰好切换 5 次");
    assert!(playback.is_ended(), "播到末刻度必须进入结束态");
    assert!(!playback.is_playing());
}

/// 判据 2 + 3：暂停、单步、调速、明确结束态。
#[test]
fn pause_step_speed_and_end_state() {
    let mut playback = TimelinePlayback::new(6);
    playback.on_scrub(0);
    assert!(playback.play());
    for _ in 0..20 {
        playback.on_frame(16.0);
    }
    assert_eq!(playback.current(), 0, "700ms 之前不得推进");
    playback.pause();
    assert!(!playback.is_playing());
    for _ in 0..200 {
        assert!(!playback.on_frame(16.0), "暂停后不得推进");
    }
    assert_eq!(playback.current(), 0);

    assert!(playback.step_forward());
    assert_eq!(playback.current(), 1);
    assert!(!playback.is_playing(), "单步必须自动暂停");
    assert!(playback.step_back());
    assert_eq!(playback.current(), 0);
    assert!(!playback.step_back(), "起点不能再后退");

    // 2x：350ms 即可推进。
    let mut fast = TimelinePlayback::new(6);
    fast.on_scrub(0);
    assert_eq!(fast.cycle_speed(), 2.0);
    assert_eq!(fast.speed(), PLAYBACK_SPEEDS[2]);
    assert!(fast.play());
    for _ in 0..21 {
        fast.on_frame(16.0);
    }
    assert_eq!(fast.current(), 0, "336ms < 350ms 不得推进");
    assert!(fast.on_frame(16.0));
    assert_eq!(fast.current(), 1);

    // 0.5x：1400ms 才推进。
    let mut slow = TimelinePlayback::new(6);
    slow.on_scrub(0);
    let speed = slow.cycle_speed();
    assert_eq!(speed, 2.0);
    let speed = slow.cycle_speed();
    assert_eq!(speed, PLAYBACK_SPEEDS[0], "速度档位应循环回 0.5x");
    assert!(slow.play());
    for _ in 0..87 {
        slow.on_frame(16.0);
    }
    assert_eq!(slow.current(), 0, "1392ms < 1400ms 不得推进");
    assert!(slow.on_frame(16.0));
    assert_eq!(slow.current(), 1);

    // 结束态：不回绕，不再调度帧。
    let mut ending = TimelinePlayback::new(6);
    ending.on_scrub(0);
    assert!(ending.play());
    for _ in 0..5 {
        assert!(ending.on_frame(PLAYBACK_DWELL_MS));
    }
    assert_eq!(ending.current(), 5);
    assert!(ending.is_ended());
    assert!(!ending.is_playing());
    assert!(!ending.should_drive_frames());
    for _ in 0..50 {
        assert!(!ending.on_frame(PLAYBACK_DWELL_MS), "结束态不得回绕");
    }
    assert_eq!(ending.current(), 5);
    assert!(!ending.step_forward(), "末刻度不能再前进");
}

/// 判据 8：只有 1 个刻度时无可播放。
#[test]
fn single_tick_history_cannot_play() {
    let mut playback = TimelinePlayback::new(1);
    assert!(!playback.play());
    assert!(!playback.is_playing());
    assert!(!playback.step_forward());
    assert!(!playback.step_back());
    assert!(!playback.on_frame(PLAYBACK_DWELL_MS));
    assert_eq!(playback.current(), 0);
    assert!(!playback.should_drive_frames());
}

/// 判据 8：所有刻度指标完全相同 → 每刻度热力向量相同，不得伪造波动。
#[test]
fn identical_metrics_do_not_fake_motion() {
    let repo = fixture_repo();
    let times = tick_times(&repo);
    let same = times[3];
    let mut timeline = timeline_from(&repo, &[same, same, same]);
    let ids = union_ids(&timeline);
    let expected = expected_heat(&timeline, 0, &ids);

    let mut playback = TimelinePlayback::new(3);
    playback.on_scrub(0);
    assert!(playback.play());
    let mut advanced = 0;
    for _ in 0..300 {
        if playback.on_frame(16.0) {
            advanced += 1;
        }
        let observed = displayed_heat(&mut timeline, &playback);
        assert_eq!(
            observed, expected,
            "指标相同的刻度不得因播放产生任何视觉变化"
        );
    }
    println!("相同指标刻度热力向量（{advanced} 次推进）: {expected:?}");
    assert_eq!(advanced, 2, "索引仍应推进，但画面读数不变");
    assert_eq!(playback.current(), 2);
    assert!(playback.is_ended());
}

/// 判据 8：播放中拖动时间轴 → 停止播放并跟手。
#[test]
fn scrub_mid_play_stops_and_follows() {
    let mut playback = TimelinePlayback::new(6);
    playback.on_scrub(0);
    assert!(playback.play());
    assert!(playback.on_frame(PLAYBACK_DWELL_MS));
    assert_eq!(playback.current(), 1);

    playback.on_scrub(4);
    assert_eq!(playback.current(), 4);
    assert!(!playback.is_playing(), "拖动后必须停止播放");
    assert!(!playback.is_ended(), "拖到中间刻度不应进入结束态");
    for _ in 0..100 {
        assert!(!playback.on_frame(PLAYBACK_DWELL_MS));
    }
    assert_eq!(playback.current(), 4, "停止后必须跟手，不得被帧推进覆盖");
}

/// 判据 6：关闭播放后与静态时间线一致，不再驱动帧。
#[test]
fn disabled_playback_matches_static_timeline() {
    let mut timeline = fixture_timeline();
    let ticks = timeline.ticks();
    let static_heat = displayed_heat(&mut timeline, &TimelinePlayback::new(ticks));
    assert_eq!(timeline.current_index(), 5);

    let mut playback = TimelinePlayback::new(6);
    playback.set_enabled(false);
    assert!(!playback.enabled());
    assert!(!playback.play());
    assert!(!playback.should_drive_frames());
    for _ in 0..200 {
        assert!(!playback.on_frame(16.0));
    }
    assert_eq!(playback.current(), 5);
    assert_eq!(displayed_heat(&mut timeline, &playback), static_heat);
    println!(
        "关闭播放后索引={}, 静态索引={}, 热力向量一致",
        playback.current(),
        timeline.current_index()
    );

    // 播放中关闭：立刻冻结，之后不再推进。
    let mut stopping = TimelinePlayback::new(6);
    stopping.on_scrub(2);
    assert!(stopping.play());
    assert!(stopping.on_frame(PLAYBACK_DWELL_MS));
    let frozen = stopping.current();
    stopping.set_enabled(false);
    assert!(!stopping.is_playing());
    for _ in 0..50 {
        assert!(!stopping.on_frame(PLAYBACK_DWELL_MS));
    }
    assert_eq!(stopping.current(), frozen);
}

/// 判据 4：采样/截断/浅克隆必须有可见标记文案。
#[test]
fn degradation_labels_are_visible_text() {
    let normal = TimelineFlags {
        sampled: false,
        truncated: false,
        shallow: false,
    };
    assert_eq!(degradation_label(normal), None);

    let sampled = TimelineFlags {
        sampled: true,
        ..normal
    };
    assert_eq!(degradation_label(sampled), Some(DEGRADED_LABEL));
    let truncated = TimelineFlags {
        truncated: true,
        ..normal
    };
    assert_eq!(degradation_label(truncated), Some(DEGRADED_LABEL));
    let shallow = TimelineFlags {
        shallow: true,
        ..normal
    };
    assert_eq!(degradation_label(shallow), Some(SHALLOW_LABEL));
}
