param(
    [string]$PrivateKeyPath = (Join-Path $env:USERPROFILE '.tauri/ts-sddirecta.key'),
    [string]$Notes = ''
)

$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
Push-Location -LiteralPath $projectRoot
try {
    $package = Get-Content -LiteralPath 'package.json' -Raw -Encoding UTF8 | ConvertFrom-Json
    $tauri = Get-Content -LiteralPath 'src-tauri/tauri.conf.json' -Raw -Encoding UTF8 | ConvertFrom-Json
    $cargoText = [System.IO.File]::ReadAllText((Join-Path $projectRoot 'src-tauri/Cargo.toml'))
    $cargoVersion = [regex]::Match($cargoText, '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
    $version = $package.version
    if ($version -ne $tauri.version -or $version -ne $cargoVersion) {
        throw 'package.json / tauri.conf.json / Cargo.toml のバージョンを一致させてください。'
    }
    if (-not (Test-Path -LiteralPath $PrivateKeyPath -PathType Leaf)) {
        throw "署名用の秘密鍵がありません: $PrivateKeyPath"
    }
    $publicKey = [System.IO.File]::ReadAllText("$PrivateKeyPath.pub").Trim()
    $embeddedKey = [System.IO.File]::ReadAllText((Join-Path $projectRoot 'src-tauri/update-public-key.txt')).Trim()
    if ($publicKey -ne $embeddedKey) {
        throw '署名鍵の公開鍵とアプリに組み込む公開鍵が一致しません。'
    }
    & npm.cmd run build:exe
    if ($LASTEXITCODE -ne 0) { throw '配布用exeのビルドに失敗しました。' }
    $exePath = Join-Path $projectRoot 'src-tauri/target/release/ts-sddirecta.exe'
    $productVersion = (Get-Item -LiteralPath $exePath).VersionInfo.ProductVersion
    if ($productVersion -ne $version) {
        throw "生成されたexeのバージョンが一致しません: $productVersion / $version"
    }
    $architecture = (& rustc -vV | Select-String '^host: ').ToString().Substring(6)
    $target = switch -Regex ($architecture) {
        '^x86_64-pc-windows' { 'windows-x86_64'; break }
        '^aarch64-pc-windows' { 'windows-aarch64'; break }
        default { throw "未対応のビルド環境です: $architecture" }
    }
    $outputDirectory = Join-Path $projectRoot "dist-update/v$version"
    New-Item -ItemType Directory -Path $outputDirectory -Force | Out-Null
    $outputExe = Join-Path $outputDirectory 'ts-sddirecta.exe'
    Copy-Item -LiteralPath $exePath -Destination $outputExe -Force
    $manifest = [ordered]@{
        version = $version
        target = $target
        url = "https://github.com/shakenyanta/ts-sddirecta-releases/releases/download/v$version/ts-sddirecta.exe"
        size = (Get-Item -LiteralPath $outputExe).Length
        sha256 = (Get-FileHash -LiteralPath $outputExe -Algorithm SHA256).Hash.ToLowerInvariant()
        notes = $Notes
    }
    $manifestPath = Join-Path $outputDirectory 'latest.json'
    $manifestJson = $manifest | ConvertTo-Json -Depth 4
    [System.IO.File]::WriteAllText($manifestPath, $manifestJson, (New-Object System.Text.UTF8Encoding($false)))
    & npm.cmd run tauri -- signer sign -f $PrivateKeyPath $manifestPath
    if ($LASTEXITCODE -ne 0) { throw '更新情報の署名に失敗しました。リリースを公開しないでください。' }
    Write-Host "配布ファイルの作成が完了しました: $outputDirectory"
    Write-Host "GitHub Releasesでタグ v$version を作成し、以下の3ファイルを添付してください。"
    Write-Host '  ts-sddirecta.exe / latest.json / latest.json.sig'
    Write-Host '初回配布はexeを手動で配置します。次のバージョンからアプリ内の更新を利用できます。'
} finally {
    Pop-Location
}
