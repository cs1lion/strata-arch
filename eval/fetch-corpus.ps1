# 固定样本仓的拉取 / 校验脚本（对应 eval/corpus.toml）
#
# 用法：
#   pwsh eval/fetch-corpus.ps1                      # 缓存缺失则拉取，存在则校验
#   pwsh eval/fetch-corpus.ps1 -Name tsx            # 只处理一个仓库
#   pwsh eval/fetch-corpus.ps1 -VerifyOnly          # 只校验，不下载
#   pwsh eval/fetch-corpus.ps1 -Force               # 重新下载并替换缓存
#   STRATA_CORPUS_DIR=D:\corpus pwsh eval/fetch-corpus.ps1
#
# 行为：
#   - 缓存目录来自 -CacheRoot > 环境变量 STRATA_CORPUS_DIR > corpus.toml 的 [cache] root。
#   - 已存在：按 corpus.toml 的 files / manifest_sha256 校验；不符即报错并以 1 退出，
#     绝不静默使用错误版本。
#   - 缺失：从 api.github.com 的 zipball 地址下载（会 302 到 codeload.github.com），
#     解压后校验，通过才替换缓存目录。
#   - 归档副本不含 .git，故不校验 commit 本身，而校验内容清单哈希（算法见 corpus.toml）。

[CmdletBinding()]
param(
    [string[]]$Name = @(),
    [string]$CacheRoot,
    [switch]$VerifyOnly,
    [switch]$Force
)

$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent $PSScriptRoot

function Read-CorpusSpec {
    param([string]$Path)

    if (-not (Test-Path -LiteralPath $Path)) {
        throw "pin 清单不存在: $Path"
    }

    $cache = @{}
    $repos = New-Object System.Collections.Generic.List[hashtable]
    $current = $null
    $section = ''

    foreach ($raw in (Get-Content -LiteralPath $Path -Encoding utf8)) {
        $line = $raw.Trim()
        if ($line -eq '' -or $line.StartsWith('#')) { continue }

        if ($line -eq '[[repo]]') {
            $current = @{}
            $repos.Add($current)
            $section = 'repo'
            continue
        }

        if ($line -match '^\[([A-Za-z0-9_.-]+)\]$') {
            $section = $Matches[1]
            continue
        }

        if ($line -notmatch '^([A-Za-z0-9_.-]+)\s*=\s*(.+)$') {
            throw "无法解析的行: $raw"
        }

        $key = $Matches[1]
        $value = $Matches[2].Trim()
        if ($value -match '^"([^"]*)"$') {
            $parsed = $Matches[1]
        } elseif ($value -match '^-?\d+$') {
            $parsed = [int64]$value
        } else {
            throw "不支持的值类型: $raw"
        }

        switch ($section) {
            'cache' { $cache[$key] = $parsed }
            'repo' { $current[$key] = $parsed }
            default { throw "未知段: $section" }
        }
    }

    return @{ Cache = $cache; Repos = $repos }
}

