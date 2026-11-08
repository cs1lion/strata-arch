# A/D 视觉等价证据：同窗口尺寸抓静态最终态，按票 10 先验容差比较像素。
#
# 容差（票 10「A/D 视觉等价容差」，测量前固定）：
#   A      base vs A         changed% <= 0.50%，单像素最大通道差 <= 24
#   D1     A vs AD1          changed% <= 0.01%，单像素最大通道差 <= 2
#   D2     AD1 vs ADclip     changed% <= 0.50%，单像素最大通道差 <= 24（超出即判可选、默认关）
# 口径：通道差 > 6 记为变化像素；最大通道差取所有像素三通道差的最大值。
#
# 用法：
#   cargo build --release -p arch-view
#   pwsh -File eval/map-pixel-compare.ps1
#   pwsh -File eval/map-pixel-compare.ps1 -Repo tests/fixtures/layers -OutDir out/map-pixel-layers
#
# 依赖可见桌面会话（Windows）；产物在 out/ 下，已被 .gitignore 排除。

[CmdletBinding()]
param(
    [string]$Repo = "tests/corpus/excalidraw",
    [string]$OutDir = "out/map-pixel",
    [string]$Exe = "target/release/arch-view.exe",
    [int]$ChangeTolerance = 6,
    [double]$EdgeMaxChangedRatio = 0.005,
    [int]$EdgeMaxChannelDelta = 24,
    [double]$DietMaxChangedRatio = 0.0001,
    [int]$DietMaxChannelDelta = 2,
    [int]$WarmupMs = 900,
    [int]$AutoCloseMs = 9000,
    # 复用 OutDir 里已有的四张 PNG，只重算像素比较（调试用；判据不变）。
    [switch]$SkipCapture,
    # 诊断：四个配置全部用 base 参数重复抓帧，用于量「跨进程重绘噪声」。
    [switch]$RepeatBase
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
$capture = Join-Path $root "eval/capture-window.ps1"
if (-not (Test-Path $Exe)) { throw "缺少 $Exe；先运行 cargo build --release -p arch-view" }
if (-not (Test-Path $Repo)) { throw "缺少目标仓库 $Repo" }
if (-not (Test-Path $OutDir)) { New-Item -ItemType Directory -Force -Path $OutDir | Out-Null }

Add-Type -AssemblyName System.Drawing

if (-not ("PixelDiff" -as [type])) {
    Add-Type -TypeDefinition @'
using System;

public class PixelDiffResult {
    public int Width;
    public int Height;
    public long Total;
    public long Changed;
    public int MaxChannelDelta;
    public double ChangedRatio;
    public int OverLimit;
    public string Sample = "";
    public int MinX = -1;
    public int MinY = -1;
    public int MaxX = -1;
    public int MaxY = -1;
}

public static class PixelDiff {
    public static PixelDiffResult Compare(byte[] a, int aWidth, int aHeight, int aStride,
            byte[] b, int bWidth, int bHeight, int bStride, int tolerance, int deltaLimit) {
        var result = new PixelDiffResult { Width = aWidth, Height = aHeight };
        if (aWidth != bWidth || aHeight != bHeight) {
            result.Total = (long)aWidth * aHeight;
            result.Changed = result.Total;
            result.ChangedRatio = 1.0;
            result.MaxChannelDelta = 255;
            return result;
        }
        long changed = 0;
        int maxDelta = 0;
        for (int y = 0; y < aHeight; y++) {
            int li = y * aStride;
            int ri = y * bStride;
            for (int x = 0; x < aWidth; x++) {
                int lb = li + x * 4;
                int rb = ri + x * 4;
                int db = Math.Abs(a[lb] - b[rb]);
                int dg = Math.Abs(a[lb + 1] - b[rb + 1]);
                int dr = Math.Abs(a[lb + 2] - b[rb + 2]);
                int delta = Math.Max(db, Math.Max(dg, dr));
                if (delta > maxDelta) maxDelta = delta;
                if (db > tolerance || dg > tolerance || dr > tolerance) {
                    changed++;
                    if (delta > deltaLimit) {
                        result.OverLimit++;
                        if (result.OverLimit <= 6) {
                            result.Sample += String.Format("{0},{1}:({2},{3},{4})vs({5},{6},{7}) ",
                                x, y, a[lb + 2], a[lb + 1], a[lb], b[rb + 2], b[rb + 1], b[rb]);
                        }
                    }
                    if (result.MinX < 0 || x < result.MinX) result.MinX = x;
                    if (result.MaxX < 0 || x > result.MaxX) result.MaxX = x;
                    if (result.MinY < 0 || y < result.MinY) result.MinY = y;
                    if (result.MaxY < 0 || y > result.MaxY) result.MaxY = y;
                }
            }
        }
        result.Total = (long)aWidth * aHeight;
        result.Changed = changed;
        result.ChangedRatio = result.Total == 0 ? 0.0 : (double)changed / result.Total;
        result.MaxChannelDelta = maxDelta;
        return result;
    }
}
'@
}

function Load-Pixels {
    param([string]$Path)
    if (-not (Test-Path $Path)) { throw "缺少帧文件 $Path" }
    $bmp = [System.Drawing.Bitmap]::FromFile((Resolve-Path $Path).Path)
    $width = $bmp.Width
    $height = $bmp.Height
    $rect = New-Object System.Drawing.Rectangle 0, 0, $width, $height
    $locked = $bmp.LockBits($rect, [System.Drawing.Imaging.ImageLockMode]::ReadOnly, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $bytes = New-Object byte[] ($locked.Stride * $height)
    [System.Runtime.InteropServices.Marshal]::Copy($locked.Scan0, $bytes, 0, $bytes.Length)
    $stride = $locked.Stride
    $bmp.UnlockBits($locked)
    $bmp.Dispose()
    return [pscustomobject]@{ Bytes = $bytes; Width = $width; Height = $height; Stride = $stride }
}

$configs = @(
    @{ Name = "base"; Env = @{ STRATA_EDGE_BATCH = "0"; STRATA_NODE_OPACITY_SKIP = "0"; STRATA_NODE_CLIP = "1" } },
    @{ Name = "A"; Env = @{ STRATA_EDGE_BATCH = "1"; STRATA_NODE_OPACITY_SKIP = "0"; STRATA_NODE_CLIP = "1" } },
    @{ Name = "AD1"; Env = @{ STRATA_EDGE_BATCH = "1"; STRATA_NODE_OPACITY_SKIP = "1"; STRATA_NODE_CLIP = "1" } },
    @{ Name = "ADclip"; Env = @{ STRATA_EDGE_BATCH = "1"; STRATA_NODE_OPACITY_SKIP = "1"; STRATA_NODE_CLIP = "0" } }
)

$captured = @{}
if ($RepeatBase) {
    foreach ($config in $configs) {
        $config.Env = @{ STRATA_EDGE_BATCH = "0"; STRATA_NODE_OPACITY_SKIP = "0"; STRATA_NODE_CLIP = "1" }
    }
}
if ($SkipCapture) {
    foreach ($config in $configs) {
        $captured[$config.Name] = Join-Path $OutDir ("{0}.png" -f $config.Name)
    }
} else {
# 需要阶段打点才能等到「内容已呈现」再抓帧；打点本身不影响绘制。
Set-Item -Path "env:STRATA_PHASE_MARKERS" -Value "1"
foreach ($config in $configs) {
    foreach ($key in $config.Env.Keys) { Set-Item -Path "env:$key" -Value $config.Env[$key] }
    $png = Join-Path $OutDir ("{0}.png" -f $config.Name)
    $envText = (($config.Env.GetEnumerator() | ForEach-Object { "$($_.Key)=$($_.Value)" }) -join " ")
    Write-Output ("capture {0}: {1}" -f $config.Name, $envText)
    # 直接在同进程调用：`pwsh -File` 会把数组参数展开成多个命令行 token，
    # `--auto-close-ms` 会被当成 capture-window 的参数名而报错。
    & $capture -AppArgs @("map", $Repo, "--auto-close-ms", "$AutoCloseMs") `
        -Marker "phase: content:presented" -WarmupMs $WarmupMs -Frames 1 -SettleMs 200 `
        -Method print -OutPng $png -StdoutFile (Join-Path $OutDir ("{0}.stdout.txt" -f $config.Name)) | Out-Null
    $captured[$config.Name] = $png
    foreach ($key in $config.Env.Keys) { Remove-Item -Path "env:$key" -ErrorAction SilentlyContinue }
}
}

$comparisons = @(
    @{ Name = "A"; Left = "base"; Right = "A"; MaxRatio = $EdgeMaxChangedRatio; MaxDelta = $EdgeMaxChannelDelta },
    @{ Name = "D1"; Left = "A"; Right = "AD1"; MaxRatio = $DietMaxChangedRatio; MaxDelta = $DietMaxChannelDelta },
    @{ Name = "D2"; Left = "AD1"; Right = "ADclip"; MaxRatio = $EdgeMaxChangedRatio; MaxDelta = $EdgeMaxChannelDelta }
)

$rows = @()
foreach ($comparison in $comparisons) {
    $left = $captured[$comparison.Left]
    $right = $captured[$comparison.Right]
    if (-not (Test-Path $left) -or -not (Test-Path $right)) { throw "缺少抓帧：$left / $right" }
    $leftImage = Load-Pixels $left
    $rightImage = Load-Pixels $right
    $diff = [PixelDiff]::Compare(
        $leftImage.Bytes, $leftImage.Width, $leftImage.Height, $leftImage.Stride,
        $rightImage.Bytes, $rightImage.Width, $rightImage.Height, $rightImage.Stride,
        $ChangeTolerance, $comparison.MaxDelta)
    $passed = ($diff.ChangedRatio -le $comparison.MaxRatio) -and ($diff.MaxChannelDelta -le $comparison.MaxDelta)
    $rows += [pscustomobject]@{
        Pair = "$($comparison.Left) vs $($comparison.Right)"
        ChangedPixels = $diff.Changed
        TotalPixels = $diff.Total
        ChangedPercent = [math]::Round($diff.ChangedRatio * 100, 4)
        MaxChannelDelta = $diff.MaxChannelDelta
        OverLimitPixels = $diff.OverLimit
        Bounds = if ($diff.MinX -ge 0) { "{0},{1}-{2},{3}" -f $diff.MinX, $diff.MinY, $diff.MaxX, $diff.MaxY } else { "-" }
        Sample = $diff.Sample
        AllowedPercent = [math]::Round($comparison.MaxRatio * 100, 4)
        AllowedDelta = $comparison.MaxDelta
        Status = if ($passed) { "PASS" } else { "FAIL" }
    }
}

$failed = @($rows | Where-Object { $_.Status -ne "PASS" })
$payload = [pscustomobject]@{
    repo = $Repo
    changeTolerance = $ChangeTolerance
    comparisons = $rows
    passed = ($failed.Count -eq 0)
}
$payload | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $OutDir "report.json") -Encoding utf8

Write-Output ""
Write-Output ("{0,-16} {1,-8} {2,9} {3,10} {4,10} {5,10}  {6}" -f "pair", "changed%", "maxDelta", "overLimit", "allowed%", "allowedδ", "status")
foreach ($row in $rows) {
    Write-Output ("{0,-16} {1,8}% {2,9} {3,10} {4,9}% {5,9}  {6}  bbox={7}" -f `
            $row.Pair, $row.ChangedPercent, $row.MaxChannelDelta, $row.OverLimitPixels, `
            $row.AllowedPercent, $row.AllowedDelta, $row.Status, $row.Bounds)
}
Write-Output ""
Write-Output ("pixels: {0}" -f (Join-Path $OutDir "report.json"))
if ($failed.Count -gt 0) {
    Write-Output ("map pixel compare: FAILED（{0} 对超容差：{1}）" -f $failed.Count, (($failed | ForEach-Object { $_.Pair }) -join ", "))
    exit 1
}
Write-Output "map pixel compare: PASSED"
exit 0
