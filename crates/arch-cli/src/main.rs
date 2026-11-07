use arch_core::{build_index, ScanError, SCHEMA_VERSION};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "用法:
  arch-cli --schema
  arch-cli index <目标仓库> [--out <产物文件>]";

enum CliError {
    Input(String),
    Internal(String),
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(CliError::Input(message)) => {
            eprintln!("错误: {message}");
            ExitCode::from(2)
        }
        Err(CliError::Internal(message)) => {
            eprintln!("内部错误: {message}");
            ExitCode::from(3)
        }
    }
}

fn run() -> Result<(), CliError> {
    let mut args = env::args().skip(1);
    let Some(command) = args.next() else {
        return Err(CliError::Input(format!("缺少子命令\n{USAGE}")));
    };

    match command.as_str() {
        "--schema" => {
            if args.next().is_some() {
                return Err(CliError::Input("--schema 不接受额外参数".to_string()));
            }
            println!("{SCHEMA_VERSION}");
            Ok(())
        }
        "index" => run_index(args),
        other => Err(CliError::Input(format!("未知子命令: {other}\n{USAGE}"))),
    }
}

fn run_index(mut args: impl Iterator<Item = String>) -> Result<(), CliError> {
    let target = args
        .next()
        .ok_or_else(|| CliError::Input(format!("index 缺少目标仓库路径\n{USAGE}")))?;
    if target.starts_with("--") {
        return Err(CliError::Input(format!("index 缺少目标仓库路径\n{USAGE}")));
    }

    let mut out: Option<PathBuf> = None;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--out" => {
                let value = args
                    .next()
                    .ok_or_else(|| CliError::Input("--out 缺少文件路径".to_string()))?;
                out = Some(PathBuf::from(value));
            }
            other => {
                return Err(CliError::Input(format!("未知参数: {other}\n{USAGE}")));
            }
        }
    }

    let artifact = build_index(&PathBuf::from(&target), env!("CARGO_PKG_VERSION")).map_err(
        |error| match error {
            ScanError::TooManyFiles { .. } | ScanError::NotADirectory(_) => {
                CliError::Input(error.to_string())
            }
            ScanError::Io(_) => CliError::Internal(error.to_string()),
        },
    )?;

    let json = serde_json::to_string_pretty(&artifact)
        .map_err(|error| CliError::Internal(format!("序列化产物失败: {error}")))?;

    if let Some(path) = out {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).map_err(|error| {
                    CliError::Internal(format!("创建输出目录失败 {}: {error}", parent.display()))
                })?;
            }
        }
        fs::write(&path, format!("{json}\n")).map_err(|error| {
            CliError::Internal(format!("写入产物失败 {}: {error}", path.display()))
        })?;
    } else {
        println!("{json}");
    }

    Ok(())
}