function Get-CorpusManifest {
    param([string]$Dir)

    $prefix = $Dir.TrimEnd([char]'\') + '\'
    $files = @(Get-ChildItem -LiteralPath $Dir -Recurse -File -Force)

    $map = @{}
    $names = New-Object System.Collections.Generic.List[string]
    foreach ($file in $files) {
        $relative = $file.FullName.Substring($prefix.Length).Replace('\', '/')
        $map[$relative] = $file
        $names.Add($relative)
    }

    $sorted = $names.ToArray()
    [Array]::Sort($sorted, [System.StringComparer]::Ordinal)

    $builder = New-Object System.Text.StringBuilder
    foreach ($relative in $sorted) {
        $file = $map[$relative]
        $hash = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        [void]$builder.Append($hash).Append(' ').Append([string]$file.Length).Append(' ').Append($relative).Append("`n")
    }

    $algorithm = [System.Security.Cryptography.SHA256]::Create()
    try {
        $bytes = [System.Text.Encoding]::UTF8.GetBytes($builder.ToString())
        $digest = ($algorithm.ComputeHash($bytes) | ForEach-Object { $_.ToString('x2') }) -join ''
    } finally {
        $algorithm.Dispose()
    }

    return @{ Files = $sorted.Length; Sha256 = $digest }
}

function Assert-Corpus {
    param([string]$Dir, [hashtable]$Repo)

    $actual = Get-CorpusManifest -Dir $Dir
    $expectedFiles = [int64]$Repo['files']
    $expectedManifest = [string]$Repo['manifest_sha256']

    if ($actual.Files -ne $expectedFiles) {
        throw "文件数不符（期望 $expectedFiles，实际 $($actual.Files)）: $Dir"
    }
    if ($actual.Sha256 -ne $expectedManifest) {
        throw "内容清单哈希不符（期望 $expectedManifest，实际 $($actual.Sha256)）: $Dir"
    }

    return $actual
}

function Get-ArchiveUrl {
    param([hashtable]$Repo)
    return "https://api.github.com/repos/$($Repo['owner'])/$($Repo['repo'])/zipball/$($Repo['commit'])"
}

function Install-Corpus {
    param([hashtable]$Repo, [string]$Root, [string]$Target)

    $work = Join-Path ([System.IO.Path]::GetTempPath()) ("strata-corpus-" + $Repo['name'] + '-' + [Guid]::NewGuid().ToString('N'))
    $staging = Join-Path $Root ('.staging-' + $Repo['name'])
    New-Item -ItemType Directory -Force -Path $work | Out-Null

    try {
        $zip = Join-Path $work 'archive.zip'
        $url = Get-ArchiveUrl -Repo $Repo
        Write-Output ("下载 {0} <- {1}" -f $Repo['name'], $url)
        Invoke-WebRequest -Uri $url -OutFile $zip -MaximumRedirection 5 -TimeoutSec 600 `
            -Headers @{ 'User-Agent' = 'strata-eval-corpus' }

        $extract = Join-Path $work 'extract'
        Expand-Archive -LiteralPath $zip -DestinationPath $extract -Force

        $top = @(Get-ChildItem -LiteralPath $extract -Directory -Force)
        if ($top.Count -ne 1) {
            throw "归档解压后应只有一个根目录，实际 $($top.Count) 个"
        }

        if (Test-Path -LiteralPath $staging) {
            Remove-Item -LiteralPath $staging -Recurse -Force
        }
        New-Item -ItemType Directory -Force -Path $staging | Out-Null
        Get-ChildItem -LiteralPath $top[0].FullName -Force |
            Copy-Item -Destination $staging -Recurse -Force

        $actual = Assert-Corpus -Dir $staging -Repo $Repo

        if (Test-Path -LiteralPath $Target) {
            Remove-Item -LiteralPath $Target -Recurse -Force
        }
        Rename-Item -LiteralPath $staging -NewName $Repo['name']

        return $actual
    } finally {
        if (Test-Path -LiteralPath $work) {
            Remove-Item -LiteralPath $work -Recurse -Force
        }
        if (Test-Path -LiteralPath $staging) {
            Remove-Item -LiteralPath $staging -Recurse -Force
        }
    }
}

$spec = Read-CorpusSpec -Path (Join-Path $PSScriptRoot 'corpus.toml')
$root = $CacheRoot
if (-not $root) { $root = $env:STRATA_CORPUS_DIR }
if (-not $root) { $root = [string]$spec.Cache['root'] }
if (-not [System.IO.Path]::IsPathRooted($root)) {
    $root = Join-Path $repoRoot $root
}
$root = [System.IO.Path]::GetFullPath($root)
Write-Output "corpus root: $root"
New-Item -ItemType Directory -Force -Path $root | Out-Null

$selected = @($spec.Repos)
if ($Name.Count -gt 0) {
    $known = @($spec.Repos | ForEach-Object { $_['name'] })
    foreach ($requested in $Name) {
        if ($known -notcontains $requested) {
            Write-Error "未知仓库名: $requested（可选: $($known -join ', ')）" -ErrorAction Continue
            exit 1
        }
    }
    $selected = @($spec.Repos | Where-Object { $Name -contains $_['name'] })
}

$failed = 0
foreach ($repo in $selected) {
    $target = Join-Path $root $repo['name']
    $present = (Test-Path -LiteralPath $target) -and
        ((Get-ChildItem -LiteralPath $target -Force | Measure-Object).Count -gt 0)

    try {
        if ($present -and -not $Force) {
            $actual = Assert-Corpus -Dir $target -Repo $repo
            Write-Output ("OK   {0} commit={1} files={2} manifest={3}" -f $repo['name'], $repo['commit'], $actual.Files, $actual.Sha256)
            continue
        }

        if ($VerifyOnly) {
            throw "缓存缺失或为空: $target（去掉 -VerifyOnly 可自动拉取）"
        }

        $actual = Install-Corpus -Repo $repo -Root $root -Target $target
        Write-Output ("FETCH OK {0} commit={1} files={2} manifest={3}" -f $repo['name'], $repo['commit'], $actual.Files, $actual.Sha256)
    } catch {
        $failed++
        Write-Output ("FAIL {0}: {1}" -f $repo['name'], $_.Exception.Message)
    }
}

if ($failed -gt 0) {
    exit 1
}
exit 0
