# ticket 11：Windows 手动 OS 窗口捕获（人工证据，不进 CI 门禁）。
#
# 背景：gpui 0.2.2 没有公开离屏导出入口，结构化 IR 快照只能证明布局模型；
# 真实窗口外观需要一次人工捕获作为证据。mac/Linux 不做截图。
#
# 用法（保持 ticket 11 的默认单帧入口）：
#   cargo build --release -p arch-view
#   pwsh -File eval/capture-window.ps1
#   pwsh -File eval/capture-window.ps1 -Repo tests/fixtures/layers -OutPng docs/evidence/11-window-capture.png
#
# 多帧 / 任意子命令（ticket 11 像素证据链使用；不进 CI）：
#   # 加载态：等 stdout 出现第一帧呈现标记后立刻连拍 3 帧
#   $env:STRATA_PHASE_MARKERS = '1'
#   & ./eval/capture-window.ps1 `
#     -AppArgs @('timeline','.','--auto-close-ms','6000') `
#     -Marker 'phase: frame:presented' -WarmupMs 0 `
#     -Frames 3 -IntervalMs 120 `
#     -OutPng out/pixel-evidence/loading.png -StdoutFile out/pixel-evidence/loading.stdout.txt
#
#   # 播放态：等刻度切换标记后连拍，相邻帧用于证明时间线在推进
#   & ./eval/capture-window.ps1 `
#     -AppArgs @('timeline','tests/fixtures/history','--autoplay','--auto-close-ms','6000') `
#     -Marker 'playback: frame tick 2' -WarmupMs 80 `
#     -Frames 3 -IntervalMs 400 `
#     -OutPng out/pixel-evidence/playback.png
#
# 做法：用 .NET Process 启动 `arch-view`（默认 `map <Repo>`，或用 -AppArgs 指定任意子命令），
# 后台线程持续收集 stdout；等待主窗口句柄与可选 stdout 标记，先用 PrintWindow
# （PW_RENDERFULLCONTENT）尝试离屏抓取；若返回失败则回退到屏幕 CopyFromScreen。
# 捕获后关闭进程。本脚本只作人工证据，不参与 CI；失败时退出 1 并说明原因。
#
# 注意：不要用 PowerShell `Start-Process -RedirectStandardOutput` 启动 GPUI 应用，
# 实测该方式下窗口永远不会创建（停在 app:entry）；本脚本改用 .NET Process。

[CmdletBinding()]
param(
    [string]$Repo = "tests/fixtures/layers",
    [string]$OutPng = "docs/evidence/11-window-capture.png",
    [string]$Exe = "target/release/arch-view.exe",
    # 为空时沿用旧行为：map <Repo>；否则完整指定子命令与参数。
    [string[]]$AppArgs = @(),
    [int]$WarmupMs = 1500,
    [int]$TimeoutSeconds = 20,
    # 单帧时直接写 OutPng；多帧时写 <name>-f01.png、<name>-f02.png ...
    [int]$Frames = 1,
    [int]$IntervalMs = 0,
    # 可选：等待 arch-view stdout 中出现该子串后再开始抓取（配合 STRATA_PHASE_MARKERS 使用）。
    [string]$Marker = "",
    [int]$MarkerTimeoutSeconds = 20,
    # 抓取方式：auto = 先 PrintWindow，失败回退屏幕；print = 只用 PrintWindow；screen = 只用屏幕。
    [ValidateSet("auto", "print", "screen")]
    [string]$Method = "auto",
    # 置前台后等待窗口稳定的时间（毫秒）；加载态需要更小值以抢在就绪前抓到。
    [int]$SettleMs = 300,
    # 可选：把子进程 stdout 落盘；用于核对「抓取时仍未就绪」等状态。
    [string]$StdoutFile = "",
    # 置位时，在等待标记/句柄成功、开始抓帧之前冻结 stdout；
    # 否则在抓帧结束后写（默认行为）。加载态需要前者，避免「抓帧时 loading、
    # 写盘时已 load:done」的时序歧义。
    [switch]$StdoutAtMarker
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

if ($Frames -lt 1) { throw "-Frames 必须 >= 1" }
if (-not (Test-Path $Exe)) {
    throw "缺少 $Exe；先运行 cargo build --release -p arch-view"
}
if ($AppArgs.Count -eq 0) {
    $AppArgs = @("map", $Repo)
    if (-not (Test-Path $Repo)) {
        throw "缺少目标仓库 $Repo"
    }
}

$outDir = Split-Path -Parent $OutPng
if ($outDir -and -not (Test-Path $outDir)) {
    New-Item -ItemType Directory -Path $outDir | Out-Null
}
$absolutePng = Join-Path (Get-Location) $OutPng
$outDirectory = Split-Path -Parent $absolutePng
$outFileName = [System.IO.Path]::GetFileNameWithoutExtension($absolutePng)
$outExtension = [System.IO.Path]::GetExtension($absolutePng)

Add-Type -AssemblyName System.Drawing

if (-not ("AppRunner" -as [type])) {
    Add-Type -TypeDefinition @"
using System;
using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

public struct NativeRect { public int Left; public int Top; public int Right; public int Bottom; }

public static class CaptureWin32 {
    public delegate bool EnumWindowsProc(IntPtr hwnd, IntPtr lParam);

    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hwnd, IntPtr hdc, uint flags);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd, out NativeRect rect);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hwnd, int command);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern uint GetDpiForSystem();
    [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr value);
    [DllImport("user32.dll")] private static extern bool EnumWindows(EnumWindowsProc callback, IntPtr lParam);
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint processId);

    /// Process.MainWindowHandle 在重定向 stdout 的 GPUI 进程上会返回 0；
    /// 这里按进程 id 枚举可见顶层窗口，取第一个有实际尺寸的句柄。
    public static IntPtr FindWindowForProcess(int processId) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((hwnd, lParam) => {
            uint owner;
            GetWindowThreadProcessId(hwnd, out owner);
            if ((int)owner != processId) return true;
            NativeRect rect;
            if (!GetWindowRect(hwnd, out rect)) return true;
            if (rect.Right - rect.Left < 200 || rect.Bottom - rect.Top < 200) return true;
            found = hwnd;
            return false;
        }, IntPtr.Zero);
        return found;
    }

    public static string DebugProcessWindows(int processId) {
        var report = new StringBuilder();
        EnumWindows((hwnd, lParam) => {
            uint owner;
            GetWindowThreadProcessId(hwnd, out owner);
            if ((int)owner != processId) return true;
            NativeRect rect;
            bool ok = GetWindowRect(hwnd, out rect);
            report.AppendLine("hwnd=" + hwnd + " ok=" + ok + " size=" +
                (ok ? (rect.Right - rect.Left) + "x" + (rect.Bottom - rect.Top) : "?"));
            return true;
        }, IntPtr.Zero);
        return report.ToString();
    }
}

