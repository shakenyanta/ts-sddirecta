const assert = require('node:assert/strict');
const { test } = require('node:test');
const { mkdtemp, mkdir, writeFile, readFile, stat, rm } = require('node:fs/promises');
const { join } = require('node:path');
const { tmpdir } = require('node:os');

async function fixture(t, version = '0.1.1') {
  const root = await mkdtemp(join(tmpdir(), 'sddirect-version-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  await mkdir(join(root, 'src-tauri'));
  await writeFile(join(root, 'package.json'), JSON.stringify({ version }));
  await writeFile(join(root, 'src-tauri/tauri.conf.json'), '{\r\n  "version": "0.1.0",\r\n  "app": { "windows": [] }\r\n}\r\n');
  await writeFile(join(root, 'src-tauri/Cargo.toml'), '[package]\r\nname = "app"\r\nversion = "0.1.0" # keep comment\r\n\r\n[dependencies]\r\nserde = { version = "1" }\r\n');
  await writeFile(join(root, 'package-lock.json'), JSON.stringify({ version: '0.1.0', lockfileVersion: 3,
    packages: { '': { version: '0.1.0' }, 'node_modules/dependency': { version: '9.8.7' } } }));
  return root;
}

test('one version edit updates app metadata, preserving dependencies and Cargo comments', async t => {
  const { syncVersion } = await import('../scripts/sync-version.mjs');
  const root = await fixture(t);
  const result = await syncVersion(root);
  assert.equal(result.version, '0.1.1');
  assert.equal(result.updatedFiles.length, 3);
  const config = JSON.parse(await readFile(join(root, 'src-tauri/tauri.conf.json'), 'utf8'));
  assert.equal(config.version, '0.1.1');
  assert.deepEqual(config.app.windows, []);
  assert.equal(await readFile(join(root, 'src-tauri/Cargo.toml'), 'utf8'),
    '[package]\r\nname = "app"\r\nversion = "0.1.1" # keep comment\r\n\r\n[dependencies]\r\nserde = { version = "1" }\r\n');
  const lock = JSON.parse(await readFile(join(root, 'package-lock.json'), 'utf8'));
  assert.equal(lock.version, '0.1.1');
  assert.equal(lock.packages[''].version, '0.1.1');
  assert.equal(lock.packages['node_modules/dependency'].version, '9.8.7');
  const modified = (await stat(join(root, 'src-tauri/Cargo.toml'))).mtimeMs;
  assert.deepEqual((await syncVersion(root)).updatedFiles, []);
  assert.equal((await stat(join(root, 'src-tauri/Cargo.toml'))).mtimeMs, modified);
});

test('invalid versions or malformed metadata fail before any target file is written', async t => {
  const { syncVersion } = await import('../scripts/sync-version.mjs');
  const root = await fixture(t, 'v0.1.1');
  const configPath = join(root, 'src-tauri/tauri.conf.json');
  const before = await readFile(configPath, 'utf8');
  for (const version of ['v0.1.1', '01.2.3', '1.2.3-01', '1.2', '', '1.2.3\n', '18446744073709551616.0.0']) {
    await writeFile(join(root, 'package.json'), JSON.stringify({ version }));
    await assert.rejects(syncVersion(root), /SemVer/);
    assert.equal(await readFile(configPath, 'utf8'), before);
  }
  await writeFile(join(root, 'package.json'), '{"version":"0.1.1"}');
  await writeFile(join(root, 'package-lock.json'), 'not json');
  await assert.rejects(syncVersion(root));
  assert.equal(await readFile(configPath, 'utf8'), before);
});

test('prerelease versions work without a lockfile; a missing Cargo package version stops synchronization', async t => {
  const { syncVersion } = await import('../scripts/sync-version.mjs');
  const root = await fixture(t, '0.2.0-beta.1+build.2');
  await rm(join(root, 'package-lock.json'));
  await syncVersion(root);
  assert.equal(JSON.parse(await readFile(join(root, 'src-tauri/tauri.conf.json'), 'utf8')).version, '0.2.0-beta.1+build.2');
  await writeFile(join(root, 'src-tauri/Cargo.toml'), '[package]\nname = "app"\n[dependencies]\nversion = "2"\n');
  await assert.rejects(syncVersion(root), /Cargo.toml/);
});
