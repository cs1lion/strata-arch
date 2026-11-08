use arch_core::callgraph::{build_call_graph_with_options, CallGraphOptions};
use arch_core::drift::{build_drift, record_baseline, DriftBaseline, DRIFT_SCHEMA_VERSION};
use arch_core::guided_path::{build_path, list_entries, render_narrative, PathError};
use arch_core::history::{build_history, HistoryError, HistoryOptions};
use arch_core::{build_index, layers::build_layers, ScanError, SCHEMA_VERSION};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "用法:
  arch-cli --schema
  arch-cli index <目标仓库> [--out <产物文件>]
  arch-cli layers <目标仓库> [--out <产物文件>]
  arch-cli callgraph <目标仓库> [--out <产物文件>] [--include-unattributed]
  arch-cli history <目标仓库> [--repo <目标仓库>] [--out <产物文件>]
      [--max-commits <N>] [--since <日期>] [--until <日期>]
  arch-cli path [<目标仓库>|--repo <目标仓库>] [--out <产物文件>] --entries
  arch-cli path [<目标仓库>|--repo <目标仓库>] [--out <产物文件>]
      --from <入口ID> --to <符号ID> [--narrative]
  arch-cli drift <目标仓库> --record --out <基线文件>
  arch-cli drift <目标仓库> --base <基线文件> [--out <报告文件>]";

enum CliError {
    Input(String),
    /// 历史不可用：非 git、无提交、浅克隆。
    History(String),
    /// 调用图中不存在从入口到目标的路径。
    Unreachable(String),
    /// 漂移门禁未通过（报告已完整输出，退出码 1）。
    DriftDetected(String),
    Internal(String),
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(CliError::Input(message)) => {
            eprintln!("错误: {message}");
            ExitCode::from(2)
        }
        Err(CliError::History(message)) => {
            eprintln!("错误: {message}");
            ExitCode::from(5)
        }
        Err(CliError::Unreachable(message)) => {
            eprintln!("错误: {message}");
            ExitCode::from(4)
        }
        Err(CliError::DriftDetected(message)) => {
            eprintln!("错误: {message}");
            ExitCode::from(1)
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
        "layers" => run_layers(args),
        "callgraph" => run_callgraph(args),
        "history" => run_history(args),
        "path" => run_path(args),
        "drift" => run_drift(args),
        other => Err(CliError::Input(format!("未知子命令: {other}\n{USAGE}"))),
    }
}

fn run_index(args: impl Iterator<Item = String>) -> Result<(), CliError> {
    let (target, out) = parse_repo_and_out("index", args)?;
    let artifact = build_index(&target, env!("CARGO_PKG_VERSION")).map_err(scan_error)?;
    let json = serde_json::to_string_pretty(&artifact)
        .map_err(|error| CliError::Internal(format!("序列化产物失败: {error}")))?;
    emit_json(&json, out)
}

fn run_layers(args: impl Iterator<Item = String>) -> Result<(), CliError> {
    let (target, out) = parse_repo_and_out("layers", args)?;
    let artifact = build_layers(&target, env!("CARGO_PKG_VERSION")).map_err(scan_error)?;
    let json = serde_json::to_string_pretty(&artifact)
        .map_err(|error| CliError::Internal(format!("序列化产物失败: {error}")))?;
    emit_json(&json, out)
}

fn run_callgraph(args: impl Iterator<Item = String>) -> Result<(), CliError> {
    let (target, out, include_unattributed) = parse_callgraph_args(args)?;
    let artifact = build_call_graph_with_options(
        &target,
        env!("CARGO_PKG_VERSION"),
        CallGraphOptions {
            include_unattributed,
        },
    )
    .map_err(scan_error)?;
    let json = serde_json::to_string_pretty(&artifact)
        .map_err(|error| CliError::Internal(format!("序列化产物失败: {error}")))?;
    emit_json(&json, out)
}

