# 生成 ticket 08 的固定历史样本仓库（tests/fixtures/history）。
#
# 仓库拓扑（全部时间为 2026-01）：
#   c1 12:00 新增 src/app.ts（3 行）、src/util.ts（4 行）
#   c2 次日 修改 src/app.ts（+1 -1）
#   c3 03 日 重命名 src/util.ts -> src/helpers.ts（纯重命名）
#   c4 04 日 空提交
#   c5 05 日 修改 src/helpers.ts（+1 -1）
#   side 分支 06 日 新增 src/merge-only.ts、修改 src/app.ts（第二父分支）
#   merge 07 日 合并 side（--no-ff）
#
# 预期读数见 docs/tickets/arch-tool/08-history-sampler-evolution-metrics.md。
# 该目录被 .gitignore 排除，不入库；可重复运行，每次都会重建。

$ErrorActionPreference = 'Stop'

$seed = $PSScriptRoot
$target = Join-Path (Split-Path -Parent $seed) 'history'

if (Test-Path -LiteralPath $target) {
    Remove-Item -LiteralPath $target -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $target | Out-Null

function Invoke-Git {
    param([string[]]$GitArguments)
    & git -C $target @GitArguments
    if ($LASTEXITCODE -ne 0) {
        throw "git $($GitArguments -join ' ') 失败"
    }
}

function Write-RepoFile {
    param([string]$Relative, [string]$Content)
    $path = Join-Path $target $Relative
    $parent = Split-Path -Parent $path
    if (-not (Test-Path -LiteralPath $parent)) {
        New-Item -ItemType Directory -Force -Path $parent | Out-Null
    }
    [System.IO.File]::WriteAllText($path, $Content, (New-Object System.Text.UTF8Encoding($false)))
}

function Commit {
    param([string]$Date, [string]$Message, [switch]$AllowEmpty)
    $env:GIT_AUTHOR_DATE = $Date
    $env:GIT_COMMITTER_DATE = $Date
    Invoke-Git @('add', '-A')
    $arguments = @('commit', '-q', '-m', $Message)
    if ($AllowEmpty) { $arguments += '--allow-empty' }
    Invoke-Git $arguments
}

Invoke-Git @('init', '-q')
Invoke-Git @('config', 'user.name', 'Fixture')
Invoke-Git @('config', 'user.email', 'fixture@example.com')
Invoke-Git @('config', 'core.autocrlf', 'false')
Invoke-Git @('config', 'commit.gpgsign', 'false')

Write-RepoFile 'src/app.ts' "export const a = 1;`nexport const b = 2;`nexport const c = 3;`n"
Write-RepoFile 'src/util.ts' "u1`nu2`nu3`nu4`n"
Commit '2026-01-01T12:00:00Z' 'c1 init'

Write-RepoFile 'src/app.ts' "export const a = 1;`nexport const b = 2;`nexport const c = 4;`n"
Commit '2026-01-02T12:00:00Z' 'c2 app'

Invoke-Git @('mv', 'src/util.ts', 'src/helpers.ts')
Commit '2026-01-03T12:00:00Z' 'c3 rename'

Commit '2026-01-04T12:00:00Z' 'c4 empty' -AllowEmpty

Write-RepoFile 'src/helpers.ts' "u1`nu2`nu3x`nu4`n"
Commit '2026-01-05T12:00:00Z' 'c5 helpers'

$branch = (& git -C $target symbolic-ref --short HEAD).Trim()
Invoke-Git @('checkout', '-q', '-b', 'side')
Write-RepoFile 'src/app.ts' "export const a = 1;`nexport const b = 2;`nexport const c = 4;`nexport const d = 5;`n"
Write-RepoFile 'src/merge-only.ts' "export const side = 1;`nexport const other = 2;`n"
Commit '2026-01-06T12:00:00Z' 'side change'

Invoke-Git @('checkout', '-q', $branch)
$env:GIT_AUTHOR_DATE = '2026-01-07T12:00:00Z'
$env:GIT_COMMITTER_DATE = '2026-01-07T12:00:00Z'
Invoke-Git @('merge', '-q', '--no-ff', '-m', 'merge side', 'side')

$env:GIT_AUTHOR_DATE = $null
$env:GIT_COMMITTER_DATE = $null

Write-Output "fixture ready: $target"
Write-Output ("first-parent commits: " + (& git -C $target rev-list --count --first-parent HEAD).Trim())
Write-Output ("HEAD: " + (& git -C $target rev-parse HEAD).Trim())
