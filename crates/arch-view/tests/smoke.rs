//! ticket 11 断言清单（B 档，先写测试）：
//!
//! 1. `arch-view smoke tests/fixtures/tiny` 在 3 秒内退出码 `0`。
//! 2. stdout 含 `smoke: frame rendered`，证明窗口创建后至少渲染一帧。
//! 3. stderr 不含 panic 文本。
//! 4. 缺参数退出码非 `0` 且给出用法。
//!
//! 截图能力与 mac/Linux 运行不在本测试范围（见 ticket 11 开放项）。

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn fixture_tiny() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/tiny")
}

fn run_smoke() -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_arch-view"))
        .arg("smoke")
        .arg(fixture_tiny())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn arch-view smoke");

    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match child.try_wait().expect("poll smoke process") {
            Some(_) => return child.wait_with_output().expect("collect smoke output"),
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let output = child
                    .wait_with_output()
                    .expect("collect killed smoke output");
                panic!(
                    "smoke 未在 3 秒内退出: stdout={} stderr={}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    }
}

#[test]
fn smoke_renders_one_frame_and_exits_cleanly() {
    let output = run_smoke();
    assert!(
        output.status.success(),
        "smoke failed with {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("smoke: frame rendered"),
        "missing frame marker: {stdout}"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("panicked") && !stderr.contains("RUST_BACKTRACE"),
        "smoke reported a panic: {stderr}"
    );
}

#[test]
fn smoke_without_repo_reports_usage() {
    let output = Command::new(env!("CARGO_BIN_EXE_arch-view"))
        .arg("smoke")
        .output()
        .expect("run arch-view smoke without repo");
    assert!(!output.status.success(), "missing repo must fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("用法"), "stderr must show usage: {stderr}");
}
