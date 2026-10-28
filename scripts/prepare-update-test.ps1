param(
    [string]$Version = '0.1.0',
    [string]$SourceExe = ''
)

$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
if ($Version -notmatch '^\d+\.\d+\.\d+$') {
    throw '更新元のバージョンは 0.1.0 のように指定してください。'
}
if (-not $SourceExe) {
    $SourceExe = Join-Path $projectRoot "dist-update/v$Version/ts-sddirecta.exe"
    if (-not (Test-Path -LiteralPath $SourceExe -PathType Leaf)) {
        $SourceExe = Join-Path $projectRoot 'src-tauri/target/release/ts-sddirecta.exe'
    }
}
if (-not (Test-Path -LiteralPath $SourceExe -PathType Leaf)) {
    throw 'コピー元のexeがありません。旧版の配布用exeを -SourceExe で指定してください。'
}
$source = Get-Item -LiteralPath $SourceExe
if ($source.VersionInfo.ProductVersion -ne $Version) {
    throw "コピー元のバージョンが違います: $($source.VersionInfo.ProductVersion)（必要: $Version）。旧版のexeを -SourceExe で指定してください。"
}
$testDirectory = Join-Path $projectRoot "dist-update/update-test/v$Version"
if (Test-Path -LiteralPath $testDirectory) {
    throw "テスト用フォルダーは既に存在します。上書きせず終了します: $testDirectory"
}
New-Item -ItemType Directory -Path $testDirectory -Force | Out-Null
$testExe = Join-Path $testDirectory 'ts-sddirecta.exe'
Copy-Item -LiteralPath $source.FullName -Destination $testExe
if ((Get-FileHash -LiteralPath $source.FullName -Algorithm SHA256).Hash -ne
    (Get-FileHash -LiteralPath $testExe -Algorithm SHA256).Hash) {
    throw 'コピー後のexeが一致しません。テスト用exeを使用しないでください。'
}
Write-Host "更新テスト用の $Version を準備しました。"
Write-Host "コピー元: $($source.FullName)"
Write-Host "テスト用exe: $testExe"
Write-Host '新バージョンを公開した後、このテスト用exeを起動して「アップデート確認」を押してください。'
