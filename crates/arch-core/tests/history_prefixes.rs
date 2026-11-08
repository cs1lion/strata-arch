//! ticket 09 采样性能修复：批式前缀产物必须与逐刻度 `build_history` 逐字节一致，
//! 并满足 ticket 里预先钉死的 C 档预算（见
//! `docs/tickets/arch-tool/09-evolution-timeline-heat-ui.md`）。

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use arch_core::history::{build_history, build_history_prefixes, HistoryOptions};
use sha2::{Digest, Sha256};

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("arch-prefix-{tag}-{}-{nanos}", std::process::id()));
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

fn fixture_repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/history")
}

fn run_git(dir: &Path, args: &[&str], date: Option<&str>) -> String {
    let mut command = Command::new("git");
    command.arg("-C").arg(dir).args(args);
    command.env("GIT_CONFIG_NOSYSTEM", "1");
    if let Some(date) = date {
        command
            .env("GIT_AUTHOR_DATE", date)
            .env("GIT_COMMITTER_DATE", date)
            .env("GIT_AUTHOR_NAME", "Fixture")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.com")
            .env("GIT_COMMITTER_NAME", "Fixture")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.com");
    }
    let output = command.output().expect("run git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn write(dir: &Path, relative: &str, content: &str) {
    let path = dir.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    fs::write(path, content).expect("write file");
}

fn init_repo(dir: &Path) {
    run_git(dir, &["init", "-q"], None);
    run_git(dir, &["config", "user.name", "Fixture"], None);
    run_git(dir, &["config", "user.email", "fixture@example.com"], None);
    run_git(dir, &["config", "core.autocrlf", "false"], None);
    run_git(dir, &["config", "commit.gpgsign", "false"], None);
}

fn commit_at(dir: &Path, epoch: u64, message: &str) {
    let date = format!("@{epoch} +0000");
    run_git(dir, &["add", "-A"], None);
    run_git(dir, &["commit", "-q", "-m", message], Some(&date));
}

/// first-parent 刻度时间（按时间正序，与 `arch-view` 的 timeline 构造口径一致）。
fn tick_times(dir: &Path) -> Vec<u64> {
    let output = run_git(
        dir,
        &["log", "--reverse", "--first-parent", "--format=%ct"],
        None,
    );
    output
        .lines()
        .map(|line| line.trim().parse::<u64>().expect("epoch"))
        .collect()
}

fn legacy_artifact(dir: &Path, time: u64, max_commits: usize) -> Vec<u8> {
    let options = HistoryOptions {
        max_commits,
        since: None,
        until: Some(format!("@{}", time.saturating_add(1))),
    };
    let artifact = build_history(dir, "test", options).expect("legacy history");
    serde_json::to_vec(&artifact).expect("serialize legacy")
}

fn batch_artifacts(dir: &Path, times: &[u64], max_commits: usize) -> Vec<Vec<u8>> {
    let options = HistoryOptions {
        max_commits,
        since: None,
        until: None,
    };
    build_history_prefixes(dir, "test", options, times)
        .expect("batch history")
        .iter()
        .map(|artifact| serde_json::to_vec(artifact).expect("serialize batch"))
        .collect()
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(|left, right| left.partial_cmp(right).expect("finite"));
    values[values.len() / 2]
}

/// B4：固定样本 `tests/fixtures/history` 的每个刻度都逐字节等于旧实现。
#[test]
fn prefixes_are_byte_identical_to_per_tick_history() {
    let dir = fixture_repo();
    let times = tick_times(&dir);
    assert_eq!(times.len(), 6, "fixture 必须保持 6 个 first-parent 刻度");

    let batch = batch_artifacts(&dir, &times, 500);
    assert_eq!(batch.len(), times.len());

    for (index, (time, batch_bytes)) in times.iter().zip(batch.iter()).enumerate() {
        let legacy_bytes = legacy_artifact(&dir, *time, 500);
        assert_eq!(
            batch_bytes, &legacy_bytes,
            "刻度 {index} @{time} 与旧实现字节不等"
        );
        assert_eq!(
            sha256_hex(batch_bytes),
            sha256_hex(&legacy_bytes),
            "刻度 {index} @{time} SHA256 不等"
        );
        println!(
            "tick {index} @{time} sha256={} bytes={}",
            sha256_hex(batch_bytes),
            batch_bytes.len()
        );
    }
}

/// B4 + 反向用例 3：`--max-commits` 截断下批式与旧实现同口径。
#[test]
fn prefixes_honor_max_commits_truncation() {
    let dir = fixture_repo();
    let times = tick_times(&dir);
    let batch = batch_artifacts(&dir, &times, 2);
    assert_eq!(batch.len(), times.len());

    for (index, (time, batch_bytes)) in times.iter().zip(batch.iter()).enumerate() {
        let legacy_bytes = legacy_artifact(&dir, *time, 2);
        assert_eq!(
            batch_bytes, &legacy_bytes,
            "截断刻度 {index} @{time} 与旧实现字节不等"
        );
    }

    let last: serde_json::Value = serde_json::from_slice(&batch[batch.len() - 1]).expect("json");
    assert_eq!(last["window"]["maxCommits"], 2);
    assert_eq!(last["truncated"], true);
    assert_eq!(last["stats"]["commitsScanned"], 2);
    assert_eq!(last["stats"]["commitsAvailable"], 6);
}

/// 反向用例 1 + 2：反复修改必须累计；只改非源码文件的刻度不得变化。
#[test]
fn repeated_modifications_accumulate_and_unrelated_commit_is_neutral() {
    let temp = TempDir::new("repeat");
    let dir = temp.path();
    init_repo(dir);

    let base = 1_770_000_000u64;
    write(dir, "src/repeat.ts", "const v = 1;\n");
    commit_at(dir, base, "c1 repeat");

    write(dir, "src/repeat.ts", "const v = 2;\n");
    commit_at(dir, base + 3600, "c2 repeat");

    write(dir, "docs/notes.md", "# notes\n");
    commit_at(dir, base + 7200, "c3 unrelated");

    write(dir, "src/repeat.ts", "const v = 3;\n");
    commit_at(dir, base + 10800, "c4 repeat");

    write(dir, "src/repeat.ts", "const v = 4;\n");
    commit_at(dir, base + 14400, "c5 repeat");

    let times = tick_times(dir);
    assert_eq!(times.len(), 5);
    let batch = batch_artifacts(dir, &times, 500);

    let mut previous_total = 0u64;
    let mut previous_count = 0usize;
    for (index, bytes) in batch.iter().enumerate() {
        let artifact: serde_json::Value = serde_json::from_slice(bytes).expect("json");
        let modules = artifact["modules"].as_array().expect("modules");
        assert_eq!(
            modules.len(),
            1,
            "刻度 {index} 只应包含 src/repeat.ts，非源码文件不得进入指标"
        );
        let module = &modules[0];
        assert_eq!(module["id"], "src/repeat.ts");

        let expected_count =
            1 + usize::from(index >= 1) + usize::from(index >= 3) + usize::from(index >= 4);
        let count = module["commitCount"].as_u64().expect("count") as usize;
        assert_eq!(
            count, expected_count,
            "刻度 {index} commitCount 必须按真实 touches 累加"
        );
        assert!(count >= previous_count, "刻度 {index} commitCount 不得回退");
        previous_count = count;

        let total = module["churn"]["total"].as_u64().expect("churn");
        assert!(
            total >= previous_total,
            "刻度 {index} churn 必须累计而非被覆盖"
        );
        previous_total = total;
    }

    // c3 只改 docs/notes.md：刻度 3（index 2）的 modules 必须与刻度 2（index 1）逐字节相同。
    let at_c2: serde_json::Value = serde_json::from_slice(&batch[1]).expect("json");
    let at_c3: serde_json::Value = serde_json::from_slice(&batch[2]).expect("json");
    assert_eq!(
        serde_json::to_vec(&at_c2["modules"]).expect("modules"),
        serde_json::to_vec(&at_c3["modules"]).expect("modules"),
        "只改非源码文件的提交不得改变任何模块读数"
    );

    // 最终刻度：4 次 touch，churn 累计 4 增 3 删（每次覆盖旧内容，首提交无删除）。
    let last: serde_json::Value = serde_json::from_slice(&batch[4]).expect("json");
    let module = &last["modules"][0];
    assert_eq!(module["commitCount"].as_u64(), Some(4));
    assert_eq!(module["churn"]["additions"].as_u64(), Some(4));
    assert_eq!(module["churn"]["deletions"].as_u64(), Some(3));
    assert_eq!(module["churn"]["total"].as_u64(), Some(7));
}

fn build_synthetic_repo(commits: usize) -> TempDir {
    let temp = TempDir::new("synthetic");
    let dir = temp.path();
    init_repo(dir);

    // 用一次 fast-import 造 200 个提交，避免每个提交两次 git 进程把测试本身拖成分钟级。
    let base = 1_769_904_000u64;
    let mut stream = String::new();
    for index in 0..commits {
        let epoch = base + index as u64 * 3600;
        let module = index % 6;
        let message = format!("c{index}");
        let content = format!("export const v = {index};\n");
        stream.push_str("commit refs/heads/master\n");
        stream.push_str(&format!(
            "author Fixture <fixture@example.com> {epoch} +0000\n"
        ));
        stream.push_str(&format!(
            "committer Fixture <fixture@example.com> {epoch} +0000\n"
        ));
        stream.push_str(&format!("data {}\n{message}\n", message.len()));
        stream.push_str(&format!(
            "M 100644 inline src/mod_{module}.ts\ndata {}\n{content}",
            content.len()
        ));
        stream.push('\n');
    }

    let mut child = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["fast-import", "--quiet"])
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .stdin(std::process::Stdio::piped())
        .spawn()
        .expect("spawn git fast-import");
    child
        .stdin
        .as_mut()
        .expect("fast-import stdin")
        .write_all(stream.as_bytes())
        .expect("write fast-import stream");
    let status = child.wait().expect("wait fast-import");
    assert!(status.success(), "git fast-import failed");

    run_git(dir, &["symbolic-ref", "HEAD", "refs/heads/master"], None);
    temp
}