public class AppRunner : IDisposable {
    private Process process;
    private readonly object gate = new object();
    private readonly StringBuilder stdout = new StringBuilder();
    private readonly StringBuilder stderr = new StringBuilder();

    public void Start(string exe, string[] args) {
        var psi = new ProcessStartInfo {
            FileName = exe,
            UseShellExecute = false,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            CreateNoWindow = false
        };
        foreach (var argument in args) psi.ArgumentList.Add(argument);
        process = new Process { StartInfo = psi };
        process.Start();
        StartPump(process.StandardOutput, stdout);
        StartPump(process.StandardError, stderr);
    }

    private void StartPump(StreamReader reader, StringBuilder sink) {
        var thread = new Thread(() => {
            try {
                string line;
                while ((line = reader.ReadLine()) != null) {
                    lock (gate) sink.AppendLine(line);
                }
            } catch { }
        });
        thread.IsBackground = true;
        thread.Start();
    }

    public string Stdout { get { lock (gate) { return stdout.ToString(); } } }
    public string Stderr { get { lock (gate) { return stderr.ToString(); } } }
    public int Id { get { return process.Id; } }
    public IntPtr Handle { get { process.Refresh(); return process.MainWindowHandle; } }
    public bool HasExited { get { process.Refresh(); return process.HasExited; } }
    public int ExitCode { get { process.Refresh(); return process.HasExited ? process.ExitCode : -1; } }

    public void Close() {
        if (process == null) return;
        try {
            if (!process.HasExited) {
                process.CloseMainWindow();
                Thread.Sleep(500);
                if (!process.HasExited) process.Kill(true);
            }
        } catch { }
    }

    public void Dispose() {
        Close();
        if (process != null) process.Dispose();
    }
}
"@
}

