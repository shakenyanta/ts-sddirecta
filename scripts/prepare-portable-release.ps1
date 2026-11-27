param(
    [string]$PrivateKeyPath = (Join-Path $env:USERPROFILE '.tauri/ts-sddirecta.key'),
    [string]$Notes = ''
)

$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$signingPassword = $null
$probePath = $null

function Invoke-ReleaseSignature([string]$FilePath) {
    $previousPassword = [Environment]::GetEnvironmentVariable('TAURI_SIGNING_PRIVATE_KEY_PASSWORD', 'Process')
    $passwordPointer = [IntPtr]::Zero
    try {
        $passwordPointer = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($signingPassword)
        # Pass the password only through the signer's process environment, never command arguments.
        [Environment]::SetEnvironmentVariable('TAURI_SIGNING_PRIVATE_KEY_PASSWORD', [Runtime.InteropServices.Marshal]::PtrToStringBSTR($passwordPointer), 'Process')
        # PowerShell 5.1 drops empty native arguments and cannot retain an empty env value.
        $passwordArguments = @()
        if ($signingPassword.Length -eq 0) { $passwordArguments = @('--password=') }
        & node node_modules/@tauri-apps/cli/tauri.js signer sign -f $PrivateKeyPath @passwordArguments $FilePath
        if ($LASTEXITCODE -ne 0) { throw '署名に失敗しました。秘密鍵とパスワードを確認してください。' }
    } finally {
        [Environment]::SetEnvironmentVariable('TAURI_SIGNING_PRIVATE_KEY_PASSWORD', $previousPassword, 'Process')
        if ($passwordPointer -ne [IntPtr]::Zero) { [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($passwordPointer) }
    }
}

Push-Location -LiteralPath $projectRoot
try {
    & node scripts/sync-version.mjs
    if ($LASTEXITCODE -ne 0) { throw 'バージョンの自動同期に失敗しました。' }
    $package = Get-Content -LiteralPath 'package.json' -Raw -Encoding UTF8 | ConvertFrom-Json
    $tauri = Get-Content -LiteralPath 'src-tauri/tauri.conf.json' -Raw -Encoding UTF8 | ConvertFrom-Json
    $cargoText = [System.IO.File]::ReadAllText((Join-Path $projectRoot 'src-tauri/Cargo.toml'))
    $cargoVersion = [regex]::Match($cargoText, '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
    $version = $package.version
    if ($version -ne $tauri.version -or $version -ne $cargoVersion) {
        throw 'バージョンの自動同期後も不一致があります。同期対象ファイルを確認してください。'
    }
    if (-not (Test-Path -LiteralPath $PrivateKeyPath -PathType Leaf)) {
        throw "署名用の秘密鍵がありません: $PrivateKeyPath"
    }
    $publicKey = [System.IO.File]::ReadAllText("$PrivateKeyPath.pub").Trim()
    $embeddedKey = [System.IO.File]::ReadAllText((Join-Path $projectRoot 'src-tauri/update-public-key.txt')).Trim()
    if ($publicKey -ne $embeddedKey) {
        throw '署名鍵の公開鍵とアプリに組み込む公開鍵が一致しません。'
    }
    $existingPassword = [Environment]::GetEnvironmentVariable('TAURI_SIGNING_PRIVATE_KEY_PASSWORD', 'Process')
    if ($null -ne $existingPassword -and $existingPassword -ne '') {
        $signingPassword = ConvertTo-SecureString -String $existingPassword -AsPlainText -Force
    } else {
        $signingPassword = Read-Host '署名鍵のパスワード（未設定の場合はEnter）' -AsSecureString
    }
    $existingPassword = $null
    $probePath = Join-Path ([System.IO.Path]::GetTempPath()) ("ts-sddirecta-signing-" + [Guid]::NewGuid().ToString('N') + '.txt')
    [System.IO.File]::WriteAllText($probePath, 'ts-sddirecta signing check')
    Write-Host 'ビルド前に署名鍵とパスワードを確認しています。'
    Invoke-ReleaseSignature $probePath
    Remove-Item -LiteralPath $probePath, "$probePath.sig" -Force
    $probePath = $null
    Write-Host '署名の事前確認が完了しました。exeのビルドを開始します。'
    & npm.cmd run build:exe:only
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
    Invoke-ReleaseSignature $manifestPath
    Write-Host "配布ファイルの作成が完了しました: $outputDirectory"
    Write-Host 'exeの起動確認後、npm.cmd run release:publish で3ファイルをGitHubへ公開してください。'
    Write-Host '初回配布はexeを手動で配置します。次のバージョンからアプリ内の更新を利用できます。'
} finally {
    if ($null -ne $probePath) {
        Remove-Item -LiteralPath $probePath, "$probePath.sig" -Force -ErrorAction SilentlyContinue
    }
    if ($null -ne $signingPassword) { $signingPassword.Dispose() }
    Pop-Location
}
