$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$source = Join-Path $projectRoot 'src-tauri/target/release/ts-sddirecta.exe'
if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
    throw '配布用exeがありません。npm.cmd run edition:test:prepare を実行してください。'
}
$package = Get-Content -LiteralPath (Join-Path $projectRoot 'package.json') -Raw -Encoding UTF8 | ConvertFrom-Json
if ((Get-Item -LiteralPath $source).VersionInfo.ProductVersion -ne $package.version) {
    throw '配布用exeのバージョンが一致しません。npm.cmd run edition:test:prepare で再ビルドしてください。'
}
$runName = (Get-Date -Format 'yyyyMMdd-HHmmss') + '-' + [guid]::NewGuid().ToString('N').Substring(0, 8)
$root = Join-Path $projectRoot "dist-update/edition-test/$runName"
$hash = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash
foreach ($case in @('lite', 'pro', 'invalid')) {
    $directory = Join-Path $root $case
    New-Item -ItemType Directory -Path $directory -ErrorAction Stop | Out-Null
    $exe = Join-Path $directory 'ts-sddirecta.exe'
    Copy-Item -LiteralPath $source -Destination $exe -ErrorAction Stop
    if ((Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash -ne $hash) {
        throw "コピー後のexeが一致しません: $exe"
    }
    if ($case -ne 'lite') {
        $config = Join-Path $directory '.config'
        New-Item -ItemType Directory -Path $config -ErrorAction Stop | Out-Null
        $contents = if ($case -eq 'pro') { '{"edition":"pro"}' } else { '{"edition":' }
        [IO.File]::WriteAllText((Join-Path $config 'edition.json'), $contents, [Text.UTF8Encoding]::new($false))
    }
}
Write-Host "版の確認用フォルダーを作成しました: $root"
Write-Host 'lite: 設定なし → ts-sddirecta lite、詳細ボタンなし'
Write-Host 'pro: pro設定あり → ts-sddirecta pro、詳細ボタンあり'
Write-Host 'invalid: 不正設定 → ts-sddirecta lite、警告あり、詳細ボタンなし'
Write-Host 'proを起動したまま設定をliteに変更しても版は変わらず、終了・再起動後にライト版になることも確認してください。'
Write-Host 'SD書込・初期化・フォーマットは操作せず、表示だけを確認してください。'