fn parse_callgraph_args(
    mut args: impl Iterator<Item = String>,
) -> Result<(PathBuf, Option<PathBuf>, bool), CliError> {
    let target = args
        .next()
        .ok_or_else(|| CliError::Input(format!("callgraph 缺少目标仓库路径\n{USAGE}")))?;
    if target.starts_with("--") {
        return Err(CliError::Input(format!(
            "callgraph 缺少目标仓库路径\n{USAGE}"
        )));
    }

    let mut out: Option<PathBuf> = None;
    let mut include_unattributed = false;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--out" => {
                let value = args
                    .next()
                    .ok_or_else(|| CliError::Input("--out 缺少文件路径".to_string()))?;
                out = Some(PathBuf::from(value));
            }
            "--include-unattributed" => include_unattributed = true,
            other => {
                return Err(CliError::Input(format!("未知参数: {other}\n{USAGE}")));
            }
        }
    }

    Ok((PathBuf::from(target), out, include_unattributed))
}

fn run_history(args: impl Iterator<Item = String>) -> Result<(), CliError> {
    let (target, out, options) = parse_history_args(args)?;
    let artifact =
        build_history(&target, env!("CARGO_PKG_VERSION"), options).map_err(history_error)?;
    let json = serde_json::to_string_pretty(&artifact)
        .map_err(|error| CliError::Internal(format!("序列化产物失败: {error}")))?;
    emit_json(&json, out)
}

fn parse_history_args(
    mut args: impl Iterator<Item = String>,
) -> Result<(PathBuf, Option<PathBuf>, HistoryOptions), CliError> {
    let usage_missing = || CliError::Input(format!("history 缺少目标仓库路径\n{USAGE}"));

    let mut target: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut options = HistoryOptions::default();

    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--repo" => {
                let value = args
                    .next()
                    .ok_or_else(|| CliError::Input("--repo 缺少目标仓库路径".to_string()))?;
                target = Some(PathBuf::from(value));
            }
            "--out" => {
                let value = args
                    .next()
                    .ok_or_else(|| CliError::Input("--out 缺少文件路径".to_string()))?;
                out = Some(PathBuf::from(value));
            }
            "--max-commits" => {
                let value = args
                    .next()
                    .ok_or_else(|| CliError::Input("--max-commits 缺少数值".to_string()))?;
                let parsed = value.parse::<usize>().map_err(|_| {
                    CliError::Input(format!("--max-commits 需要正整数，实际: {value}"))
                })?;
                if parsed == 0 {
                    return Err(CliError::Input("--max-commits 必须大于 0".to_string()));
                }
                options.max_commits = parsed;
            }
            "--since" => {
                let value = args
                    .next()
                    .ok_or_else(|| CliError::Input("--since 缺少日期".to_string()))?;
                options.since = Some(value);
            }
            "--until" => {
                let value = args
                    .next()
                    .ok_or_else(|| CliError::Input("--until 缺少日期".to_string()))?;
                options.until = Some(value);
            }
            other if other.starts_with("--") => {
                return Err(CliError::Input(format!("未知参数: {other}\n{USAGE}")));
            }
            positional => {
                if target.is_some() {
                    return Err(CliError::Input(format!(
                        "history 只接受一个目标仓库路径，多余参数: {positional}"
                    )));
                }
                target = Some(PathBuf::from(positional));
            }
        }
    }

    let target = target.ok_or_else(usage_missing)?;
    Ok((target, out, options))
}

fn history_error(error: HistoryError) -> CliError {
    match error {
        HistoryError::NotDirectory(message) => CliError::Input(message),
        HistoryError::Io(message) => CliError::Internal(message),
        other => CliError::History(other.to_string()),
    }
}

struct PathCliOptions {
    target: PathBuf,
    out: Option<PathBuf>,
    entries: bool,
    narrative: bool,
    from: Option<String>,
    to: Option<String>,
}

fn run_path(args: impl Iterator<Item = String>) -> Result<(), CliError> {
    let options = parse_path_args(args)?;
    let version = env!("CARGO_PKG_VERSION");

    let json = if options.entries {
        let artifact = list_entries(&options.target, version).map_err(path_error)?;
        serde_json::to_string_pretty(&artifact)
            .map_err(|error| CliError::Internal(format!("序列化产物失败: {error}")))?
    } else {
        let from = options.from.as_deref().unwrap_or_default();
        let to = options.to.as_deref().unwrap_or_default();
        let artifact = build_path(&options.target, version, from, to).map_err(path_error)?;
        if options.narrative {
            return emit_text(&render_narrative(&artifact), options.out);
        }
        serde_json::to_string_pretty(&artifact)
            .map_err(|error| CliError::Internal(format!("序列化产物失败: {error}")))?
    };

    emit_json(&json, options.out)
}

