const { test } = require('node:test');
const assert = require('node:assert/strict');
const { mkdtempSync, mkdirSync, copyFileSync, writeFileSync, rmSync, readFileSync } = require('node:fs');
const { tmpdir } = require('node:os');
const { join, resolve } = require('node:path');
const { spawnSync } = require('node:child_process');

function prepare({ failAt = '', password = 'test-only-password', inherited = false, realSigner = false, keyPassword = password } = {}) {
  const root = mkdtempSync(join(tmpdir(), 'sddirecta-signing-test-'));
  try {
    for (const folder of ['scripts', 'src-tauri/target/release']) mkdirSync(join(root, folder), { recursive: true });
    copyFileSync(resolve('scripts/prepare-portable-release.ps1'), join(root, 'scripts/prepare-portable-release.ps1'));
    writeFileSync(join(root, 'package.json'), JSON.stringify({ version: '0.1.99' }));
    writeFileSync(join(root, 'src-tauri/tauri.conf.json'), JSON.stringify({ version: '0.1.99' }));
    writeFileSync(join(root, 'src-tauri/Cargo.toml'), '[package]\nversion = "0.1.99"\n');
    writeFileSync(join(root, 'src-tauri/update-public-key.txt'), 'test-public-key');
    writeFileSync(join(root, 'test.key'), 'test-private-key');
    writeFileSync(join(root, 'test.key.pub'), failAt === 'public-key' ? 'wrong-key' : 'test-public-key');
    const cli = resolve('node_modules/@tauri-apps/cli/tauri.js');
    if (realSigner) {
      const generated = spawnSync(process.execPath, [cli, 'signer', 'generate', '--ci', '-p', keyPassword, '-w', join(root, 'generated.key')], { encoding: 'utf8', timeout: 30000 });
      assert.equal(generated.status, 0, 'test key generation failed');
      copyFileSync(join(root, 'generated.key'), join(root, 'test.key'));
      copyFileSync(join(root, 'generated.key.pub'), join(root, 'test.key.pub'));
      copyFileSync(join(root, 'generated.key.pub'), join(root, 'src-tauri/update-public-key.txt'));
    }
    const harness = `
$ErrorActionPreference = 'Stop'
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = $null
${inherited ? `$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = '${password}'` : ''}
$originalPassword = [Environment]::GetEnvironmentVariable('TAURI_SIGNING_PRIVATE_KEY_PASSWORD', 'Process')
$global:testSignPaths = New-Object 'System.Collections.Generic.List[string]'
$global:testSignCount = 0
function Read-Host {
    param([string]$Prompt, [switch]$AsSecureString)
    if (-not $AsSecureString) { throw 'password must be masked' }
    Write-Host 'CALL:prompt'
    $secure = New-Object System.Security.SecureString
    foreach ($character in '${password}'.ToCharArray()) { $secure.AppendChar($character) }
    return $secure
}
function node {
    if ($args -contains 'signer') {
        $global:testSignCount++
        Write-Host "CALL:sign:$global:testSignCount"
        if ($args -contains '${password}' -and '${password}' -ne '') { throw 'password leaked to arguments' }
        if ([Environment]::GetEnvironmentVariable('TAURI_SIGNING_PRIVATE_KEY_PASSWORD', 'Process') -ne $originalPassword -and '${password}' -eq '') { throw 'unexpected empty password environment' }
        if ('${password}' -ne '' -and $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD -cne '${password}') { throw 'password not passed to signer' }
        $path = $args[-1]
        $global:testSignPaths.Add($path)
        ${realSigner ? `& '${process.execPath.replaceAll("'", "''")}' '${cli.replaceAll("'", "''")}' @($args | Select-Object -Skip 1)
        return` : ''}
        if ('${failAt}' -eq 'password' -or ('${failAt}' -eq 'final-sign' -and $global:testSignCount -eq 2)) {
            $global:LASTEXITCODE = 1
        } else {
            [IO.File]::WriteAllText("$path.sig", 'test-signature')
            $global:LASTEXITCODE = 0
        }
    } else { Write-Host 'CALL:sync'; $global:LASTEXITCODE = 0 }
}
function npm.cmd {
    Write-Host 'CALL:build'
    if ([Environment]::GetEnvironmentVariable('TAURI_SIGNING_PRIVATE_KEY_PASSWORD', 'Process') -cne $originalPassword) { throw 'password environment leaked into build' }
    [IO.File]::WriteAllText((Join-Path $PSScriptRoot 'src-tauri/target/release/ts-sddirecta.exe'), 'test-exe')
    $global:LASTEXITCODE = $(if ('${failAt}' -eq 'build') { 1 } else { 0 })
}
function rustc { 'host: x86_64-pc-windows-msvc'; $global:LASTEXITCODE = 0 }
function Get-Item {
    param([string]$LiteralPath)
    if ($LiteralPath -like '*ts-sddirecta.exe') {
        return [pscustomobject]@{ Length = 8; VersionInfo = [pscustomobject]@{ ProductVersion = '0.1.99' } }
    }
    Microsoft.PowerShell.Management\Get-Item -LiteralPath $LiteralPath
}
$failed = $false
try { & "$PSScriptRoot/scripts/prepare-portable-release.ps1" -PrivateKeyPath "$PSScriptRoot/test.key" }
catch { $failed = $true; Write-Host 'RESULT:failed'; Write-Host $_.Exception.Message }
finally {
    if ([Environment]::GetEnvironmentVariable('TAURI_SIGNING_PRIVATE_KEY_PASSWORD', 'Process') -cne $originalPassword) { throw 'password environment was not restored' }
    foreach ($path in $global:testSignPaths) {
        if ($path -like '*ts-sddirecta-signing-*' -and ((Test-Path -LiteralPath $path) -or (Test-Path -LiteralPath "$path.sig"))) { throw 'probe was not cleaned up' }
    }
    Write-Host 'RESULT:cleanup-ok'
}
if ($failed) { exit 1 }
`;
    writeFileSync(join(root, 'test.ps1'), '\uFEFF' + harness);
    const result = spawnSync('powershell', ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', join(root, 'test.ps1')], { encoding: 'utf8', timeout: 90000 });
    if (password) {
      assert.ok(!result.stdout.includes(password));
      assert.ok(!result.stderr.includes(password));
    }
    return result;
  } finally { rmSync(root, { recursive: true, force: true }); }
}

test('password is requested and checked before building and reused for the final signature', { skip: process.platform !== 'win32' }, () => {
  const result = prepare();
  assert.equal(result.status, 0, result.stderr + result.stdout);
  assert.deepEqual(result.stdout.match(/CALL:[^\r\n]+/g), ['CALL:sync', 'CALL:prompt', 'CALL:sign:1', 'CALL:build', 'CALL:sign:2']);
  assert.match(result.stdout, /RESULT:cleanup-ok/);
});

test('wrong password stops before building and all failures restore the environment and delete the probe', { skip: process.platform !== 'win32' }, () => {
  for (const failAt of ['password', 'build', 'final-sign', 'public-key']) {
    const result = prepare({ failAt });
    assert.equal(result.status, 1, result.stderr);
    assert.match(result.stdout, /RESULT:failed/);
    assert.match(result.stdout, /RESULT:cleanup-ok/);
    if (failAt === 'password' || failAt === 'public-key') assert.doesNotMatch(result.stdout, /CALL:build/);
    if (failAt === 'public-key') assert.doesNotMatch(result.stdout, /CALL:prompt|CALL:sign/);
    if (failAt === 'build') assert.doesNotMatch(result.stdout, /CALL:sign:2/);
    if (failAt === 'build' || failAt === 'final-sign') assert.match(result.stdout, /CALL:build/);
    if (failAt === 'final-sign') assert.match(result.stdout, /CALL:sign:2/);
  }
});

test('installed Tauri signer accepts reused and empty passwords and rejects a wrong password before building', { skip: process.platform !== 'win32' }, () => {
  for (const options of [
    { password: 'test-only-password' },
    { password: '' },
    { password: 'wrong-test-password', keyPassword: 'correct-test-password' },
  ]) {
    const result = prepare({ ...options, realSigner: true });
    assert.equal(result.status, options.keyPassword ? 1 : 0, result.stderr + result.stdout);
    assert.match(result.stdout, /RESULT:cleanup-ok/);
    if (options.keyPassword) assert.doesNotMatch(result.stdout, /CALL:build/);
    else assert.match(result.stdout, /CALL:sign:2/);
  }
});

test('preconfigured and empty passwords work without exposing the password', { skip: process.platform !== 'win32' }, () => {
  for (const options of [{ inherited: true }, { password: '' }]) {
    const result = prepare(options);
    assert.equal(result.status, 0, result.stderr + result.stdout);
    assert.equal((result.stdout.match(/CALL:prompt/g) || []).length, options.inherited ? 0 : 1);
    assert.match(result.stdout, /CALL:sign:2/);
    assert.match(result.stdout, /RESULT:cleanup-ok/);
  }
});
