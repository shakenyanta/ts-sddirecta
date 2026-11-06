const assert = require('node:assert/strict');
const { test } = require('node:test');
const { spawnSync } = require('node:child_process');

test('failure evidence bounds events and survives missing Windows/device evidence', { skip: process.platform !== 'win32' }, () => {
  function run(mode, capacity = 512000000) {
    const result = spawnSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass',
      '-File', 'tests/storage-diagnostics.mock.ps1', '-Mode', mode], {
      input: JSON.stringify({ disk: 77, capacity, failure_ms: 1793921496551 }), encoding: 'utf8', timeout: 15000,
    });
    assert.ifError(result.error);
    assert.equal(result.status, 0, result.stdout + result.stderr);
    return result.stdout.trim().split(/\r?\n/).map(s => JSON.parse(s));
  }
  const events = run('success');
  const volume = events.find(e => e.kind === 'volume_observed_after_failure').data;
  assert.equal(volume.offset, 0);
  assert.equal(volume.volumes[0].file_system, 'FAT32');
  assert.ok(events.indexOf(events.find(e => e.kind === 'volume_observed_after_failure')) < events.indexOf(events.find(e => e.kind === 'driver_chain')));
  assert.equal(events.filter(e => e.kind === 'windows_event').length, 48); // 24 per channel
  const warning = events.find(e => e.kind === 'windows_event').data;
  assert.equal(warning.id, 153);
  assert.equal(warning.properties[0], '01-02-03');
  assert.ok(warning.message.length < 1300);
  assert.equal(events.filter(e => e.kind === 'driver_chain').length, 1); // parent cycle stopped
  assert.equal(events.find(e => e.kind === 'driver_chain').data.version, '1.2.3');
  assert.equal(events.at(-1).kind, 'collection_complete');
  const mismatch = run('success', 513000000);
  assert.equal(mismatch.find(e => e.kind === 'device_observed_after_failure').data.expected_capacity_matches, false);
  assert.ok(mismatch.some(e => e.kind === 'driver_chain')); // observed data, not attributed to failed card
  const missing = run('no-events');
  assert.ok(missing.filter(e => e.kind === 'event_query').every(e => e.data.status === 'unavailable'));
  assert.ok(missing.some(e => e.kind === 'driver_chain'));
  const deviceError = run('device-error');
  assert.ok(deviceError.some(e => e.kind === 'device_query_error'));
  assert.equal(deviceError.at(-1).kind, 'collection_complete');
  const raw = run('raw-volume');
  assert.equal(raw.find(e => e.kind === 'volume_observed_after_failure').data.volumes[0].file_system, 'RAW');
  const failedVolume = run('volume-error');
  assert.ok(failedVolume.some(e => e.kind === 'volume_query_error'));
  assert.ok(!failedVolume.some(e => e.kind === 'volume_observed_after_failure'));
  assert.ok(failedVolume.some(e => e.kind === 'windows_event'));
  assert.equal(failedVolume.at(-1).kind, 'collection_complete');
});