fn fixture_six_tick_runs() -> Vec<f64> {
    let dir = fixture_repo();
    let times = tick_times(&dir);
    let mut readings = Vec::new();
    for _ in 0..3 {
        let started = Instant::now();
        let artifacts = build_history_prefixes(&dir, "test", HistoryOptions::default(), &times)
            .expect("batch fixture");
        readings.push(started.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(artifacts.len(), times.len());
    }
    readings
}

/// C 档：B1/B2/B3 预算断言，3 次运行，打印每次读数、中位数与 p95（3 样本取最大）。
#[test]
fn batch_sampling_meets_pinned_budget() {
    let fixture_runs = fixture_six_tick_runs();
    let mut fixture_sorted = fixture_runs.clone();
    let fixture_median = median(&mut fixture_sorted);
    println!(
        "B1 fixture 6 ticks ms: {:?} median={fixture_median:.1} p95={:.1}",
        fixture_runs,
        fixture_runs.iter().copied().fold(0.0_f64, f64::max)
    );
    assert!(
        fixture_median <= 250.0,
        "B1 超预算: 6 刻度中位数 {fixture_median:.1}ms > 250ms"
    );

    let synthetic = build_synthetic_repo(200);
    let dir = synthetic.path();
    let times = tick_times(dir);
    assert_eq!(times.len(), 200);

    let mut six_runs = Vec::new();
    let mut twenty_runs = Vec::new();
    let mut full_runs = Vec::new();
    for _ in 0..3 {
        let started = Instant::now();
        build_history_prefixes(dir, "test", HistoryOptions::default(), &times[..6])
            .expect("batch six");
        six_runs.push(started.elapsed().as_secs_f64() * 1000.0);

        let started = Instant::now();
        build_history_prefixes(dir, "test", HistoryOptions::default(), &times[..20])
            .expect("batch twenty");
        twenty_runs.push(started.elapsed().as_secs_f64() * 1000.0);

        let started = Instant::now();
        build_history_prefixes(dir, "test", HistoryOptions::default(), &times).expect("batch full");
        full_runs.push(started.elapsed().as_secs_f64() * 1000.0);
    }

    let mut six_sorted = six_runs.clone();
    let mut twenty_sorted = twenty_runs.clone();
    let mut full_sorted = full_runs.clone();
    let six_median = median(&mut six_sorted);
    let twenty_median = median(&mut twenty_sorted);
    let full_median = median(&mut full_sorted);
    println!(
        "B3 200-commit repo: 6 ticks ms={six_runs:?} median={six_median:.1}; 20 ticks ms={twenty_runs:?} median={twenty_median:.1}; ratio={:.2}",
        twenty_median / six_median
    );
    println!(
        "B2 200 ticks ms={full_runs:?} median={full_median:.1} p95={:.1}",
        full_runs.iter().copied().fold(0.0_f64, f64::max)
    );

    let ratio = twenty_median / six_median;
    assert!(
        ratio <= 4.0,
        "B3 超预算: 20/6 刻度耗时比 {ratio:.2} > 4.0（平方增长约 11.1）"
    );
    assert!(
        full_median <= 1500.0,
        "B2 超预算: 200 刻度中位数 {full_median:.1}ms > 1500ms"
    );
}
