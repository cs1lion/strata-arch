# ticket 11：Windows 手动 OS 窗口捕获（人工证据，不进 CI 门禁）。
#
# 背景：gpui 0.2.2 没有公开离屏导出入口，结构化 IR 快照只能证明布局模型；
# 真实窗口外观需要一次人工捕获作为证据。mac/Linux 不做截图。
#
# 用法：
#   cargo build --release -p arch-view
#   pwsh -File eval/capture-window.ps1
#   pwsh -File eval/capture-window.ps1 -Repo tests/fixtures/layers -OutPng docs/evidence/11-window-capture.png
#
# 做法：启动 `arch-view map <repo>`，等待主窗口句柄，先用 PrintWindow（PW_RENDERFULLCONTENT）
# 尝试离屏抓取；若返回失败则回退到屏幕 CopyFromScreen。捕获后关闭进程。
# 本脚本只作人工证据，不参与 CI；失败时退出 1 并说明原因。

[CmdletBinding()]
param(
    [string]$Repo = "tests/fixtures/layers",
    [string]$OutPng = "docs/evidence/11-window-capture.png",
    [string]$Exe = "target/release/arch-view.exe",
    [int]$WarmupMs = 1500,
    [int]$TimeoutSeconds = 20
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

if (-not (Test-Path $Exe)) {
    throw "缺少 $Exe；先运行 cargo build --release -p arch-view"
}
if (-not (Test-Path $Repo)) {
    throw "缺少目标仓库 $Repo"
}

$outDir = Split-Path -Parent $OutPng
if ($outDir -and -not (Test-Path $outDir)) {
    New-Item -ItemType Directory -Path $outDir | Out-Null
}
$absolutePng = Join-Path (Get-Location) $OutPng

Add-Type -AssemblyName System.Drawing

$signature = @"
using System;
using System.Runtime.InteropServices;

public struct NativeRect { public int Left; public int Top; public int Right; public int Bottom; }

public static class CaptureWin32 {
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hwnd, IntPtr hdc, uint flags);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd, out NativeRect rect);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hwnd, int command);
}
"@
Add-Type -TypeDefinition $signature

$proc = Start-Process -FilePath $Exe -ArgumentList @("map", $Repo) -PassThru
try {
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ((Get-Date) -lt $deadline -and $proc.MainWindowHandle -eq 0) {
        Start-Sleep -Milliseconds 100
        $proc.Refresh()
    }
    if ($proc.MainWindowHandle -eq 0) {
        throw "在 $TimeoutSeconds 秒内没有拿到 arch-view 主窗口句柄"
    }

    Start-Sleep -Milliseconds $WarmupMs
    [CaptureWin32]::ShowWindow($proc.MainWindowHandle, 5) | Out-Null
    [CaptureWin32]::SetForegroundWindow($proc.MainWindowHandle) | Out-Null
    Start-Sleep -Milliseconds 300

    $rect = New-Object NativeRect
    if (-not [CaptureWin32]::GetWindowRect($proc.MainWindowHandle, [ref]$rect)) {
        throw "GetWindowRect 失败"
    }
    $width = $rect.Right - $rect.Left
    $height = $rect.Bottom - $rect.Top
    if ($width -le 0 -or $height -le 0) {
        throw "窗口尺寸无效: ${width}x${height}"
    }

    $bitmap = New-Object System.Drawing.Bitmap $width, $height
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $hdc = $graphics.GetHdc()
    $printed = [CaptureWin32]::PrintWindow($proc.MainWindowHandle, $hdc, 2)
    $graphics.ReleaseHdc($hdc)
    $graphics.Dispose()

    if (-not $printed) {
        Write-Output "PrintWindow 失败，回退到屏幕捕获"
        $bitmap.Dispose()
        $bitmap = New-Object System.Drawing.Bitmap $width, $height
        $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
        $graphics.CopyFromScreen($rect.Left, $rect.Top, 0, 0, (New-Object System.Drawing.Size $width, $height))
        $graphics.Dispose()
    }

    $bitmap.Save($absolutePng, [System.Drawing.Imaging.ImageFormat]::Png)
    $bitmap.Dispose()

    Write-Output "captured ${width}x${height} -> $absolutePng"
    Get-FileHash -Algorithm SHA256 -Path $absolutePng | ForEach-Object {
        Write-Output ("sha256 {0}" -f $_.Hash)
    }
} finally {
    if (-not $proc.HasExited) {
        $proc.CloseMainWindow() | Out-Null
        Start-Sleep -Milliseconds 500
        if (-not $proc.HasExited) { $proc.Kill() }
    }
}