fn parse_path_args(mut args: impl Iterator<Item = String>) -> Result<PathCliOptions, CliError> {
    let usage_missing = || CliError::Input(format!("path 缺少目标仓库路径\n{USAGE}"));

    let mut target: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut entries = false;
    let mut narrative = false;
    let mut from: Option<String> = None;
    let mut to: Option<String> = None;

    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--repo" => {
                let value = args
                    .next()
                    .ok_or_else(|| CliError::Input("--repo 缺少目标仓库路径".to_string()))?;
                if target.is_some() {
                    return Err(CliError::Input("path 只接受一个目标仓库路径".to_string()));
                }
                target = Some(PathBuf::from(value));
            }
            "--out" => {
                let value = args
                    .next()
                    .ok_or_else(|| CliError::Input("--out 缺少文件路径".to_string()))?;
                out = Some(PathBuf::from(value));
            }
            "--entries" => entries = true,
            "--narrative" => narrative = true,
            "--from" => {
                let value = args
                    .next()
                    .ok_or_else(|| CliError::Input("--from 缺少入口 ID".to_string()))?;
                from = Some(value);
            }
            "--to" => {
                let value = args
                    .next()
                    .ok_or_else(|| CliError::Input("--to 缺少符号 ID".to_string()))?;
                to = Some(value);
            }
            other if other.starts_with("--") => {
                return Err(CliError::Input(format!("未知参数: {other}\n{USAGE}")));
            }
            positional => {
                if target.is_some() {
                    return Err(CliError::Input(format!(
                        "path 只接受一个目标仓库路径，多余参数: {positional}"
                    )));
                }
                target = Some(PathBuf::from(positional));
            }
        }
    }

    let target = target.ok_or_else(usage_missing)?;
    if entries && narrative {
        return Err(CliError::Input(
            "--entries 不能与 --narrative 同时使用".to_string(),
        ));
    }
    if entries && (from.is_some() || to.is_some()) {
        return Err(CliError::Input(
            "--entries 不能与 --from/--to 同时使用".to_string(),
        ));
    }
    if !entries && (from.is_none() || to.is_none()) {
        return Err(CliError::Input(format!(
            "path 需要 --entries，或同时给出 --from 与 --to\n{USAGE}"
        )));
    }

    Ok(PathCliOptions {
        target,
        out,
        entries,
        narrative,
        from,
        to,
    })
}

fn path_error(error: PathError) -> CliError {
    match error {
        PathError::Scan(scan) => scan_error(scan),
        PathError::NoEntries(message) => {
            CliError::History(format!("目标仓库没有可识别的入口候选: {message}"))
        }
        PathError::UnknownSymbol(message) => CliError::Input(format!("未知符号: {message}")),
        PathError::Unreachable { from, to } => {
            CliError::Unreachable(format!("路径不可达: {from} -> {to}"))
        }
        PathError::Io(message) => CliError::Internal(message),
    }
}

struct DriftCliOptions {
    target: PathBuf,
    base: Option<PathBuf>,
    record: bool,
    out: Option<PathBuf>,
}

fn run_drift(args: impl Iterator<Item = String>) -> Result<(), CliError> {
    let options = parse_drift_args(args)?;
    let version = env!("CARGO_PKG_VERSION");

    if options.record {
        let layers = build_layers(&options.target, version).map_err(scan_error)?;
        let baseline = record_baseline(&layers, version);
        let json = serde_json::to_string_pretty(&baseline)
            .map_err(|error| CliError::Internal(format!("序列化基线失败: {error}")))?;
        return emit_json(&json, options.out);
    }

    let base_path = options
        .base
        .expect("parse_drift_args requires --base when --record is absent");
    let text = fs::read_to_string(&base_path).map_err(|error| {
        CliError::Input(format!("读取基线失败 {}: {error}", base_path.display()))
    })?;
    let baseline: DriftBaseline = serde_json::from_str(&text).map_err(|error| {
        CliError::Input(format!(
            "基线 JSON 解析失败 {}: {error}",
            base_path.display()
        ))
    })?;
    if baseline.schema_version != DRIFT_SCHEMA_VERSION {
        return Err(CliError::Input(format!(
            "基线 schemaVersion 不匹配: 期望 {DRIFT_SCHEMA_VERSION}，实际 {}",
            baseline.schema_version
        )));
    }

    let artifact = build_drift(&options.target, version, &baseline).map_err(scan_error)?;
    let json = serde_json::to_string_pretty(&artifact)
        .map_err(|error| CliError::Internal(format!("序列化报告失败: {error}")))?;
    emit_json(&json, options.out)?;

    if !artifact.gate.passed {
        return Err(CliError::DriftDetected(format!(
            "漂移门禁未通过: {} 条新增违规，详见报告 newViolations",
            artifact.new_violations.len()
        )));
    }

    Ok(())
}