$exePath = (Resolve-Path $Exe).Path
# 让当前 PowerShell 进入 per-monitor v2 DPI 感知：否则 GetWindowRect 返回虚拟化尺寸，
# 而 PrintWindow 按物理像素渲染，抓出来的图会被右下裁掉（播放面板/刻度条消失）。
[CaptureWin32]::SetProcessDpiAwarenessContext([IntPtr]-4) | Out-Null
Write-Output ("dpi: system={0} scale={1}" -f [CaptureWin32]::GetDpiForSystem(), ([CaptureWin32]::GetDpiForSystem() / 96.0))
$runner = New-Object AppRunner
$runner.Start($exePath, [string[]]$AppArgs)
try {
    # 等待放在 PowerShell 侧：实测在 C# 方法里连续 Refresh + 轮询会让 GPUI 子进程停在 app:created。
    $hwnd = [IntPtr]::Zero
    $deadline = (Get-Date).AddSeconds([Math]::Max($TimeoutSeconds, $MarkerTimeoutSeconds))
    $lastHandle = [IntPtr]::Zero
    $markerSeen = ($Marker -eq "")
    while ((Get-Date) -lt $deadline) {
        if ($runner.HasExited) {
            $stderr = $runner.Stderr
            throw "arch-view 在窗口/标记就绪前退出（exit=$($runner.ExitCode)）：$stderr"
        }
        if (-not $markerSeen) {
            $markerSeen = $runner.Stdout.Contains($Marker)
        }
        $hwnd = $runner.Handle
        if ($hwnd -eq [IntPtr]::Zero) {
            $hwnd = [CaptureWin32]::FindWindowForProcess($runner.Id)
        }
        if ($hwnd -ne [IntPtr]::Zero) { $lastHandle = $hwnd }
        if ($hwnd -ne [IntPtr]::Zero -and $markerSeen) { break }
        Start-Sleep -Milliseconds 50
    }

    if ($hwnd -eq [IntPtr]::Zero -or -not $markerSeen) {
        $stdout = $runner.Stdout
        Write-Output "wait timeout: markerSeen=$markerSeen lastHandle=$lastHandle pid=$($runner.Id)"
        Write-Output "windows:"
        Write-Output ([CaptureWin32]::DebugProcessWindows($runner.Id))
        if ($stdout) { Write-Output $stdout }
        throw "等待超时：markerSeen=$markerSeen lastHandle=$lastHandle stdoutLen=$($stdout.Length)"
    }

    if ($StdoutAtMarker -and $StdoutFile -ne "") {
        $markerDirectory = Split-Path -Parent $StdoutFile
        if ($markerDirectory -and -not (Test-Path $markerDirectory)) {
            New-Item -ItemType Directory -Path $markerDirectory | Out-Null
        }
        [System.IO.File]::WriteAllText((Join-Path (Get-Location) $StdoutFile), $runner.Stdout)
        Write-Output "stdout(marker) -> $StdoutFile"
    }

    if ($WarmupMs -gt 0) {
        Start-Sleep -Milliseconds $WarmupMs
    }
    [CaptureWin32]::ShowWindow($hwnd, 5) | Out-Null
    [CaptureWin32]::SetForegroundWindow($hwnd) | Out-Null
    if ($SettleMs -gt 0) {
        Start-Sleep -Milliseconds $SettleMs
    }

    $rect = New-Object NativeRect
    if (-not [CaptureWin32]::GetWindowRect($hwnd, [ref]$rect)) {
        throw "GetWindowRect 失败"
    }
    $width = $rect.Right - $rect.Left
    $height = $rect.Bottom - $rect.Top
    if ($width -le 0 -or $height -le 0) {
        throw "窗口尺寸无效: ${width}x${height}"
    }

    for ($index = 1; $index -le $Frames; $index++) {
        if ($index -gt 1 -and $IntervalMs -gt 0) {
            Start-Sleep -Milliseconds $IntervalMs
        }

        if ($Frames -eq 1) {
            $framePath = $absolutePng
        } else {
            $framePath = Join-Path $outDirectory ("{0}-f{1:D2}{2}" -f $outFileName, $index, $outExtension)
        }

        $bitmap = New-Object System.Drawing.Bitmap $width, $height
        $printed = $false
        if ($Method -ne "screen") {
            $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
            $hdc = $graphics.GetHdc()
            $printed = [CaptureWin32]::PrintWindow($hwnd, $hdc, 2)
            $graphics.ReleaseHdc($hdc)
            $graphics.Dispose()
        }

        if (-not $printed) {
            if ($Method -eq "print") {
                $bitmap.Dispose()
                throw "PrintWindow 返回失败（Method=print）"
            }
            if ($Method -eq "screen") {
                Write-Output "使用屏幕捕获"
            } else {
                Write-Output "PrintWindow 失败，回退到屏幕捕获"
            }
            $bitmap.Dispose()
            $bitmap = New-Object System.Drawing.Bitmap $width, $height
            $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
            $graphics.CopyFromScreen($rect.Left, $rect.Top, 0, 0, (New-Object System.Drawing.Size $width, $height))
            $graphics.Dispose()
        }

        $bitmap.Save($framePath, [System.Drawing.Imaging.ImageFormat]::Png)
        $bitmap.Dispose()

        if ($Frames -eq 1) {
            Write-Output "captured ${width}x${height} -> $framePath"
        } else {
            Write-Output "captured frame $index/$Frames ${width}x${height} -> $framePath"
        }
        Get-FileHash -Algorithm SHA256 -Path $framePath | ForEach-Object {
            Write-Output ("sha256 {0}" -f $_.Hash)
        }
    }

    if ($StdoutFile -ne "" -and -not $StdoutAtMarker) {
        $stdoutDirectory = Split-Path -Parent $StdoutFile
        if ($stdoutDirectory -and -not (Test-Path $stdoutDirectory)) {
            New-Item -ItemType Directory -Path $stdoutDirectory | Out-Null
        }
        [System.IO.File]::WriteAllText((Join-Path (Get-Location) $StdoutFile), $runner.Stdout)
        Write-Output "stdout -> $StdoutFile"
    }
} finally {
    $runner.Dispose()
}
