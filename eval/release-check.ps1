# 本地发布前检查：串起最小回归，输出一张读数表；任何一步非零则整体非零。
#
# 只用于本地发布前核对，不进入 CI（CI 已覆盖构建、测试与漂移门禁）。
#
# 覆盖：
#   - cargo fmt --check
#   - cargo test --workspace
#   - cargo build --locked --workspace（跳过用 -SkipBuild）
#   - arch-cli index（tiny fixture，产出 JSON）
#   - arch-view map（tiny fixture，窗口自动退出）
#   - arch-view timeline（临时仓库，2 个提交）
#   - arch-view diff（同一临时仓库 HEAD~1 → HEAD）
#   - arch-view map --from/--to（path fixture，路径播放）
#   - drift gate -PlanOnly
#
# 临时仓库从 tests/fixtures/tiny 复制并建两个提交，不依赖 tests/corpus 外部语料。
#
# 用法：
#   pwsh -File eval/release-check.ps1
#   pwsh -File eval/release-check.ps1 -SkipBuild -AutoCloseMs 3000
#   pwsh -File eval/release-check.ps1 -OutJson out/release-check.json

[CmdletBinding()]
param(
    [string]$ExeDir = "target/debug",
    [int]$AutoCloseMs = 5000,
    [string]$OutJson = "out/release-check.json",
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$suffix = if ($IsWindows) { ".exe" } else { "" }
$cli = Join-Path $ExeDir "arch-cli$suffix"
$view = Join-Path $ExeDir "arch-view$suffix"

$results = @()

function Invoke-Step {
    param(
        [string]$Name,
        [string]$Command,
        [scriptblock]$Body
    )

    $watch = [System.Diagnostics.Stopwatch]::StartNew()
    $code = 0
    $detail = ""
    try {
        $output = & $Body 2>&1
        $code = $LASTEXITCODE
        if ($null -eq $code) { $code = 0 }
        $tail = @($output | Where-Object { -not [string]::IsNullOrWhiteSpace($_) } | Select-Object -Last 1)
        if ($tail.Count -gt 0) { $detail = [string]$tail[0] }
    } catch {
        $code = 1
        $detail = ($_ | Out-String).Trim()
    }
    $watch.Stop()

    $script:results += [pscustomobject]@{
        Step = $Name
        Command = $Command
        Status = if ($code -eq 0) { "PASS" } else { "FAIL" }
        ExitCode = $code
        Milliseconds = $watch.ElapsedMilliseconds
        Detail = $detail
    }
    Write-Output ("{0,-4} {1,7}ms  {2}" -f `
            $(if ($code -eq 0) { "PASS" } else { "FAIL" }), $watch.ElapsedMilliseconds, $Name)
}

Write-Output "release check: exeDir=$ExeDir autoCloseMs=$AutoCloseMs skipBuild=$($SkipBuild.IsPresent)"

if (-not $SkipBuild) {
    Invoke-Step -Name "cargo build --locked --workspace" -Command "cargo build --locked --workspace" -Body {
        cargo build --locked --workspace
    }
}

Invoke-Step -Name "cargo fmt --check" -Command "cargo fmt --check" -Body {
    cargo fmt --check
}

Invoke-Step -Name "cargo test --workspace" -Command "cargo test --workspace" -Body {
    cargo test --workspace
}

# 临时仓库：tiny fixture + 两个提交，供 timeline 与 diff 使用。
$tempRepo = Join-Path $env:TEMP "strata-release-check"
if (Test-Path -LiteralPath $tempRepo) {
    Remove-Item -LiteralPath $tempRepo -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $tempRepo | Out-Null
Copy-Item -Path (Join-Path "tests/fixtures/tiny" "*") -Destination $tempRepo -Recurse -Force

Invoke-Step -Name "prepare temp repo (2 commits)" -Command "git init + 2 commits" -Body {
    git -C $tempRepo init -q
    git -C $tempRepo config user.name "release-check"
    git -C $tempRepo config user.email "release-check@example.invalid"
    git -C $tempRepo config core.autocrlf false
    git -C $tempRepo add -A
    git -C $tempRepo commit -q -m "base: tiny fixture"
    Set-Content -LiteralPath (Join-Path $tempRepo "src/release-check-extra.ts") `
        -Encoding utf8 -Value "export function releaseCheckExtra() {`n  return 1;`n}`n"
    git -C $tempRepo add src/release-check-extra.ts
    git -C $tempRepo commit -q -m "head: add release-check-extra.ts"
    Write-Output "prepared $tempRepo"
}

Invoke-Step -Name "arch-cli index (tiny)" -Command "$cli index tests/fixtures/tiny --out out/release-check-index.json" -Body {
    & $cli index tests/fixtures/tiny --out out/release-check-index.json
}

Invoke-Step -Name "arch-view map (tiny)" -Command "$view map tests/fixtures/tiny --auto-close-ms $AutoCloseMs" -Body {
    & $view map tests/fixtures/tiny --auto-close-ms $AutoCloseMs
}

Invoke-Step -Name "arch-view timeline (temp repo)" -Command "$view timeline <tempRepo> --auto-close-ms $AutoCloseMs" -Body {
    & $view timeline $tempRepo --auto-close-ms $AutoCloseMs
}

Invoke-Step -Name "arch-view diff (temp repo HEAD~1..HEAD)" -Command "$view diff <tempRepo> --from HEAD~1 --to HEAD --auto-close-ms $AutoCloseMs" -Body {
    & $view diff $tempRepo --from HEAD~1 --to HEAD --auto-close-ms $AutoCloseMs
}

Invoke-Step -Name "arch-view path (path fixture)" -Command "$view map tests/fixtures/path --from src/cli.ts#module-toplevel --to src/utils/helper.ts#function:helper --auto-close-ms $AutoCloseMs" -Body {
    & $view map tests/fixtures/path --from "src/cli.ts#module-toplevel" --to "src/utils/helper.ts#function:helper" --auto-close-ms $AutoCloseMs
}

Invoke-Step -Name "drift gate -PlanOnly" -Command "pwsh eval/drift-gate.ps1 -PlanOnly" -Body {
    pwsh -NoProfile -File eval/drift-gate.ps1 -PlanOnly -OutJson out/release-check-drift-plan.json
}

$outDir = Split-Path -Parent $OutJson
if ($outDir -and -not (Test-Path -LiteralPath $outDir)) {
    New-Item -ItemType Directory -Force -Path $outDir | Out-Null
}

Write-Output ""
Write-Output ("{0,-40} {1,-6} {2,8}  {3}" -f "step", "status", "ms", "detail")
foreach ($result in $results) {
    $detailText = $result.Detail
    if ($detailText.Length -gt 80) { $detailText = $detailText.Substring(0, 77) + "..." }
    Write-Output ("{0,-40} {1,-6} {2,8}  {3}" -f `
            $result.Step, $result.Status, $result.Milliseconds, $detailText)
}

$failed = @($results | Where-Object { $_.ExitCode -ne 0 })
$payload = [pscustomobject]@{
    schemaVersion = 1
    generatedAt = (Get-Date).ToString("o")
    exeDir = $ExeDir
    autoCloseMs = $AutoCloseMs
    skipBuild = $SkipBuild.IsPresent
    steps = $results
    passed = ($failed.Count -eq 0)
}
$payload | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $OutJson -Encoding utf8
Write-Output ""
Write-Output "wrote $OutJson"

if ($failed.Count -gt 0) {
    Write-Output ("release check: FAILED（{0} 步非零：{1}）" -f $failed.Count, ($failed.Step -join ", "))
    exit 1
}
Write-Output "release check: PASSED"
exit 0
