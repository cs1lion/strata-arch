# ticket 11：本地像素证据链（人工证据，不进 CI）。
#
# 目的：把票 16/18/19 反复挂着的「只有模型断言、没有像素证据」补成一条可复跑的链路。
# 覆盖八个状态：加载态非白屏、失败态错误文案、diff 三色、P1 上下文淡出前后帧、P3 播放相邻帧，
# 以及 ticket 20 的模块搜索三态（有结果 / 无匹配 / 聚焦含标签）与清除后帧。
#
# 前置：
#   cargo build --release -p arch-view
#   # 需要可见桌面会话（Windows）。无头环境不可用，属于人工项，见 docs/acceptance-gaps.md。
#
# 运行：
#   pwsh -File eval/pixel-evidence.ps1
#
# 产物（全部在 out/pixel-evidence/，被 .gitignore 排除）：
#   loading-f01..f03.png / failure-f01..f03.png / diff-f01..f02.png /
#   p1-off-f01..f02.png / p1-on-f01..f03.png / p3-f01..f05.png / p3-static-f01..f02.png
#   *.stdout.txt（抓取时刻的 arch-view stdout 快照）
#   report.json（判据、阈值、实测读数、每条断言的通过/失败）
#
# 阈值纪律：先定依据再取值；本文常量旁的注释即依据。任何事后调整必须在
# docs/acceptance-gaps.md 登记 post-hoc 并写明旧/新读数与理由。