fn parse_drift_args(mut args: impl Iterator<Item = String>) -> Result<DriftCliOptions, CliError> {
    let usage_missing = || CliError::Input(format!("drift 缺少目标仓库路径\n{USAGE}"));

    let mut target: Option<PathBuf> = None;
    let mut base: Option<PathBuf> = None;
    let mut record = false;
    let mut out: Option<PathBuf> = None;

    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--repo" => {
                let value = args
                    .next()
                    .ok_or_else(|| CliError::Input("--repo 缺少目标仓库路径".to_string()))?;
                if target.is_some() {
                    return Err(CliError::Input("drift 只接受一个目标仓库路径".to_string()));
                }
                target = Some(PathBuf::from(value));
            }
            "--base" => {
                let value = args
                    .next()
                    .ok_or_else(|| CliError::Input("--base 缺少基线文件路径".to_string()))?;
                base = Some(PathBuf::from(value));
            }
            "--record" => record = true,
            "--out" => {
                let value = args
                    .next()
                    .ok_or_else(|| CliError::Input("--out 缺少文件路径".to_string()))?;
                out = Some(PathBuf::from(value));
            }
            other if other.starts_with("--") => {
                return Err(CliError::Input(format!("未知参数: {other}\n{USAGE}")));
            }
            positional => {
                if target.is_some() {
                    return Err(CliError::Input(format!(
                        "drift 只接受一个目标仓库路径，多余参数: {positional}"
                    )));
                }
                target = Some(PathBuf::from(positional));
            }
        }
    }

    let target = target.ok_or_else(usage_missing)?;
    if record {
        if base.is_some() {
            return Err(CliError::Input(
                "--record 不能与 --base 同时使用".to_string(),
            ));
        }
    } else if base.is_none() {
        return Err(CliError::Input(format!(
            "drift 需要 --record 或 --base <基线文件>\n{USAGE}"
        )));
    }

    Ok(DriftCliOptions {
        target,
        base,
        record,
        out,
    })
}

fn parse_repo_and_out(
    command: &str,
    mut args: impl Iterator<Item = String>,
) -> Result<(PathBuf, Option<PathBuf>), CliError> {
    let target = args
        .next()
        .ok_or_else(|| CliError::Input(format!("{command} 缺少目标仓库路径\n{USAGE}")))?;
    if target.starts_with("--") {
        return Err(CliError::Input(format!(
            "{command} 缺少目标仓库路径\n{USAGE}"
        )));
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

    Ok((PathBuf::from(target), out))
}

fn scan_error(error: ScanError) -> CliError {
    match error {
        ScanError::TooManyFiles { .. } | ScanError::NotADirectory(_) => {
            CliError::Input(error.to_string())
        }
        ScanError::Io(_) => CliError::Internal(error.to_string()),
    }
}

fn emit_json(json: &str, out: Option<PathBuf>) -> Result<(), CliError> {
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

fn emit_text(text: &str, out: Option<PathBuf>) -> Result<(), CliError> {
    if let Some(path) = out {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).map_err(|error| {
                    CliError::Internal(format!("创建输出目录失败 {}: {error}", parent.display()))
                })?;
            }
        }
        fs::write(&path, text).map_err(|error| {
            CliError::Internal(format!("写入产物失败 {}: {error}", path.display()))
        })?;
    } else {
        print!("{text}");
    }
    Ok(())
}
