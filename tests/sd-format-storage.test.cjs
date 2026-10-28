const assert = require('node:assert/strict');
const { test } = require('node:test');
const { mkdtempSync, mkdirSync, readdirSync, rmSync } = require('node:fs');
const { resolve, join, sep } = require('node:path');
const { spawnSync } = require('node:child_process');

test('Windows formatter sequences only the confirmed disk and stops on errors', { skip: process.platform !== 'win32' }, () => {
  const cache = resolve('.cache/format-storage-tests');
  mkdirSync(cache, { recursive: true });
  const root = mkdtempSync(join(cache, 'case-'));
  const request = {
    mode: 'format', drive: 'H:', file_system: 'FAT', label: 'TEST',
    target: { disk: 77, capacity: 536870912, path: 'test-device', unique_id: 'test-id', serial: 'test-card', sector_size: 512 },
  };
  function run(mode, change = {}) {
    const result = spawnSync('powershell.exe', ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-File',
      'tests/sd-format-storage.mock.ps1', '-Mode', mode, '-Root', root], {
      input: JSON.stringify({ ...request, ...change }), encoding: 'utf8', timeout: 30000,
    });
    assert.ifError(result.error);
    return { ...result, events: result.stdout.trim().split(/\r?\n/).filter(Boolean).map(line => JSON.parse(line)) };
  }
  try {
    const good = run('success');
    assert.equal(good.status, 0, good.stdout + good.stderr);
    assert.deepEqual(good.events.filter(e => e.stage).map(e => e.stage), ['format_partition', 'format_volume', 'format_verify']);
    assert.equal(good.events.at(-1).result.file_system, 'FAT');
    assert.deepEqual(readdirSync(root), []); // Readback succeeded and the temporary file was removed.
    const calls = good.stderr.match(/CALL:(Clear-Disk|Initialize-Disk|New-Partition|Format-Volume)/g);
    assert.deepEqual(calls, ['CALL:Clear-Disk', 'CALL:Initialize-Disk', 'CALL:New-Partition', 'CALL:Format-Volume']);
    const raw = run('raw');
    assert.equal(raw.status, 0, raw.stdout + raw.stderr);
    assert.ok(!raw.stderr.includes('CALL:Clear-Disk'));
    assert.equal(raw.events.at(-1).result.file_system, 'FAT');
    const initialized = run('keep-mbr');
    assert.equal(initialized.status, 0, initialized.stdout + initialized.stderr);
    assert.ok(!initialized.stderr.includes('CALL:Initialize-Disk'));
    assert.equal(initialized.events.at(-1).result.file_system, 'FAT');
    const unassigned = run('unassigned', { drive: 'PhysicalDrive77' });
    assert.equal(unassigned.status, 0, unassigned.stdout + unassigned.stderr);
    assert.ok(!unassigned.stderr.includes('CALL:Clear-Disk'));
    assert.ok(!unassigned.stderr.includes('CALL:New-Partition'));
    assert.ok(unassigned.stderr.includes('CALL:Add-PartitionAccessPath'));
    assert.equal(unassigned.events.at(-1).result.drive, 'H:');
    for (const mode of ['changed', 'system', 'readonly']) {
      const failed = run(mode);
      assert.notEqual(failed.status, 0);
      assert.ok(failed.events.at(-1).error);
      assert.ok(!failed.stderr.includes('CALL:Clear-Disk'), mode);
    }
    for (const change of [{ file_system: 'NTFS' }, { label: 'bad;label' },
      { target: { ...request.target, capacity: 3 * 1024 ** 4 } }]) {
      const failed = run('success', change);
      assert.notEqual(failed.status, 0);
      assert.ok(!failed.stderr.includes('CALL:Clear-Disk'));
    }
    for (const mode of ['format-error', 'bad-result']) {
      const failed = run(mode);
      assert.notEqual(failed.status, 0);
      assert.ok(failed.events.at(-1).error);
      assert.ok(!failed.events.some(e => e.result));
    }
  } finally {
    const resolved = resolve(root);
    assert.ok(resolved.startsWith(cache + sep));
    rmSync(resolved, { recursive: true, force: true });
  }
});
