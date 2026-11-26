const { test } = require('node:test');
const assert = require('node:assert/strict');
const { mkdtempSync, mkdirSync, copyFileSync, writeFileSync, rmSync } = require('node:fs');
const { tmpdir } = require('node:os');
const { join, resolve } = require('node:path');
const { spawnSync } = require('node:child_process');

function runRelease(args = '', failAt = '', version = '0.1.18', latest = 'v0.1.17') {
  const root = mkdtempSync(join(tmpdir(), 'sddirecta-release-'));
  try {
    mkdirSync(join(root, 'scripts'));
    copyFileSync(resolve('scripts/release.ps1'), join(root, 'scripts/release.ps1'));
    writeFileSync(join(root, 'package.json'), JSON.stringify({ version }));
    const harness = `
$ErrorActionPreference = 'Stop'
function gh { Write-Host "CALL:gh:$args"; Write-Output '{"tagName":"${latest}"}'; $global:LASTEXITCODE = $(if ('${failAt}' -eq 'gh') { 1 } else { 0 }) }
function node { Write-Output "CALL:node:$args"; $global:LASTEXITCODE = $(if ('${failAt}' -eq 'node') { 1 } else { 0 }) }
function npm.cmd { Write-Output "CALL:npm:$args"; $global:LASTEXITCODE = $(if ('${failAt}' -eq 'build') { 1 } else { 0 }) }
function powershell {
    Write-Output "CALL:powershell:$args"
    $stage = if ($args -contains 'scripts/prepare-portable-release.ps1') { 'prepare' } else { 'publish' }
    $global:LASTEXITCODE = $(if ('${failAt}' -eq $stage) { 1 } else { 0 })
}
& "$PSScriptRoot/scripts/release.ps1" ${args}
`;
    writeFileSync(join(root, 'test.ps1'), '\uFEFF' + harness);
    return spawnSync('powershell', ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', join(root, 'test.ps1')], { encoding: 'utf8' });
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

test('release checks published version, syncs, prepares and publishes in order', { skip: process.platform !== 'win32' }, () => {
  const result = runRelease('-Notes "release notes"');
  assert.equal(result.status, 0, result.stderr);
  const calls = result.stdout.split(/\r?\n/).filter(line => line.startsWith('CALL:'));
  assert.equal(calls.length, 4);
  assert.match(calls[0], /gh:release view --repo shakenyanta\/ts-sddirecta-releases --json tagName/);
  assert.match(calls[1], /node:scripts\/sync-version.mjs/);
  assert.match(calls[2], /prepare-portable-release.ps1.*-Notes release notes/);
  assert.match(calls[3], /publish-portable-release.ps1/);
});

test('release stops after each failed stage', { skip: process.platform !== 'win32' }, () => {
  for (const [index, stage] of ['gh', 'node', 'prepare', 'publish'].entries()) {
    const result = runRelease('-Notes "test"', stage);
    assert.notEqual(result.status, 0, stage);
    assert.equal((result.stdout.match(/CALL:/g) || []).length, index + 1, stage);
  }
});

test('release dry run and invalid inputs never execute commands', { skip: process.platform !== 'win32' }, () => {
  for (const [args, version, status] of [
    ['-DryRun', '0.1.18', 0],
    ['-Bump patch', '0.1.18', 1],
    ['', '0.1.18-beta', 1],
  ]) {
    const result = runRelease(args, '', version);
    assert.equal(result.status, status, result.stderr);
    assert.doesNotMatch(result.stdout, /CALL:/);
  }
});

test('release rejects unchanged, older or invalid published versions before building', { skip: process.platform !== 'win32' }, () => {
  for (const latest of ['v0.1.18', 'v0.1.19', 'v0.2.0', 'v1.0.0', 'v0.1.17-beta', 'invalid']) {
    const result = runRelease('', '', '0.1.18', latest);
    assert.notEqual(result.status, 0, latest);
    assert.equal((result.stdout.match(/CALL:/g) || []).length, 1, latest);
  }
});

test('release compares numeric components across patch, minor and major changes', { skip: process.platform !== 'win32' }, () => {
  for (const [version, latest] of [['0.1.10', 'v0.1.9'], ['0.2.0', 'v0.1.99'], ['1.0.0', 'v0.99.99']]) {
    const result = runRelease('', '', version, latest);
    assert.equal(result.status, 0, result.stderr);
    assert.equal((result.stdout.match(/CALL:/g) || []).length, 4);
  }
});

test('normal exe build publishes only a newer version', { skip: process.platform !== 'win32' }, () => {
  const newer = runRelease('-Build -Notes "new version"');
  assert.equal(newer.status, 0, newer.stderr);
  assert.match(newer.stdout, /prepare-portable-release.ps1/);
  assert.match(newer.stdout, /publish-portable-release.ps1/);
  for (const latest of ['v0.1.18', 'v0.1.19']) {
    const result = runRelease('-Build', '', '0.1.18', latest);
    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stdout, /CALL:npm:run build:exe:only/);
    assert.equal((result.stdout.match(/CALL:/g) || []).length, 2);
  }
  for (const [stage, latest] of [['gh', 'v0.1.17'], ['build', 'v0.1.18'], ['prepare', 'v0.1.17']]) {
    const result = runRelease('-Build', stage, '0.1.18', latest);
    assert.notEqual(result.status, 0, stage);
    assert.doesNotMatch(result.stdout, /publish-portable-release.ps1/);
  }
});

test('release preparation and edition tests use builds without automatic publishing', () => {
  const { readFileSync } = require('node:fs');
  const pkg = JSON.parse(readFileSync('package.json', 'utf8'));
  assert.match(pkg.scripts['build:exe'], /release.ps1 -Build/);
  assert.match(pkg.scripts['build:exe:only'], /tauri build --no-bundle/);
  assert.match(pkg.scripts['edition:test:prepare'], /npm run build:exe:only/);
  assert.match(readFileSync('scripts/prepare-portable-release.ps1', 'utf8'), /npm.cmd run build:exe:only\r?\n/);
  assert.doesNotMatch(pkg.scripts.build, /release/);
});
