param(
    [switch]$DryRun
)

$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$repository = 'shakenyanta/ts-sddirecta-releases'
$package = Get-Content -LiteralPath (Join-Path $projectRoot 'package.json') -Raw -Encoding UTF8 | ConvertFrom-Json
$version = $package.version
if ($version -notmatch '^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$') {
    throw '通常版のバージョンを package.json に指定してください（例: 0.1.1）。'
}
$tag = "v$version"
$directory = Join-Path $projectRoot "dist-update/$tag"
$files = @('ts-sddirecta.exe', 'latest.json', 'latest.json.sig') | ForEach-Object {
    $path = Join-Path $directory $_
    if (-not (Test-Path -LiteralPath $path -PathType Leaf) -or (Get-Item -LiteralPath $path).Length -eq 0) {
        throw "配布ファイルがないか空です: $path 。先に npm.cmd run release:prepare を実行してください。"
    }
    $path
}
$manifest = Get-Content -LiteralPath $files[1] -Raw -Encoding UTF8 | ConvertFrom-Json
$exe = Get-Item -LiteralPath $files[0]
$downloadUrl = "https://github.com/$repository/releases/download/$tag/ts-sddirecta.exe"
if ($manifest.version -ne $version -or $exe.VersionInfo.ProductVersion -ne $version) {
    throw 'package.json・更新情報・exeのバージョンが一致しません。release:prepare で作り直してください。'
}
if ($manifest.url -cne $downloadUrl -or $manifest.target -notin @('windows-x86_64', 'windows-aarch64')) {
    throw '更新情報の配布先または対象CPUが不正です。release:prepare で作り直してください。'
}
if ($manifest.size -ne $exe.Length -or $manifest.sha256 -ne (Get-FileHash -LiteralPath $files[0] -Algorithm SHA256).Hash) {
    throw 'exeのサイズまたはSHA-256が更新情報と一致しません。release:prepare で作り直してください。'
}
Write-Host "公開先: $repository / $tag （最新版）"
$files | ForEach-Object { Write-Host "添付: $_" }
Write-Host "説明: $($manifest.notes)"
if ($DryRun) {
    Write-Host '確認のみ完了しました。GitHubへの接続・公開は行っていません。'
    return
}
if (-not (Get-Command gh -ErrorAction SilentlyContinue)) {
    throw 'GitHub CLIが必要です。winget install --id GitHub.cli --exact でインストールし、ターミナルを開き直してください。'
}

function Invoke-GitHub {
    param([string[]]$Arguments)
    $result = & gh @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw 'GitHub操作に失敗しました。上のエラーを確認してください。既存のリリースは上書きしません。下書きが残った場合はGitHubで内容を確認してください。'
    }
    return $result
}

& gh auth status --hostname github.com
if ($LASTEXITCODE -ne 0) {
    throw 'GitHub認証を確認できません。gh auth login --hostname github.com を実行してからやり直してください。'
}
$notesPath = Join-Path $directory 'release-notes.txt'
$notes = @"
$($manifest.notes)

既にお使いの方は、アプリの「アップデート確認」から更新できます。
初めて使う方は、Assetsの ts-sddirecta.exe をダウンロードしてください。
"@
[System.IO.File]::WriteAllText($notesPath, $notes, (New-Object System.Text.UTF8Encoding($false)))

# Attach everything to a draft before making it available to the updater.
$createArguments = @('release', 'create', $tag, '--repo', $repository, '--target', 'main', '--title', $tag, '--notes-file', $notesPath, '--draft') + $files
Invoke-GitHub -Arguments $createArguments | Out-Host
$draft = (Invoke-GitHub -Arguments @('release', 'view', $tag, '--repo', $repository, '--json', 'isDraft,isPrerelease,assets')) -join "`n" | ConvertFrom-Json
if (-not $draft.isDraft -or $draft.isPrerelease -or @($draft.assets).Count -ne 3) {
    throw '下書きの状態または添付ファイル数が想定と違うため、公開を停止しました。GitHubの下書きを確認してください。'
}
foreach ($file in $files) {
    $local = Get-Item -LiteralPath $file
    $asset = @($draft.assets | Where-Object { $_.name -ceq $local.Name })
    if ($asset.Count -ne 1 -or $asset[0].size -ne $local.Length -or $asset[0].state -ne 'uploaded') {
        throw "添付ファイルのアップロードを確認できないため、公開を停止しました: $($local.Name)"
    }
}
Invoke-GitHub -Arguments @('release', 'edit', $tag, '--repo', $repository, '--draft=false', '--prerelease=false', '--latest') | Out-Host
$published = (Invoke-GitHub -Arguments @('release', 'view', '--repo', $repository, '--json', 'tagName,isDraft,isPrerelease,url')) -join "`n" | ConvertFrom-Json
if ($published.tagName -ne $tag -or $published.isDraft -or $published.isPrerelease) {
    throw '公開操作後、最新版としての表示を確認できませんでした。GitHubで公開状態を確認してください。'
}
Write-Host "公開が完了しました: $($published.url)"
Write-Host 'テスト用の旧版exeを起動し、「アップデート確認」から更新してください。'