[CmdletBinding()]
param(
    [string]$Exe = "target/release/arch-view.exe",
    [string]$OutDir = "out/pixel-evidence",
    [int]$TimeoutSeconds = 30
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
$captureScript = Join-Path $root "eval/capture-window.ps1"

if (-not (Test-Path $Exe)) { throw "缺少 $Exe；先运行 cargo build --release -p arch-view" }
if (-not (Test-Path $captureScript)) { throw "缺少 eval/capture-window.ps1" }

# ---- 阈值与依据（先定依据，后取值）----------------------------------------
# 画布物理像素 2728x1879（DPI 216 / 2.25x，PrintWindow 渲染尺寸）。
$Thresholds = [ordered]@{
    # 有效帧：正常内容帧整帧平均亮度实测 24–30；空白帧（只有标题栏）约 8.5。
    # 取 15 分界：高于空白帧近一倍，低于正常帧最低值约 40%。白屏则接近 255。
    FrameMinMeanLuminance = 15.0
    FrameMaxMeanLuminance = 200.0

    # 加载态文案：深色主题下正文为 #f3f3f3（亮度 243）；中心区实测 10012 像素 / 3 行。
    # 2.25x 下 14px 字号三行，每行至少约 200 像素；取 600（约 3 行 × 200）为下限。
    LoadingTextMinPixels = 600
    LoadingTextMinBands = 3
    # 每行至少 30 像素才计入一个文字行带；14px 字号单字宽约 30 物理像素。
    LoadingTextRowMinPixels = 30

    # 失败态橙色文案：实测主色 #e07b39，中心区 17482 像素。
    # 三行 CJK 错误文案按 14px/2.25x 估算每行 ≥ 300 像素，三行 ≥ 900；取 2000 留约 2x 余量。
    FailureOrangeMinPixels = 2000
    FailureOrangeRgbMin = @(140, 60, 20)
    FailureOrangeRgbMax = @(255, 160, 100)

    # diff 三色（#2f6b3f / #7a3138 / #7a5b24），容差 ±8 覆盖 GPU 合成舍入。
    # 单行 tint 约 2400x54 ≈ 1.2e5 像素；取 2000（约单行 1.5%）证明是整行填充而非孤立抗锯齿像素。
    DiffTintMinPixels = 2000
    DiffTintTolerance = 8

    # P1：16 节点中 13 个非路径节点透明度 0.25→0.68；节点瓦片+标签约占画面 10%。
    # 取 2%（约 1/5 余量）；另要求 >=10 个 32x32 热点单元（13 个节点各至少覆盖 1 格）。
    P1MinChangedRatio = 0.02
    P1MinHotCells = 10
    HotCellChangedRatio = 0.30

    # P3：单个时间线热力格约 0.4% 画面；取 0.2%（约半个格）为下限，并至少 1 个热点单元。
    P3MinChangedRatio = 0.002
    P3MinHotCells = 1

    # 静态基线：同状态两帧在本机实测逐位一致（changed=0）。取 0.1% 作为噪声上限，与
    # P3 下限 0.2% 拉开 2x，避免把 GPU 抖动误判为播放推进。
    StaticMaxChangedRatio = 0.001

    # 帧差判据统一口径：通道差 > 6 记为变化像素；热点单元 32x32。
    ChangeTolerance = 6
    CellSize = 32

    # ticket 20：面板与标签都用不透明底色 #161b22(22,27,34)，场景里没有第二个同色块，
    # 因此按「背景色块像素数 + 纵向范围」度量，不受底层节点文字干扰
    # （先前的「正文白像素增量」被面板自身遮挡底层文字抵消，已废弃，理由记在票 20）。
    SearchPanelBg = @(22, 27, 34)
    SearchBgTolerance = 3
    # 面板最小形态 420x51 逻辑 ≈ 945x115 物理 ≈ 108k 像素，纯背景占 2/3 以上；取 20000（约 1/5）。
    SearchPanelBgMinPixels = 20000
    # 面板宽 945 物理像素；一行至少 50 个背景像素才算「仍处于面板内」。
    SearchPanelMinRowWidth = 50
    # 6 行命中每行约 23 逻辑 ≈ 52 物理像素，最保守 6x23x2.25 = 310；取 200 作为「列表把面板撑高」的下限。
    SearchPanelRowsMinHeightDiff = 200
    # 标签 220x24 逻辑 ≈ 500x54 物理 ≈ 27k 像素，背景占 60% 以上；取 5000（约 1/3）。
    SearchLabelBgMinPixels = 5000
    # 标签内正文白像素；集合外模块已淡到 0.68（白字混合后亮度约 160），不计入近白；取 300。
    SearchLabelMinTextPixels = 300
    # 清除后 / 无面板时的背景色容差上限（允许抗锯齿残留）。
    SearchClearedMaxBgPixels = 100

    # 搜索区域口径。
    SearchPanelX1 = 0.30
    SearchPanelY1 = 0.25
    SearchLabelX0 = 0.60
    SearchLabelY0 = 0.90
}

$centerX0 = 0.20; $centerX1 = 0.80
$centerY0 = 0.25; $centerY1 = 0.75

# ---- 像素工具（PowerShell 负责解码，C# 只处理 byte[]）----------------------
Add-Type -AssemblyName System.Drawing

if (-not ("MaskStats" -as [type])) {
    Add-Type -TypeDefinition @'
using System;

public class MaskStats {
    public int Total;
    public int Matched;
    public int Bands;
    public double Ratio;
    public int MinX = -1;
    public int MinY = -1;
    public int MaxX = -1;
    public int MaxY = -1;
}

public static class PixelMetrics {
    public static MaskStats Bright(byte[] data, int width, int height, int stride,
            int x0, int y0, int x1, int y1, int threshold, int minPerRow) {
        var stats = new MaskStats();
        if (x0 < 0) x0 = 0;
        if (y0 < 0) y0 = 0;
        if (x1 > width) x1 = width;
        if (y1 > height) y1 = height;
        int rows = y1 - y0;
        if (rows <= 0) return stats;
        int[] rowCounts = new int[rows];
        for (int y = y0; y < y1; y++) {
            int rowBase = y * stride;
            for (int x = x0; x < x1; x++) {
                int i = rowBase + x * 4;
                int b = data[i];
                int g = data[i + 1];
                int r = data[i + 2];
                double lum = 0.299 * r + 0.587 * g + 0.114 * b;
                stats.Total++;
                if (lum >= threshold) {
                    stats.Matched++;
                    rowCounts[y - y0]++;
                    if (stats.MinX < 0 || x < stats.MinX) stats.MinX = x;
                    if (stats.MaxX < 0 || x > stats.MaxX) stats.MaxX = x;
                    if (stats.MinY < 0 || y < stats.MinY) stats.MinY = y;
                    if (stats.MaxY < 0 || y > stats.MaxY) stats.MaxY = y;
                }
            }
        }
        stats.Ratio = stats.Total == 0 ? 0 : (double)stats.Matched / stats.Total;
        bool inBand = false;
        for (int i = 0; i < rows; i++) {
            bool on = rowCounts[i] >= minPerRow;
            if (on && !inBand) { stats.Bands++; inBand = true; }
            else if (!on) inBand = false;
        }
        return stats;
    }

    public static MaskStats Hue(byte[] data, int width, int height, int stride,
            int x0, int y0, int x1, int y1,
            int rMin, int rMax, int gMin, int gMax, int bMin, int bMax) {
        var stats = new MaskStats();
        if (x0 < 0) x0 = 0;
        if (y0 < 0) y0 = 0;
        if (x1 > width) x1 = width;
        if (y1 > height) y1 = height;
        for (int y = y0; y < y1; y++) {
            int rowBase = y * stride;
            for (int x = x0; x < x1; x++) {
                int i = rowBase + x * 4;
                int b = data[i];
                int g = data[i + 1];
                int r = data[i + 2];
                stats.Total++;
                if (r >= rMin && r <= rMax && g >= gMin && g <= gMax && b >= bMin && b <= bMax) {
                    stats.Matched++;
                    if (stats.MinX < 0 || x < stats.MinX) stats.MinX = x;
                    if (stats.MaxX < 0 || x > stats.MaxX) stats.MaxX = x;
                    if (stats.MinY < 0 || y < stats.MinY) stats.MinY = y;
                    if (stats.MaxY < 0 || y > stats.MaxY) stats.MaxY = y;
                }
            }
        }
        stats.Ratio = stats.Total == 0 ? 0 : (double)stats.Matched / stats.Total;
        return stats;
    }

    public static int CountNear(byte[] data, int width, int height, int stride,
            int x0, int y0, int x1, int y1, int tr, int tg, int tb, int tolerance) {
        if (x0 < 0) x0 = 0;
        if (y0 < 0) y0 = 0;
        if (x1 > width) x1 = width;
        if (y1 > height) y1 = height;
        int count = 0;
        for (int y = y0; y < y1; y++) {
            int rowBase = y * stride;
            for (int x = x0; x < x1; x++) {
                int i = rowBase + x * 4;
                int b = data[i];
                int g = data[i + 1];
                int r = data[i + 2];
                if (Math.Abs(r - tr) <= tolerance && Math.Abs(g - tg) <= tolerance && Math.Abs(b - tb) <= tolerance) {
                    count++;
                }
            }
        }
        return count;
    }

    /// 区域内某颜色的纵向范围与总数：逐行统计匹配像素，行内匹配数 >= minRowCount 才计入范围。
    /// 用于「面板/标签背景色」这类唯一色块，避免被底层内容干扰。
    public static long[] ColorExtent(byte[] data, int width, int height, int stride,
            int x0, int y0, int x1, int y1, int tr, int tg, int tb, int tolerance, int minRowCount) {
        if (x0 < 0) x0 = 0;
        if (y0 < 0) y0 = 0;
        if (x1 > width) x1 = width;
        if (y1 > height) y1 = height;
        long total = 0;
        int first = -1;
        int last = -1;
        for (int y = y0; y < y1; y++) {
            int rowBase = y * stride;
            int rowHits = 0;
            for (int x = x0; x < x1; x++) {
                int i = rowBase + x * 4;
                int b = data[i];
                int g = data[i + 1];
                int r = data[i + 2];
                if (Math.Abs(r - tr) <= tolerance && Math.Abs(g - tg) <= tolerance && Math.Abs(b - tb) <= tolerance) {
                    rowHits++;
                }
            }
            total += rowHits;
            if (rowHits >= minRowCount) {
                if (first < 0) first = y;
                last = y;
            }
        }
        return new long[] { first, last, total };
    }

    public static double MeanLuminance(byte[] data, int width, int height, int stride,
            int x0, int y0, int x1, int y1) {
        if (x0 < 0) x0 = 0;
        if (y0 < 0) y0 = 0;
        if (x1 > width) x1 = width;
        if (y1 > height) y1 = height;
        double sum = 0;
        long n = 0;
        for (int y = y0; y < y1; y++) {
            int rowBase = y * stride;
            for (int x = x0; x < x1; x++) {
                int i = rowBase + x * 4;
                int b = data[i];
                int g = data[i + 1];
                int r = data[i + 2];
                sum += 0.299 * r + 0.587 * g + 0.114 * b;
                n++;
            }
        }
        return n == 0 ? 0 : sum / n;
    }
}
'@
}

if (-not ("DiffStats" -as [type])) {
    Add-Type -TypeDefinition @'
using System;

public class DiffStats {
    public double ChangedRatio;
    public double MeanAbs;
    public double MaxCellRatio;
    public int HotCells;
    public int MinX = -1;
    public int MinY = -1;
    public int MaxX = -1;
    public int MaxY = -1;
}

public static class FrameDiff {
    public static DiffStats Compare(byte[] a, byte[] b, int width, int height,
            int strideA, int strideB, int cellSize, int changeTolerance, double hotThreshold) {
        var stats = new DiffStats();
        double changed = 0;
        double sumAbs = 0;
        long n = 0;
        int cellsX = (width + cellSize - 1) / cellSize;
        int cellsY = (height + cellSize - 1) / cellSize;
        long[] cellChanged = new long[cellsX * cellsY];
        long[] cellTotal = new long[cellsX * cellsY];
        for (int y = 0; y < height; y++) {
            int rowA = y * strideA;
            int rowB = y * strideB;
            int cellRow = (y / cellSize) * cellsX;
            for (int x = 0; x < width; x++) {
                int ia = rowA + x * 4;
                int ib = rowB + x * 4;
                int db = Math.Abs(a[ia] - b[ib]);
                int dg = Math.Abs(a[ia + 1] - b[ib + 1]);
                int dr = Math.Abs(a[ia + 2] - b[ib + 2]);
                int maxd = Math.Max(db, Math.Max(dg, dr));
                n++;
                cellTotal[cellRow + (x / cellSize)]++;
                if (maxd > changeTolerance) {
                    changed++;
                    cellChanged[cellRow + (x / cellSize)]++;
                    if (stats.MinX < 0 || x < stats.MinX) stats.MinX = x;
                    if (stats.MaxX < 0 || x > stats.MaxX) stats.MaxX = x;
                    if (stats.MinY < 0 || y < stats.MinY) stats.MinY = y;
                    if (stats.MaxY < 0 || y > stats.MaxY) stats.MaxY = y;
                }
                sumAbs += (db + dg + dr) / 3.0;
            }
        }
        stats.HotCells = 0;
        for (int i = 0; i < cellChanged.Length; i++) {
            if (cellTotal[i] == 0) continue;
            double ratio = (double)cellChanged[i] / cellTotal[i];
            if (ratio > stats.MaxCellRatio) stats.MaxCellRatio = ratio;
            if (ratio >= hotThreshold) stats.HotCells++;
        }
        stats.ChangedRatio = n == 0 ? 0 : changed / n;
        stats.MeanAbs = n == 0 ? 0 : sumAbs / n;
        return stats;
    }
}
'@
}

function Load-Pixels {
    param([string]$Path)
    if (-not (Test-Path $Path)) { throw "缺少帧文件 $Path" }
    $bmp = [System.Drawing.Bitmap]::FromFile($Path)
    $w = $bmp.Width; $h = $bmp.Height
    $rect = New-Object System.Drawing.Rectangle 0, 0, $w, $h
    $data = $bmp.LockBits($rect, [System.Drawing.Imaging.ImageLockMode]::ReadOnly, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $bytes = New-Object byte[] ($data.Stride * $h)
    [System.Runtime.InteropServices.Marshal]::Copy($data.Scan0, $bytes, 0, $bytes.Length)
    $stride = $data.Stride
    $bmp.UnlockBits($data)
    $bmp.Dispose()
    return @{ Bytes = $bytes; Width = $w; Height = $h; Stride = $stride; Path = $Path }
}

function Get-FrameFile {
    param([string]$Name, [int]$Index)
    return (Join-Path $OutDir ("{0}-f{1:D2}.png" -f $Name, $Index))
}

function Get-MeanLuminance {
    param($Img)
    return [PixelMetrics]::MeanLuminance($Img.Bytes, $Img.Width, $Img.Height, $Img.Stride, 0, 0, $Img.Width, $Img.Height)
}

function Test-FrameValid {
    param($Img)
    $lum = Get-MeanLuminance $Img
    return ($lum -ge $Thresholds.FrameMinMeanLuminance -and $lum -le $Thresholds.FrameMaxMeanLuminance)
}

function Get-CenterRect {
    param($Img)
    return @(
        [int]($Img.Width * $centerX0), [int]($Img.Height * $centerY0),
        [int]($Img.Width * $centerX1), [int]($Img.Height * $centerY1)
    )
}

# 区域内的正文白像素（#e6edf3 ±20）；用于搜索面板与聚焦标签。
function Get-TextPixels {
    param($Img, [double]$X0, [double]$Y0, [double]$X1, [double]$Y1)
    $r = @(
        [int]($Img.Width * $X0), [int]($Img.Height * $Y0),
        [int]($Img.Width * $X1), [int]($Img.Height * $Y1)
    )
    return [PixelMetrics]::CountNear($Img.Bytes, $Img.Width, $Img.Height, $Img.Stride, $r[0], $r[1], $r[2], $r[3], 230, 237, 243, 20)
}

function Get-PanelText {
    param($Img)
    return Get-TextPixels $Img 0 0 $Thresholds.SearchPanelX1 $Thresholds.SearchPanelY1
}

function Get-LabelText {
    param($Img)
    return Get-TextPixels $Img $Thresholds.SearchLabelX0 $Thresholds.SearchLabelY0 1.0 1.0
}

# 背景色块范围：返回 @(首行, 末行, 总像素数)。
function Get-ColorExtent {
    param($Img, [double]$X0, [double]$Y0, [double]$X1, [double]$Y1)
    $r = @(
        [int]($Img.Width * $X0), [int]($Img.Height * $Y0),
        [int]($Img.Width * $X1), [int]($Img.Height * $Y1)
    )
    $bg = $Thresholds.SearchPanelBg
    $extent = [PixelMetrics]::ColorExtent($Img.Bytes, $Img.Width, $Img.Height, $Img.Stride,
        $r[0], $r[1], $r[2], $r[3], $bg[0], $bg[1], $bg[2], $Thresholds.SearchBgTolerance,
        $Thresholds.SearchPanelMinRowWidth)
    return @([int]$extent[0], [int]$extent[1], [long]$extent[2])
}

function Get-PanelExtent {
    param($Img)
    return Get-ColorExtent $Img 0 0 $Thresholds.SearchPanelX1 $Thresholds.SearchPanelY1
}

function Get-LabelExtent {
    param($Img)
    return Get-ColorExtent $Img $Thresholds.SearchLabelX0 $Thresholds.SearchLabelY0 1.0 1.0
}

function Select-Frames {
    param([string]$Name, [int[]]$Indices)
    $valid = @()
    $blank = @()
    foreach ($i in $Indices) {
        $img = Load-Pixels (Get-FrameFile $Name $i)
        if (Test-FrameValid $img) { $valid += [pscustomobject]@{ Index = $i; Img = $img } }
        else { $blank += $i }
    }
    return [pscustomobject]@{ Valid = $valid; Blank = $blank }
}

# ---- 断言收集 --------------------------------------------------------------
$Assertions = @()
function Add-Assertion {
    param([string]$Id, [string]$Criterion, [string]$Reading, [bool]$Pass, [string]$Note = "")
    $script:Assertions += [pscustomobject]@{
        id = $Id; criterion = $Criterion; reading = $Reading; pass = $Pass; note = $Note
    }
    $flag = if ($Pass) { "PASS" } else { "FAIL" }
    Write-Output ("[{0}] {1} :: {2}" -f $flag, $Id, $Reading)
}

function Invoke-Capture {
    param(
        [string]$Name, [string[]]$AppArgs, [string]$Marker,
        [int]$WarmupMs, [int]$SettleMs, [int]$Frames, [int]$IntervalMs,
        [switch]$StdoutAtMarker,
        [int]$KeysAfterFrame = 0, [string]$Keys = "", [int]$KeysSettleMs = 300
    )
    $png = Join-Path $OutDir "$Name.png"
    $stdoutFile = Join-Path $OutDir "$Name.stdout.txt"
    $splat = @{
        AppArgs = $AppArgs
        OutPng = $png
        Exe = $Exe
        WarmupMs = $WarmupMs
        TimeoutSeconds = $TimeoutSeconds
        Frames = $Frames
        IntervalMs = $IntervalMs
        Marker = $Marker
        MarkerTimeoutSeconds = $TimeoutSeconds
        Method = "auto"
        SettleMs = $SettleMs
        StdoutFile = $stdoutFile
        StdoutAtMarker = $StdoutAtMarker
        KeysAfterFrame = $KeysAfterFrame
        Keys = $Keys
        KeysSettleMs = $KeysSettleMs
        KeysMethod = "post"
    }
    & $captureScript @splat | Out-Null
    return $stdoutFile
}

# ---- 固定样本 -------------------------------------------------------------
$historyDir = Join-Path $root "tests/fixtures/history"
if (-not (Test-Path (Join-Path $historyDir ".git"))) {
    & (Join-Path $root "tests/fixtures/history-seed/build.ps1") | Out-Null
}

# 合成 diff 仓库（复刻 crates/arch-core/tests/commit_diff.rs 的 build_fixture_repo）：
# added 3 / removed 2 / modified 1，保证三色齐全（history fixture 的 modified=0）。
$fixtureDir = Join-Path $OutDir "fixtures/diff-repo"
if (Test-Path $fixtureDir) { Remove-Item -LiteralPath $fixtureDir -Recurse -Force }
New-Item -ItemType Directory -Force -Path $fixtureDir | Out-Null

function Invoke-FixtureGit {
    param([string[]]$GitArgs)
    & git -C $fixtureDir @GitArgs | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "git -C fixture $($GitArgs -join ' ') 失败" }
}

function Write-FixtureFile {
    param([string]$Relative, [string]$Content)
    $path = Join-Path $fixtureDir $Relative
    $parent = Split-Path -Parent $path
    if (-not (Test-Path $parent)) { New-Item -ItemType Directory -Force -Path $parent | Out-Null }
    [System.IO.File]::WriteAllText($path, $Content, (New-Object System.Text.UTF8Encoding($false)))
}

$env:GIT_AUTHOR_NAME = "Fixture"; $env:GIT_AUTHOR_EMAIL = "fixture@example.com"
$env:GIT_COMMITTER_NAME = "Fixture"; $env:GIT_COMMITTER_EMAIL = "fixture@example.com"
Invoke-FixtureGit @("init", "-q")
Invoke-FixtureGit @("config", "user.name", "Fixture")
Invoke-FixtureGit @("config", "user.email", "fixture@example.com")
Invoke-FixtureGit @("config", "core.autocrlf", "false")
Invoke-FixtureGit @("config", "commit.gpgsign", "false")

Write-FixtureFile "src/a.ts" "export function alpha() {`n  return 1;`n}`n"
Write-FixtureFile "src/b.ts" "import { alpha } from `"./a`";`nexport function beta() {`n  return alpha();`n}`n"
Write-FixtureFile "src/old.ts" "export function legacy() {`n  return 3;`n}`n"
Write-FixtureFile "src/dead.ts" "export function dead() {`n  return 4;`n}`n"
$env:GIT_AUTHOR_DATE = "2026-02-01T12:00:00Z"; $env:GIT_COMMITTER_DATE = $env:GIT_AUTHOR_DATE
Invoke-FixtureGit @("add", "-A")
Invoke-FixtureGit @("commit", "-q", "-m", "base")
$base = (& git -C $fixtureDir rev-parse HEAD).Trim()

Write-FixtureFile "src/a.ts" "export function alpha() {`n  return 1;`n}`nexport function gamma() {`n  return 2;`n}`n"
Invoke-FixtureGit @("mv", "src/old.ts", "src/legacy.ts")
Remove-Item -LiteralPath (Join-Path $fixtureDir "src/dead.ts") -Force
Write-FixtureFile "src/new.ts" "export function fresh() {`n  return 5;`n}`n"
Write-FixtureFile "src/blank.ts" ""
$env:GIT_AUTHOR_DATE = "2026-02-02T12:00:00Z"; $env:GIT_COMMITTER_DATE = $env:GIT_AUTHOR_DATE
Invoke-FixtureGit @("add", "-A")
Invoke-FixtureGit @("commit", "-q", "-m", "next")
$head = (& git -C $fixtureDir rev-parse HEAD).Trim()
$env:GIT_AUTHOR_DATE = $null; $env:GIT_COMMITTER_DATE = $null

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

$report = [ordered]@{
    generated = (Get-Date).ToString("s")
    exe = $Exe
    thresholds = $Thresholds
    fixtures = [ordered]@{
        history = $historyDir
        diffRepo = $fixtureDir
        diffBase = $base
        diffHead = $head
    }
    states = [ordered]@{}
    blankFrames = [ordered]@{}
}

# ---- 状态 1：加载态 --------------------------------------------------------
$env:STRATA_PHASE_MARKERS = "1"
$env:STRATA_EVIDENCE_ANIMATION = $null

Write-Output "== state: loading =="
$loadingStdout = Invoke-Capture -Name "loading" -AppArgs @("timeline", ".", "--auto-close-ms", "6000") `
    -Marker "phase: frame:presented" -WarmupMs 0 -SettleMs 300 -Frames 3 -IntervalMs 100 -StdoutAtMarker
$sel = Select-Frames -Name "loading" -Indices @(1, 2, 3)
$report.blankFrames.loading = $sel.Blank
if ($sel.Valid.Count -eq 0) {
    Add-Assertion "loading-frame-valid" "存在有效帧（整帧平均亮度 15–200）" "无有效帧；空白帧=$($sel.Blank -join ',')" $false
} else {
    $img = $sel.Valid[0].Img
    $lum = Get-MeanLuminance $img
    $rect = Get-CenterRect $img
    $text = [PixelMetrics]::Bright($img.Bytes, $img.Width, $img.Height, $img.Stride, $rect[0], $rect[1], $rect[2], $rect[3], 200, $Thresholds.LoadingTextRowMinPixels)
    $report.states.loading = [ordered]@{
        frame = $sel.Valid[0].Index
        width = $img.Width; height = $img.Height
        meanLuminance = [Math]::Round($lum, 2)
        centerBrightPixels = $text.Matched
        centerBands = $text.Bands
        centerTextBbox = @($text.MinX, $text.MinY, $text.MaxX, $text.MaxY)
        blankFrames = $sel.Blank
    }
    Add-Assertion "loading-not-white" "整帧平均亮度 <= 200（排除白屏）" ("meanLuminance={0:N2}" -f $lum) ($lum -le $Thresholds.FrameMaxMeanLuminance)
    Add-Assertion "loading-text-visible" ("中心区亮度>=200 像素 >= {0} 且文字行带 >= {1}" -f $Thresholds.LoadingTextMinPixels, $Thresholds.LoadingTextMinBands) `
        ("bright={0} bands={1} bbox=({2},{3})-({4},{5})" -f $text.Matched, $text.Bands, $text.MinX, $text.MinY, $text.MaxX, $text.MaxY) `
        ($text.Matched -ge $Thresholds.LoadingTextMinPixels -and $text.Bands -ge $Thresholds.LoadingTextMinBands)
}
$loadingOut = if (Test-Path $loadingStdout) { Get-Content -Raw $loadingStdout } else { "" }
Add-Assertion "loading-captured-before-done" "抓取时 stdout 尚未出现 phase: load:done" `
    $(if ($loadingOut.Contains("phase: load:done")) { "contains load:done（可能已进入内容态）" } else { "no load:done" }) `
    (-not $loadingOut.Contains("phase: load:done"))

# ---- 状态 2：失败态 --------------------------------------------------------
Write-Output "== state: failure =="
$failureStdout = Invoke-Capture -Name "failure" -AppArgs @("timeline", "no-such-repo-for-evidence") `
    -Marker "phase: load:failed" -WarmupMs 0 -SettleMs 300 -Frames 3 -IntervalMs 100
$sel = Select-Frames -Name "failure" -Indices @(1, 2, 3)
$report.blankFrames.failure = $sel.Blank
if ($sel.Valid.Count -eq 0) {
    Add-Assertion "failure-frame-valid" "存在有效帧" "无有效帧；空白帧=$($sel.Blank -join ',')" $false
} else {
    $img = $sel.Valid[0].Img
    $lum = Get-MeanLuminance $img
    $rect = Get-CenterRect $img
    $lo = $Thresholds.FailureOrangeRgbMin; $hi = $Thresholds.FailureOrangeRgbMax
    $orange = [PixelMetrics]::Hue($img.Bytes, $img.Width, $img.Height, $img.Stride, $rect[0], $rect[1], $rect[2], $rect[3], $lo[0], $hi[0], $lo[1], $hi[1], $lo[2], $hi[2])
    $report.states.failure = [ordered]@{
        frame = $sel.Valid[0].Index
        width = $img.Width; height = $img.Height
        meanLuminance = [Math]::Round($lum, 2)
        centerOrangePixels = $orange.Matched
        centerOrangeBbox = @($orange.MinX, $orange.MinY, $orange.MaxX, $orange.MaxY)
        blankFrames = $sel.Blank
    }
    Add-Assertion "failure-frame-valid" "整帧平均亮度 15–200" ("meanLuminance={0:N2}" -f $lum) `
        ($lum -ge $Thresholds.FrameMinMeanLuminance -and $lum -le $Thresholds.FrameMaxMeanLuminance)
    Add-Assertion "failure-orange-visible" ("中心区橙色像素 >= {0}" -f $Thresholds.FailureOrangeMinPixels) `
        ("orange={0} bbox=({1},{2})-({3},{4})" -f $orange.Matched, $orange.MinX, $orange.MinY, $orange.MaxX, $orange.MaxY) `
        ($orange.Matched -ge $Thresholds.FailureOrangeMinPixels)
}
$failureOut = if (Test-Path $failureStdout) { Get-Content -Raw $failureStdout } else { "" }
Add-Assertion "failure-marker-seen" "stdout 出现 phase: load:failed" $(if ($failureOut.Contains("phase: load:failed")) { "seen" } else { "missing" }) ($failureOut.Contains("phase: load:failed"))

# ---- 状态 3：diff 三色 -----------------------------------------------------
Write-Output "== state: diff =="
$diffStdout = Invoke-Capture -Name "diff" -AppArgs @("diff", $fixtureDir, "--from", $base, "--to", $head, "--no-fade", "--auto-close-ms", "6000") `
    -Marker "diff: from=" -WarmupMs 1200 -SettleMs 300 -Frames 2 -IntervalMs 200
$sel = Select-Frames -Name "diff" -Indices @(1, 2)
$report.blankFrames.diff = $sel.Blank
$diffOut = if (Test-Path $diffStdout) { Get-Content -Raw $diffStdout } else { "" }
$diffCountsOk = $diffOut -match "added=3 removed=2 modified=1"
Add-Assertion "diff-fixture-counts" "stdout 含 added=3 removed=2 modified=1" $(($diffOut -split "`n" | Select-String "diff: from=" | ForEach-Object { $_.Line } | Select-Object -First 1)) $diffCountsOk
if ($sel.Valid.Count -eq 0) {
    Add-Assertion "diff-frame-valid" "存在有效帧" "无有效帧；空白帧=$($sel.Blank -join ',')" $false
} else {
    $img = $sel.Valid[-1].Img
    $tol = $Thresholds.DiffTintTolerance
    $added = [PixelMetrics]::CountNear($img.Bytes, $img.Width, $img.Height, $img.Stride, 0, 0, $img.Width, $img.Height, 47, 107, 63, $tol)
    $removed = [PixelMetrics]::CountNear($img.Bytes, $img.Width, $img.Height, $img.Stride, 0, 0, $img.Width, $img.Height, 122, 49, 56, $tol)
    $modified = [PixelMetrics]::CountNear($img.Bytes, $img.Width, $img.Height, $img.Stride, 0, 0, $img.Width, $img.Height, 122, 91, 36, $tol)
    $report.states.diff = [ordered]@{
        frame = $sel.Valid[-1].Index
        width = $img.Width; height = $img.Height
        addedTintPixels = $added
        removedTintPixels = $removed
        modifiedTintPixels = $modified
        blankFrames = $sel.Blank
    }
    foreach ($pair in @(@("added", 47, 107, 63, $added), @("removed", 122, 49, 56, $removed), @("modified", 122, 91, 36, $modified))) {
        Add-Assertion ("diff-tint-{0}" -f $pair[0]) ("颜色 #{0:x2}{1:x2}{2:x2} ±{3} 像素 >= {4}" -f $pair[1], $pair[2], $pair[3], $tol, $Thresholds.DiffTintMinPixels) `
            ("count={0}" -f $pair[4]) ($pair[4] -ge $Thresholds.DiffTintMinPixels)
    }
}

# ---- 状态 4：P1 上下文淡出 -------------------------------------------------
Write-Output "== state: p1-off =="
$p1Args = @("map", "tests/fixtures/callgraph", "--from", "src/b.ts#function:caller", "--to", "src/util.ts#function:helper", "--auto-close-ms", "6000")
$env:STRATA_EVIDENCE_ANIMATION = $null
$p1OffStdout = Invoke-Capture -Name "p1-off" -AppArgs $p1Args -Marker "map:" -WarmupMs 1500 -SettleMs 300 -Frames 2 -IntervalMs 200

Write-Output "== state: p1-on =="
$env:STRATA_EVIDENCE_ANIMATION = "1"
$p1OnStdout = Invoke-Capture -Name "p1-on" -AppArgs $p1Args -Marker "map:" -WarmupMs 1500 -SettleMs 300 -Frames 3 -IntervalMs 150
$env:STRATA_EVIDENCE_ANIMATION = $null

$offSel = Select-Frames -Name "p1-off" -Indices @(1, 2)
$onSel = Select-Frames -Name "p1-on" -Indices @(1, 2, 3)
$report.blankFrames.p1off = $offSel.Blank
$report.blankFrames.p1on = $onSel.Blank
if ($offSel.Valid.Count -eq 0 -or $onSel.Valid.Count -eq 0) {
    Add-Assertion "p1-frames-valid" "关闭态与开启态各至少一帧有效" ("off blank={0}; on blank={1}" -f ($offSel.Blank -join ','), ($onSel.Blank -join ',')) $false
} else {
    $offImg = $offSel.Valid[-1].Img
    $onImg = $onSel.Valid[-1].Img
    $d = [FrameDiff]::Compare($offImg.Bytes, $onImg.Bytes, $offImg.Width, $offImg.Height, $offImg.Stride, $onImg.Stride, $Thresholds.CellSize, $Thresholds.ChangeTolerance, $Thresholds.HotCellChangedRatio)
    $report.states.p1 = [ordered]@{
        offFrame = $offSel.Valid[-1].Index
        onFrame = $onSel.Valid[-1].Index
        width = $offImg.Width; height = $offImg.Height
        changedRatio = [Math]::Round($d.ChangedRatio, 4)
        meanAbs = [Math]::Round($d.MeanAbs, 3)
        maxCellRatio = [Math]::Round($d.MaxCellRatio, 4)
        hotCells = $d.HotCells
        changedBbox = @($d.MinX, $d.MinY, $d.MaxX, $d.MaxY)
        blankOff = $offSel.Blank
        blankOn = $onSel.Blank
    }
    Add-Assertion "p1-frames-distinguishable" ("帧差变化像素比例 >= {0:P1} 且热点单元(每个 >= {1:P0}) >= {2}" -f $Thresholds.P1MinChangedRatio, $Thresholds.HotCellChangedRatio, $Thresholds.P1MinHotCells) `
        ("changed={0:P3} meanAbs={1:N3} maxCell={2:P2} hotCells={3} bbox=({4},{5})-({6},{7})" -f $d.ChangedRatio, $d.MeanAbs, $d.MaxCellRatio, $d.HotCells, $d.MinX, $d.MinY, $d.MaxX, $d.MaxY) `
        ($d.ChangedRatio -ge $Thresholds.P1MinChangedRatio -and $d.HotCells -ge $Thresholds.P1MinHotCells)
}

# ---- 状态 5：P3 播放推进 ---------------------------------------------------
Write-Output "== state: p3 =="
$p3Stdout = Invoke-Capture -Name "p3" -AppArgs @("timeline", "tests/fixtures/history", "--autoplay", "--auto-close-ms", "6000") `
    -Marker "playback: frame tick 2" -WarmupMs 80 -SettleMs 200 -Frames 5 -IntervalMs 380
$sel = Select-Frames -Name "p3" -Indices @(1, 2, 3, 4, 5)
$report.blankFrames.p3 = $sel.Blank
$p3Out = if (Test-Path $p3Stdout) { Get-Content -Raw $p3Stdout } else { "" }
$tickCount = ([regex]::Matches($p3Out, "playback: frame tick")).Count
if ($sel.Valid.Count -lt 2) {
    Add-Assertion "p3-enough-frames" "至少两帧有效" ("valid={0}; blank={1}" -f $sel.Valid.Count, ($sel.Blank -join ',')) $false
} else {
    $pairs = @()
    $maxChanged = 0.0
    $maxHot = 0
    $maxCell = 0.0
    for ($i = 1; $i -lt $sel.Valid.Count; $i++) {
        $a = $sel.Valid[$i - 1].Img
        $b = $sel.Valid[$i].Img
        $d = [FrameDiff]::Compare($a.Bytes, $b.Bytes, $a.Width, $a.Height, $a.Stride, $b.Stride, $Thresholds.CellSize, $Thresholds.ChangeTolerance, $Thresholds.HotCellChangedRatio)
        $entry = [ordered]@{
            pair = ("f{0:D2}->f{1:D2}" -f $sel.Valid[$i - 1].Index, $sel.Valid[$i].Index)
            changedRatio = [Math]::Round($d.ChangedRatio, 4)
            meanAbs = [Math]::Round($d.MeanAbs, 3)
            maxCellRatio = [Math]::Round($d.MaxCellRatio, 4)
            hotCells = $d.HotCells
            changedBbox = @($d.MinX, $d.MinY, $d.MaxX, $d.MaxY)
        }
        $pairs += $entry
        if ($d.ChangedRatio -gt $maxChanged) { $maxChanged = $d.ChangedRatio }
        if ($d.HotCells -gt $maxHot) { $maxHot = $d.HotCells }
        if ($d.MaxCellRatio -gt $maxCell) { $maxCell = $d.MaxCellRatio }
        Write-Output ("  p3 {0}: changed={1:P3} meanAbs={2:N3} maxCell={3:P2} hotCells={4} bbox=({5},{6})-({7},{8})" -f $entry.pair, $d.ChangedRatio, $d.MeanAbs, $d.MaxCellRatio, $d.HotCells, $d.MinX, $d.MinY, $d.MaxX, $d.MaxY)
    }
    $report.states.p3 = [ordered]@{
        validFrames = ($sel.Valid | ForEach-Object { $_.Index })
        blankFrames = $sel.Blank
        tickLines = $tickCount
        pairs = $pairs
    }
    Add-Assertion "p3-adjacent-frame-moves" ("至少一对相邻帧 changedRatio >= {0:P1} 且热点单元 >= {1}" -f $Thresholds.P3MinChangedRatio, $Thresholds.P3MinHotCells) `
        ("maxChanged={0:P3} maxHotCells={1} maxCell={2:P2}" -f $maxChanged, $maxHot, $maxCell) `
        ($maxChanged -ge $Thresholds.P3MinChangedRatio -and $maxHot -ge $Thresholds.P3MinHotCells)
    Add-Assertion "p3-tick-lines" "stdout 至少 2 行 playback: frame tick" ("count={0}" -f $tickCount) ($tickCount -ge 2)
}

# ---- 静态基线：同状态两帧应逐位一致 ---------------------------------------
Write-Output "== state: p3-static =="
$staticStdout = Invoke-Capture -Name "p3-static" -AppArgs @("timeline", "tests/fixtures/history", "--auto-close-ms", "6000") `
    -Marker "timeline: " -WarmupMs 1200 -SettleMs 200 -Frames 2 -IntervalMs 380
$sel = Select-Frames -Name "p3-static" -Indices @(1, 2)
$report.blankFrames.p3static = $sel.Blank
if ($sel.Valid.Count -lt 2) {
    Add-Assertion "static-baseline-noise" "同状态两帧有效且帧差 <= 噪声上限" ("valid={0}; blank={1}" -f $sel.Valid.Count, ($sel.Blank -join ',')) $false
} else {
    $a = $sel.Valid[0].Img
    $b = $sel.Valid[1].Img
    $d = [FrameDiff]::Compare($a.Bytes, $b.Bytes, $a.Width, $a.Height, $a.Stride, $b.Stride, $Thresholds.CellSize, $Thresholds.ChangeTolerance, $Thresholds.HotCellChangedRatio)
    $report.states.p3static = [ordered]@{
        changedRatio = [Math]::Round($d.ChangedRatio, 6)
        meanAbs = [Math]::Round($d.MeanAbs, 6)
        maxCellRatio = [Math]::Round($d.MaxCellRatio, 6)
        hotCells = $d.HotCells
    }
    Add-Assertion "static-baseline-noise" ("同状态两帧 changedRatio <= {0:P1}" -f $Thresholds.StaticMaxChangedRatio) `
        ("changed={0:P6} meanAbs={1:N6} maxCell={2:P2} hotCells={3}" -f $d.ChangedRatio, $d.MeanAbs, $d.MaxCellRatio, $d.HotCells) `
        ($d.ChangedRatio -le $Thresholds.StaticMaxChangedRatio)
}

# ---- 状态 6：模块搜索（ticket 20）------------------------------------------
# 驱动方式：真实按键（PostMessage WM_KEYDOWN），与用户手动按键走同一条键盘链；
# 判据见 docs/tickets/arch-tool/20-module-search-focus.md C1/C2/C9/C10。
Write-Output "== state: search =="
# 与票 20 固定样本一致：802 模块语料，聚焦标签给的是真实的一跳计数。
$searchArgs = @("map", "tests/corpus/excalidraw", "--no-reveal", "--auto-close-ms", "15000")

$searchOffStdout = Invoke-Capture -Name "search-off" -AppArgs $searchArgs `
    -Marker "phase: content:presented" -WarmupMs 500 -SettleMs 300 -Frames 2 -IntervalMs 200
$searchResultsStdout = Invoke-Capture -Name "search-results" -AppArgs $searchArgs `
    -Marker "phase: content:presented" -WarmupMs 500 -SettleMs 300 -Frames 2 -IntervalMs 200 `
    -KeysAfterFrame 1 -Keys '/src' -KeysSettleMs 400
$searchNoMatchStdout = Invoke-Capture -Name "search-nomatch" -AppArgs $searchArgs `
    -Marker "phase: content:presented" -WarmupMs 500 -SettleMs 300 -Frames 2 -IntervalMs 200 `
    -KeysAfterFrame 1 -Keys '/zzzz' -KeysSettleMs 400
$searchFocusStdout = Invoke-Capture -Name "search-focus" -AppArgs $searchArgs `
    -Marker "phase: content:presented" -WarmupMs 500 -SettleMs 300 -Frames 2 -IntervalMs 200 `
    -KeysAfterFrame 1 -Keys '/src{ENTER}' -KeysSettleMs 900
$searchClearStdout = Invoke-Capture -Name "search-clear" -AppArgs $searchArgs `
    -Marker "phase: content:presented" -WarmupMs 500 -SettleMs 300 -Frames 2 -IntervalMs 200 `
    -KeysAfterFrame 1 -Keys '/src{ENTER}{ESC}' -KeysSettleMs 900

$offSel = Select-Frames -Name "search-off" -Indices @(1, 2)
$resSel = Select-Frames -Name "search-results" -Indices @(1, 2)
$noMatchSel = Select-Frames -Name "search-nomatch" -Indices @(1, 2)
$focusSel = Select-Frames -Name "search-focus" -Indices @(1, 2)
$clearSel = Select-Frames -Name "search-clear" -Indices @(1, 2)
$report.blankFrames.searchOff = $offSel.Blank
$report.blankFrames.searchResults = $resSel.Blank
$report.blankFrames.searchNoMatch = $noMatchSel.Blank
$report.blankFrames.searchFocus = $focusSel.Blank
$report.blankFrames.searchClear = $clearSel.Blank

function Read-StdoutText {
    param([string]$Path)
    if ($Path -and (Test-Path $Path)) { return (Get-Content -Raw $Path) }
    return ""
}
$resOut = Read-StdoutText $searchResultsStdout
$focusOut = Read-StdoutText $searchFocusStdout
$clearOut = Read-StdoutText $searchClearStdout

Add-Assertion "search-open-by-slash" "输入 / 后 stdout 出现 phase: search:open 与 key:/" `
    ("open={0} key={1}" -f $resOut.Contains("phase: search:open"), $resOut.Contains("phase: key:/")) `
    ($resOut.Contains("phase: search:open") -and $resOut.Contains("phase: key:/"))

if ($offSel.Valid.Count -eq 0 -or $resSel.Valid.Count -eq 0 -or $noMatchSel.Valid.Count -eq 0 -or
    $focusSel.Valid.Count -eq 0 -or $clearSel.Valid.Count -eq 0) {
    Add-Assertion "search-frames-valid" "五个搜索状态各至少一帧有效" `
        ("off={0} res={1} nomatch={2} focus={3} clear={4}" -f ($offSel.Blank -join ','), ($resSel.Blank -join ','), ($noMatchSel.Blank -join ','), ($focusSel.Blank -join ','), ($clearSel.Blank -join ',')) $false
} else {
    $offImg = $offSel.Valid[0].Img
    $resImg = $resSel.Valid[-1].Img
    $noMatchImg = $noMatchSel.Valid[-1].Img
    $focusImg = $focusSel.Valid[-1].Img
    $clearImg = $clearSel.Valid[-1].Img

    # 背景色块：@(首行, 末行, 总像素)
    $offPanel = Get-PanelExtent $offImg
    $resPanel = Get-PanelExtent $resImg
    $noMatchPanel = Get-PanelExtent $noMatchImg
    $focusPanel = Get-PanelExtent $focusImg
    $clearPanel = Get-PanelExtent $clearImg
    $focusLabel = Get-LabelExtent $focusImg
    $clearLabel = Get-LabelExtent $clearImg
    $offLabel = Get-LabelExtent $offImg
    $focusLabelText = Get-LabelText $focusImg

    $report.states.search = [ordered]@{
        offFrame = $offSel.Valid[0].Index
        resultsFrame = $resSel.Valid[-1].Index
        noMatchFrame = $noMatchSel.Valid[-1].Index
        focusFrame = $focusSel.Valid[-1].Index
        clearFrame = $clearSel.Valid[-1].Index
        panelBgOff = $offPanel[2]
        panelBgResults = $resPanel[2]
        panelBgNoMatch = $noMatchPanel[2]
        panelRowsResults = @($resPanel[0], $resPanel[1])
        panelRowsNoMatch = @($noMatchPanel[0], $noMatchPanel[1])
        panelBgFocus = $focusPanel[2]
        labelBgFocus = $focusLabel[2]
        labelTextFocus = $focusLabelText
        panelBgClear = $clearPanel[2]
        labelBgClear = $clearLabel[2]
        labelBgOff = $offLabel[2]
        focusStdout = ($focusOut -split "`n" | Select-String "search:label" | ForEach-Object { $_.Line } | Select-Object -First 1)
        blankFrames = [ordered]@{
            off = $offSel.Blank; results = $resSel.Blank; noMatch = $noMatchSel.Blank
            focus = $focusSel.Blank; clear = $clearSel.Blank
        }
    }

    Add-Assertion "search-panel-with-results" ("左上角面板底色像素 >= {0}" -f $Thresholds.SearchPanelBgMinPixels) `
        ("off={0} results={1}" -f $offPanel[2], $resPanel[2]) `
        ($resPanel[2] -ge $Thresholds.SearchPanelBgMinPixels)
    Add-Assertion "search-panel-no-match" ("左上角面板底色像素 >= {0}" -f $Thresholds.SearchPanelBgMinPixels) `
        ("off={0} nomatch={1}" -f $offPanel[2], $noMatchPanel[2]) `
        ($noMatchPanel[2] -ge $Thresholds.SearchPanelBgMinPixels)
    Add-Assertion "search-results-list-taller" ("有结果面板底边比无匹配低 >= {0}px" -f $Thresholds.SearchPanelRowsMinHeightDiff) `
        ("resultsBottom={0} nomatchBottom={1} diff={2}" -f $resPanel[1], $noMatchPanel[1], ($resPanel[1] - $noMatchPanel[1])) `
        (($resPanel[1] - $noMatchPanel[1]) -ge $Thresholds.SearchPanelRowsMinHeightDiff)
    Add-Assertion "search-focus-closes-panel" ("聚焦态左上角面板底色像素 <= {0}" -f $Thresholds.SearchClearedMaxBgPixels) `
        ("focus={0}" -f $focusPanel[2]) `
        ($focusPanel[2] -le $Thresholds.SearchClearedMaxBgPixels)
    Add-Assertion "search-focus-label-visible" ("聚焦态右下角标签底色 >= {0} 且正文白 >= {1}" -f $Thresholds.SearchLabelBgMinPixels, $Thresholds.SearchLabelMinTextPixels) `
        ("bg={0} text={1} rows={2}-{3}" -f $focusLabel[2], $focusLabelText, $focusLabel[0], $focusLabel[1]) `
        ($focusLabel[2] -ge $Thresholds.SearchLabelBgMinPixels -and $focusLabelText -ge $Thresholds.SearchLabelMinTextPixels)
    Add-Assertion "search-clear-removes-panel-and-label" ("清除后面板/标签底色 <= {0}，且与基线一致" -f $Thresholds.SearchClearedMaxBgPixels) `
        ("panel={0} label={1} offLabel={2}" -f $clearPanel[2], $clearLabel[2], $offLabel[2]) `
        ($clearPanel[2] -le $Thresholds.SearchClearedMaxBgPixels -and $clearLabel[2] -le $Thresholds.SearchClearedMaxBgPixels)
    Add-Assertion "search-focus-marker" "stdout 出现 phase: search:focus" $(if ($focusOut.Contains("phase: search:focus")) { "seen" } else { "missing" }) ($focusOut.Contains("phase: search:focus"))
    Add-Assertion "search-clear-marker" "stdout 出现 phase: search:focus 与 phase: search:clear" `
        ("focus={0} clear={1}" -f $focusOut.Contains("phase: search:focus"), $clearOut.Contains("phase: search:clear")) `
        ($clearOut.Contains("phase: search:focus") -and $clearOut.Contains("phase: search:clear"))

    $focusSummary = ($focusOut -split "`n" | Select-String "search:label module=" | ForEach-Object { $_.Line } | Select-Object -First 1)
    Add-Assertion "search-focus-stdout-counts" "聚焦态 stdout 给出 module 与 upstream/downstream 计数" $(if ($focusSummary) { $focusSummary.Trim() } else { "missing" }) ([bool]$focusSummary)
}

# ---- 汇总 ------------------------------------------------------------------
$env:STRATA_PHASE_MARKERS = $null
$report.assertions = $Assertions
$report.total = $Assertions.Count
$report.passed = ($Assertions | Where-Object { $_.pass }).Count
$report.failed = ($Assertions | Where-Object { -not $_.pass }).Count
$reportPath = Join-Path $OutDir "report.json"
[System.IO.File]::WriteAllText((Join-Path $root $reportPath), ($report | ConvertTo-Json -Depth 8), (New-Object System.Text.UTF8Encoding($false)))

Write-Output ""
Write-Output ("summary: passed={0} failed={1} -> {2}" -f $report.passed, $report.failed, $reportPath)
if ($report.failed -gt 0) {
    $Assertions | Where-Object { -not $_.pass } | ForEach-Object {
        Write-Output ("  FAILED {0}: {1} | {2} {3}" -f $_.id, $_.criterion, $_.reading, $_.note)
    }
    exit 1
}
exit 0
