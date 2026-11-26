[CmdletBinding()]
param(
    [string]$Notes = '',
    [string]$PrivateKeyPath = (Join-Path $env:USERPROFILE '.tauri/ts-sddirecta.key'),
    [switch]$DryRun,
    [switch]$Build
)

$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
Push-Location -LiteralPath $projectRoot
try {
    $package = Get-Content -LiteralPath 'package.json' -Raw -Encoding UTF8 | ConvertFrom-Json
    if ($package.version -notmatch '^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$') {
        throw 'package.json に通常版のバージョンを指定してください（例: 0.1.18）。'
    }
    Write-Host "公開予定: $($package.version)"
    Write-Host '公開済み最新版との比較 → 同期 → ビルド・署名 → GitHub公開を実行します。'
    Write-Host "説明: $Notes"
    if ($DryRun) {
        Write-Host '予定の表示のみです。ファイル変更・ビルド・GitHubへの接続は行っていません。'
        return
    }
    if (-not (Get-Command gh -ErrorAction SilentlyContinue)) {
        throw 'GitHub CLIが必要です。ghをインストールしてログインしてください。'
    }
    $latestJson = & gh release view --repo shakenyanta/ts-sddirecta-releases --json tagName
    if ($LASTEXITCODE -ne 0) { throw '公開済み最新版を取得できません。認証・接続・リリースの有無を確認してください。公開は行いません。' }
    $latest = ($latestJson -join "`n" | ConvertFrom-Json).tagName
    if ($latest -cnotmatch '^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$') {
        throw "公開済み最新版のタグが通常版の形式ではありません: $latest"
    }
    $currentParts = $package.version.Split('.')
    $latestParts = $latest.Substring(1).Split('.')
    $comparison = 0
    for ($i = 0; $i -lt 3; $i++) {
        $comparison = ([bigint]$currentParts[$i]).CompareTo([bigint]$latestParts[$i])
        if ($comparison -ne 0) { break }
    }
    Write-Host "公開済み最新版: $latest"
    if ($comparison -le 0) {
        if ($Build) {
            Write-Host '新しいバージョンではないため、exeのビルドだけを実行します。'
            & npm.cmd run build:exe:only
            if ($LASTEXITCODE -ne 0) { throw 'exeのビルドに失敗しました。' }
            return
        }
        throw 'package.json の version が公開済み最新版より新しくありません。バージョンを上げてから実行してください。'
    }
    & node scripts/sync-version.mjs
    if ($LASTEXITCODE -ne 0) { throw 'バージョン同期に失敗しました。公開は行いません。' }
    $prepareArguments = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', 'scripts/prepare-portable-release.ps1', '-PrivateKeyPath', $PrivateKeyPath)
    if ($Notes -ne '') { $prepareArguments += @('-Notes', $Notes) }
    & powershell @prepareArguments
    if ($LASTEXITCODE -ne 0) { throw '配布ファイルの作成に失敗しました。公開は行いません。原因を解消して release:prepare から再開してください。' }
    & powershell -NoProfile -ExecutionPolicy Bypass -File scripts/publish-portable-release.ps1
    if ($LASTEXITCODE -ne 0) { throw '公開処理に失敗しました。GitHubの状態を確認し、release:publish から再開してください。' }
} finally {
    Pop-Location
}
