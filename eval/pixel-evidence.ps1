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
        int gated = 0;
        int maxRowHits = 0;
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
            if (rowHits > maxRowHits) maxRowHits = rowHits;
            if (rowHits >= minRowCount) {
                gated++;
                if (first < 0) first = y;
                last = y;
            }
        }
        return new long[] { first, last, total, gated, maxRowHits };
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

# P1 的逐像素 delta 分档计数（票 24 阶段 3 · 闭掉「直方图只存在于一次性离线探针、无法从入仓
# 产物复核」的耐久性缺口）。分档口径与 DiffStats/FrameDiff 一致：取三通道绝对差的最大值，
# 「任一通道差 > 阈值」记入该档。其中 `>6` 用的就是现行 `ChangeTolerance`，其比例应与
# `report.states.p1.changedRatio` 相符——这个自校验让新分档数据本身可被证伪，而不是另起一套口径。
# 用途：判据不变的前提下区分「淡出没发生」与「淡出发生了但幅度小」，前者是功能失效、
# 后者是强度退化；两者对 P1MinChangedRatio 的影响完全不同，不能靠放宽容差去猜。
if (-not ("DeltaBuckets" -as [type])) {
    Add-Type -TypeDefinition @'
using System;
public class DeltaBuckets {
    public static long[] Of(byte[] a, byte[] b, int w, int h, int strideA, int strideB) {
        long g1 = 0, g3 = 0, g6 = 0, g12 = 0, g24 = 0;
        long g0 = 0, sum = 0;
        for (int y = 0; y < h; y++) {
            int ra = y * strideA, rb = y * strideB;
            for (int x = 0; x < w; x++) {
                int ia = ra + x * 4, ib = rb + x * 4;
                int d1 = Math.Abs(a[ia] - b[ib]);
                int d2 = Math.Abs(a[ia + 1] - b[ib + 1]);
                int d3 = Math.Abs(a[ia + 2] - b[ib + 2]);
                int mx = d1 > d2 ? (d1 > d3 ? d1 : d3) : (d2 > d3 ? d2 : d3);
                if (mx > 0) { g0++; sum += mx; }
                if (mx > 1) g1++;
                if (mx > 3) g3++;
                if (mx > 6) g6++;
                if (mx > 12) g12++;
                if (mx > 24) g24++;
            }
        }
        return new long[] { g1, g3, g6, g12, g24, g0, sum };
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
    # [0]=首个达标行 [1]=末个达标行 [2]=原始匹配像素总数（未过滤，历史判据口径，保持不变）
    # [3]=达标行数（每行 >= SearchPanelMinRowWidth，即空间连贯性度量）[4]=最长单行命中数
    return @([int]$extent[0], [int]$extent[1], [long]$extent[2], [int]$extent[3], [int]$extent[4])
}

function Get-PanelExtent {
    param($Img)
    return Get-ColorExtent $Img 0 0 $Thresholds.SearchPanelX1 $Thresholds.SearchPanelY1
}

function Get-LabelExtent {
    param($Img)
    return Get-ColorExtent $Img $Thresholds.SearchLabelX0 $Thresholds.SearchLabelY0 1.0 1.0
}

# 逐态稳定度量（票 24 阶段 3 · R3）。同一状态的**最后两个有效帧**必须一致：
# 键控态（results/noMatch/focus/clear）用 `-KeysAfterFrame 1`，其第 1 帧是「输入前」，
# 只有第 2、3 帧同属「输入后」，因此比较取 Valid[-2] 与 Valid[-1]，而不是前两帧。
# 任一不等 ⇒ 该态尚未收敛，其读数不得用于任何判据：此时判红（用户裁决），因为继续比阈值
# 得到的红/绿同时可能来自产品行为、动画残帧或采样时机，无法归因。
function Get-PanelFrameStability {
    param([string]$Name, [object]$Sel)
    $valid = @($Sel.Valid)
    if ($valid.Count -lt 2) {
        return [pscustomobject]@{
            name = $Name; ok = $false
            detail = ("{0}: 有效帧 {1} 不足 2，无法证明稳定" -f $Name, $valid.Count)
        }
    }
    $m1 = Get-PanelExtent $valid[-2].Img
    $m2 = Get-PanelExtent $valid[-1].Img
    $ok = ($m1[2] -eq $m2[2]) -and ($m1[3] -eq $m2[3]) -and ($m1[4] -eq $m2[4])
    return [pscustomobject]@{
        name = $Name; ok = $ok
        detail = ("{0}: total {1}->{2} gatedRows {3}->{4} maxRowHits {5}->{6}" -f `
            $Name, $m1[2], $m2[2], $m1[3], $m2[3], $m1[4], $m2[4])
    }
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
    # 逐像素 delta 分档（仅记录，不参与任何判据）：用于区分「淡出未发生」与「淡出发生但幅度小」。
    $buckets = [DeltaBuckets]::Of($offImg.Bytes, $onImg.Bytes, $offImg.Width, $offImg.Height, $offImg.Stride, $onImg.Stride)
    $framePx = [long]$offImg.Width * $offImg.Height
    $report.states.p1 = [ordered]@{
        offFrame = $offSel.Valid[-1].Index
        onFrame = $onSel.Valid[-1].Index
        width = $offImg.Width; height = $offImg.Height
        changedRatio = [Math]::Round($d.ChangedRatio, 4)
        meanAbs = [Math]::Round($d.MeanAbs, 3)
        maxCellRatio = [Math]::Round($d.MaxCellRatio, 4)
        hotCells = $d.HotCells
        changedBbox = @($d.MinX, $d.MinY, $d.MaxX, $d.MaxY)
        # 分档明细（口径：max(|ΔR|,|ΔG|,|ΔB|) > 阈值 的像素数）。deltaGt6 应≈changedRatio×总像素，
        # 因为 6 就是现行 ChangeTolerance；这一自校验写进产物，读者可直接判否。
        deltaBuckets = [ordered]@{
            framePixels = $framePx
            gt1 = $buckets[0]
            gt3 = $buckets[1]
            gt6 = $buckets[2]
            gt12 = $buckets[3]
            gt24 = $buckets[4]
            gt1Pct = [Math]::Round($buckets[0] / $framePx * 100, 3)
            gt3Pct = [Math]::Round($buckets[1] / $framePx * 100, 3)
            gt6Pct = [Math]::Round($buckets[2] / $framePx * 100, 3)
            gt12Pct = [Math]::Round($buckets[3] / $framePx * 100, 3)
            gt24Pct = [Math]::Round($buckets[4] / $framePx * 100, 3)
            # 以下三项为「仅记录」项，不参与任何判据（票 24 追加）。理由：P1 归因中我曾把最低档
            # `>1`（严格大于）误当「变化面积」，而淡出减弱恰恰表现为位移整体左移到 m=1 这一档——
            # 面积几乎不变、幅度塌一个数量级。仅凭 `>1` 起的分档既无法区分这两种情形，也会让读数
            # 卡在量化台阶上不稳定。故补 `gt0`（真正的可检变化面积）与变化像素最大通道色差的均值。
            # 判据、阈值、退出码语义均不变；本节读数不得用于转绿任何判据。
            gt0 = $buckets[5]
            gt0Pct = [Math]::Round($buckets[5] / $framePx * 100, 3)
            meanMaxDeltaOverGt0 = if ($buckets[5] -gt 0) { [Math]::Round($buckets[6] / $buckets[5], 2) } else { 0 }
            toleranceUsed = $Thresholds.ChangeTolerance
        }
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
    -Marker "phase: content:presented" -WarmupMs 500 -SettleMs 300 -Frames 3 -IntervalMs 200
$searchResultsStdout = Invoke-Capture -Name "search-results" -AppArgs $searchArgs `
    -Marker "phase: content:presented" -WarmupMs 500 -SettleMs 300 -Frames 3 -IntervalMs 200 `
    -KeysAfterFrame 1 -Keys '/src' -KeysSettleMs 400
$searchNoMatchStdout = Invoke-Capture -Name "search-nomatch" -AppArgs $searchArgs `
    -Marker "phase: content:presented" -WarmupMs 500 -SettleMs 300 -Frames 3 -IntervalMs 200 `
    -KeysAfterFrame 1 -Keys '/zzzz' -KeysSettleMs 400
$searchFocusStdout = Invoke-Capture -Name "search-focus" -AppArgs $searchArgs `
    -Marker "phase: content:presented" -WarmupMs 500 -SettleMs 300 -Frames 3 -IntervalMs 200 `
    -KeysAfterFrame 1 -Keys '/src{ENTER}' -KeysSettleMs 900
$searchClearStdout = Invoke-Capture -Name "search-clear" -AppArgs $searchArgs `
    -Marker "phase: content:presented" -WarmupMs 500 -SettleMs 300 -Frames 3 -IntervalMs 200 `
    -KeysAfterFrame 1 -Keys '/src{ENTER}{ESC}' -KeysSettleMs 900

$offSel = Select-Frames -Name "search-off" -Indices @(1, 2, 3)
$resSel = Select-Frames -Name "search-results" -Indices @(1, 2, 3)
$noMatchSel = Select-Frames -Name "search-nomatch" -Indices @(1, 2, 3)
$focusSel = Select-Frames -Name "search-focus" -Indices @(1, 2, 3)
$clearSel = Select-Frames -Name "search-clear" -Indices @(1, 2, 3)
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
    # 取帧统一到 Valid[-1]（最后一个有效帧）。此前 off 取 Valid[0] 而其余取 Valid[-1]，
    # 两侧稳定条件不对称；实测 off 三帧逐字节相同（PNG 大小一致、total/gatedRows/maxRowHits 相同），
    # 故该统一对 off 的读数无观测影响。键控态的 Valid[-1] 现为第 3 帧（输入后），
    # 与旧的 Valid[-1]=第 2 帧相比是**更晚的输入后帧**，属本笔刻意的语义变更。
    $offImg = $offSel.Valid[-1].Img
    $resImg = $resSel.Valid[-1].Img
    $noMatchImg = $noMatchSel.Valid[-1].Img
    $focusImg = $focusSel.Valid[-1].Img
    $clearImg = $clearSel.Valid[-1].Img

    # 背景色块：@(首个达标行, 末个达标行, 原始匹配像素总数, 达标行数, 最长单行命中数)
    # 第 3 位是历史口径（未过滤）；第 4/5 位为笔 A 新加的空间连贯性度量，本笔起用于面板判据。
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
        offFrame = $offSel.Valid[-1].Index
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
    # R3 逐态稳定度量：五态各取「输入后」的最后两个有效帧比较 total / 达标行数 / 最长单行命中。
    # 键控态的第 1 帧是「输入前」，故不能用前两帧比（笔 A 初版那样做会误报 3 个「未稳定」，
    # 属自造假红）；本笔把每态多捕一帧（-Frames 3）后才具备同条件帧对。
    # 稳定性在这里是**判据的先决条件**：某态未稳定 ⇒ 该态的面板判据直接判不通过，
    # 而不是拿一个没收敛的读数去比阈值——否则红绿都无法归因。
    $settled = @()
    $settled += Get-PanelFrameStability -Name "off" -Sel $offSel
    $settled += Get-PanelFrameStability -Name "results" -Sel $resSel
    $settled += Get-PanelFrameStability -Name "noMatch" -Sel $noMatchSel
    $settled += Get-PanelFrameStability -Name "focus" -Sel $focusSel
    $settled += Get-PanelFrameStability -Name "clear" -Sel $clearSel
    $settledOk = @{}
    foreach ($s in $settled) { $settledOk[$s.name] = [bool]$s.ok }
    $unstableNames = @($settled | Where-Object { -not $_.ok } | ForEach-Object { $_.name })
    Add-Assertion "search-states-settled" "五个搜索态输入后两帧的 total/达标行数/最长行命中必须一致" `
        ((@($settled | ForEach-Object { $_.detail })) -join " | ") ($unstableNames.Count -eq 0)
    if ($unstableNames.Count -gt 0) {
        Write-Output ("  未稳定的搜索态: {0}（这些态的面板判据按未通过处理，不得据其读数下结论）" -f `
            (($unstableNames) -join ', '))
    }

    # R2 同运行地板：关闭态的连贯行数必须为 0。这是下面两条判据的参照，也是「本机存在
    # 散点同色像素」这一事实的显式化——实测 off 恒为 total 110 / gatedRows 0 / maxRowHits 5，
    # 那 110 个像素是场景内容里的散点（只占其 bbox 的 0.05%），不是面板。
    Add-Assertion "search-off-panel-floor-zero" "关闭态连贯行数必须为 0（作为同运行地板，且该态须已稳定）" `
        ("offGatedRows={0} offTotal={1} offMaxRowHits={2} settled={3}" -f $offPanel[3], $offPanel[2], $offPanel[4], $settledOk['off']) `
        (($offPanel[3] -eq 0) -and $settledOk['off'])
    # R1 判据语义重写（本笔的期望值变更）：面板是否存在改由**空间连贯性**判定——
    # `达标行数 == 0` 即「面板已关闭」。零新增常数：门控宽度沿用既有 SearchPanelMinRowWidth=50，
    # 未做任何拟合。分离度实测为 2 个数量级（关闭态 0 行 / maxRowHits 5，展开态 159 行 / 776），
    # 判据落在结构性分界 `>0` 上。旧的「绝对像素 ≤100」在本机结构性不可满足（散点地板 110）。
    Add-Assertion "search-focus-closes-panel" "聚焦态左上角连贯面板行数必须为 0（面板已关闭，且该态须已稳定）" `
        ("focusGatedRows={0} focusTotal={1} focusMaxRowHits={2} settled={3} | 地板 offGatedRows={4}" -f `
            $focusPanel[3], $focusPanel[2], $focusPanel[4], $settledOk['focus'], $offPanel[3]) `
        (($focusPanel[3] -eq 0) -and $settledOk['focus'])
    Add-Assertion "search-focus-label-visible" ("聚焦态右下角标签底色 >= {0} 且正文白 >= {1}" -f $Thresholds.SearchLabelBgMinPixels, $Thresholds.SearchLabelMinTextPixels) `
        ("bg={0} text={1} rows={2}-{3}" -f $focusLabel[2], $focusLabelText, $focusLabel[0], $focusLabel[1]) `
        ($focusLabel[2] -ge $Thresholds.SearchLabelBgMinPixels -and $focusLabelText -ge $Thresholds.SearchLabelMinTextPixels)
    # 清除态：面板改用连贯行数判定（同上 R1）；标签区保持原绝对像素上限——其实测地板
    # offLabel=51 与清除后 label=51 均远低于 100，判据可满足，不存在面板区那种「地板高于上限」
    # 的结构性问题，故本笔不扩大改动面。
    Add-Assertion "search-clear-removes-panel-and-label" ("清除后连贯面板行数必须为 0，且标签底色 <= {0} 与基线一致（该态须已稳定）" -f $Thresholds.SearchClearedMaxBgPixels) `
        ("panelGatedRows={0} panelTotal={1} label={2} offLabel={3} offGatedRows={4} settled={5}" -f `
            $clearPanel[3], $clearPanel[2], $clearLabel[2], $offLabel[2], $offPanel[3], $settledOk['clear']) `
        (($clearPanel[3] -eq 0) -and ($clearLabel[2] -le $Thresholds.SearchClearedMaxBgPixels) -and $settledOk['clear'])
    Add-Assertion "search-focus-marker" "stdout 出现 phase: search:focus" $(if ($focusOut.Contains("phase: search:focus")) { "seen" } else { "missing" }) ($focusOut.Contains("phase: search:focus"))
    Add-Assertion "search-clear-marker" "stdout 出现 phase: search:focus 与 phase: search:clear" `
        ("focus={0} clear={1}" -f $focusOut.Contains("phase: search:focus"), $clearOut.Contains("phase: search:clear")) `
        ($clearOut.Contains("phase: search:focus") -and $clearOut.Contains("phase: search:clear"))

    $focusSummary = ($focusOut -split "`n" | Select-String "search:label module=" | ForEach-Object { $_.Line } | Select-Object -First 1)
    Add-Assertion "search-focus-stdout-counts" "聚焦态 stdout 给出 module 与 upstream/downstream 计数" $(if ($focusSummary) { $focusSummary.Trim() } else { "missing" }) ([bool]$focusSummary)

    # 把新度量写进产物，使「散点 vs 连贯面板」日后无需外部探针即可复核（此前该分析只在一次性
    # 离线探针里做过，PNG 又被 gitignore，属已登记的证据耐久性缺口）。
    $report.states.search.panelGatedRowsOff = $offPanel[3]
    $report.states.search.panelGatedRowsFocus = $focusPanel[3]
    $report.states.search.panelGatedRowsResults = $resPanel[3]
    $report.states.search.panelGatedRowsNoMatch = $noMatchPanel[3]
    $report.states.search.panelGatedRowsClear = $clearPanel[3]
    $report.states.search.panelMaxRowHitsOff = $offPanel[4]
    $report.states.search.panelMaxRowHitsFocus = $focusPanel[4]
    $report.states.search.panelMaxRowHitsResults = $resPanel[4]
    $report.states.search.panelMaxRowHitsClear = $clearPanel[4]
    $report.states.search.settledFrames = @($settled | ForEach-Object {
        [ordered]@{ name = $_.name; ok = $_.ok; detail = $_.detail } })
    # 稳定验证已覆盖五个搜索态（本笔以 -Frames 3 补出同条件帧对，原缺口关闭）：
    # off 的三帧同为「输入前」，四个键控态取输入后的第 2、3 帧为同条件对。
    $report.states.search.settledPairs = 'off=帧2/3（无按键）；results/noMatch/focus/clear=输入后的帧2/3，帧1 是输入前故不参与比较'
    $report.states.search.panelCriterionBasis = '面板存在性以「达标行数(gatedRows)」判定：0=已关闭，>0=存在连贯面板；门控宽度沿用 SearchPanelMinRowWidth=50，无新增常数'
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
