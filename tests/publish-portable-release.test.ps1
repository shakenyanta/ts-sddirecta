# Run: powershell -NoProfile -ExecutionPolicy Bypass -File tests/publish-portable-release.test.ps1
# GitHub is mocked; these tests never connect to or publish to GitHub.
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$fixture = Join-Path $projectRoot ('.cache/publish-tests/' + [guid]::NewGuid())
$scripts = New-Item -ItemType Directory -Path (Join-Path $fixture 'scripts') -Force
$assets = New-Item -ItemType Directory -Path (Join-Path $fixture 'dist-update/v0.1.1') -Force
Copy-Item -LiteralPath (Join-Path $projectRoot 'scripts/publish-portable-release.ps1') -Destination $scripts.FullName
$entry = Join-Path $scripts.FullName 'publish-portable-release.ps1'
Set-Content -LiteralPath (Join-Path $fixture 'package.json') -Value '{"version":"0.1.1"}' -Encoding UTF8
Set-Content -LiteralPath (Join-Path $assets.FullName 'ts-sddirecta.exe') -Value 'fixture executable' -Encoding UTF8
Set-Content -LiteralPath (Join-Path $assets.FullName 'latest.json.sig') -Value 'fixture signature' -Encoding UTF8
$manifestPath = Join-Path $assets.FullName 'latest.json'
$manifest = @{
    version = '0.1.1'; target = 'windows-x86_64'
    url = 'https://github.com/shakenyanta/ts-sddirecta-releases/releases/download/v0.1.1/ts-sddirecta.exe'
    size = (Get-Item -LiteralPath (Join-Path $assets.FullName 'ts-sddirecta.exe')).Length
    sha256 = (Get-FileHash -LiteralPath (Join-Path $assets.FullName 'ts-sddirecta.exe')).Hash
    notes = 'Japanese notes are preserved: 更新テスト'
}
function Save-Manifest {
    $manifest | ConvertTo-Json | Set-Content -LiteralPath $manifestPath -Encoding UTF8
}
Save-Manifest

# Supply version metadata for the small text fixture rather than requiring a built exe.
function Get-Item {
    param([string]$LiteralPath)
    $item = Microsoft.PowerShell.Management\Get-Item -LiteralPath $LiteralPath
    if ($item.Name -eq 'ts-sddirecta.exe') {
        return [pscustomobject]@{ Name = $item.Name; Length = $item.Length; VersionInfo = @{ ProductVersion = '0.1.1' } }
    }
    return $item
}
$global:publishTestCalls = @()
$global:publishTestMode = 'success'
function gh {
    $global:publishTestCalls += ,@($args)
    $global:LASTEXITCODE = 0
    if ($args[0] -eq 'auth') {
        if ($global:publishTestMode -eq 'auth-failure') { $global:LASTEXITCODE = 1 }
        return
    }
    if ($args[1] -eq 'create') {
        if ($global:publishTestMode -eq 'create-failure') { $global:LASTEXITCODE = 1 }
        return
    }
    if ($args[1] -eq 'edit') { return }
    if ($args[1] -eq 'view' -and $args[2] -eq 'v0.1.1') {
        $remoteAssets = @('ts-sddirecta.exe', 'latest.json', 'latest.json.sig') | ForEach-Object {
            $length = (Get-Item -LiteralPath (Join-Path $assets.FullName $_)).Length
            if ($global:publishTestMode -eq 'bad-upload') { $length++ }
            @{ name = $_; size = $length; state = 'uploaded' }
        }
        return (@{ isDraft = $true; isPrerelease = $false; assets = @($remoteAssets) } | ConvertTo-Json -Depth 5)
    }
    return '{"tagName":"v0.1.1","isDraft":false,"isPrerelease":false,"url":"https://example.test/release"}'
}
function Assert-Test($Condition, $Message) {
    if (-not $Condition) { throw $Message }
}
function Expect-Failure {
    $failed = $false
    try { & $entry } catch { $failed = $true }
    Assert-Test $failed 'Expected failure.'
    $edits = @($global:publishTestCalls | Where-Object { $_[1] -eq 'edit' })
    Assert-Test ($edits.Count -eq 0) 'Release must not be published on failure.'
}

& $entry -DryRun
Assert-Test ($global:publishTestCalls.Count -eq 0) 'DryRun must not invoke gh.'
Write-Host 'PASS: DryRun does not contact GitHub'

& $entry
$operations = @($global:publishTestCalls | ForEach-Object { $_[0] + ' ' + $_[1] })
Assert-Test (($operations -join ',') -eq 'auth status,release create,release view,release edit,release view') 'Upload verification must precede publishing.'
$create = $global:publishTestCalls[1]
Assert-Test ($create -contains '--draft') 'Upload must use a draft.'
Assert-Test (($create | Where-Object { $_ -like "$($assets.FullName)*" -and $_ -notlike '*release-notes.txt' }).Count -eq 3) 'Exactly three assets must be attached.'
Assert-Test ($global:publishTestCalls[3] -contains '--latest') 'Release must be marked latest.'
Assert-Test ((Get-Content -LiteralPath (Join-Path $assets.FullName 'release-notes.txt') -Raw -Encoding UTF8).Contains($manifest.notes)) 'Release notes must preserve Unicode.'
Write-Host 'PASS: upload three files, verify, publish latest with Unicode notes'

$global:publishTestCalls = @()
$global:publishTestMode = 'create-failure'
Expect-Failure
Write-Host 'PASS: creation failure never edits an existing release'

$global:publishTestCalls = @()
$global:publishTestMode = 'bad-upload'
Expect-Failure
Write-Host 'PASS: incomplete upload is not published'

$global:publishTestCalls = @()
$global:publishTestMode = 'auth-failure'
Expect-Failure
Assert-Test ($global:publishTestCalls.Count -eq 1) 'Authentication failure must prevent release creation.'
Write-Host 'PASS: authentication failure prevents release creation'

$global:publishTestCalls = @()
$manifest.sha256 = 'invalid'
Save-Manifest
Expect-Failure
Assert-Test ($global:publishTestCalls.Count -eq 0) 'Invalid local files must fail before gh.'
Write-Host 'PASS: hash mismatch fails before contacting GitHub'

$manifest.sha256 = (Get-FileHash -LiteralPath (Join-Path $assets.FullName 'ts-sddirecta.exe')).Hash
$manifest.version = '0.1.0'
Save-Manifest
Expect-Failure
Assert-Test ($global:publishTestCalls.Count -eq 0) 'Version mismatch must fail before gh.'
Write-Host 'PASS: version mismatch fails before contacting GitHub'

$manifest.version = '0.1.1'
Save-Manifest
Set-Content -LiteralPath (Join-Path $assets.FullName 'latest.json.sig') -Value '' -NoNewline -Encoding ASCII
Expect-Failure
Assert-Test ($global:publishTestCalls.Count -eq 0) 'Empty signature must fail before gh.'
Write-Host 'PASS: empty signature fails before contacting GitHub'
