const assert = require('node:assert/strict');
const { test } = require('node:test');
const { readFileSync, mkdtempSync, writeFileSync } = require('node:fs');
const { tmpdir } = require('node:os');
const { resolve, join } = require('node:path');
const { spawnSync } = require('node:child_process');
const ts = require('typescript');

function editionModule(invoke) {
  const source = readFileSync('src/editions/index.ts', 'utf8');
  const compiled = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
  }).outputText;
  const exports = {};
  const vue = {
    ref: value => ({ value }), readonly: value => value,
    computed: getter => ({ get value() { return getter(); } }),
  };
  new Function('require', 'exports', compiled)(name => name === 'vue' ? vue : { invoke }, exports);
  return exports;
}

test('edition is fetched once and pro is unavailable until Rust responds', async () => {
  let complete;
  let calls = 0;
  const module = editionModule(command => {
    assert.equal(command, 'get_edition');
    calls++;
    return new Promise(resolve => { complete = resolve; });
  });
  assert.equal(module.isLite.value, true);
  const pending = module.initializeEdition();
  assert.equal(module.initializeEdition(), pending);
  complete({ edition: 'pro', development: true, warning: null });
  await pending;
  assert.equal(calls, 1);
  assert.equal(module.isLite.value, false);
  assert.equal(module.editionLabel.value, 'プロ版［開発］');
});

test('lite, settings warnings, invalid responses and connection failures stay distinct', async () => {
  const lite = editionModule(async () => ({ edition: 'lite', development: false, warning: '設定エラー' }));
  await lite.initializeEdition();
  assert.equal(lite.isLite.value, true);
  assert.equal(lite.editionLabel.value, 'ライト版');
  assert.equal(lite.editionInfo.value.warning, '設定エラー');
  for (const response of [null, { edition: 'unknown' }]) {
    const module = editionModule(async () => response);
    await module.initializeEdition();
    assert.equal(module.isLite.value, true);
    assert.ok(module.editionError.value);
  }
  const failed = editionModule(async () => { throw new Error('接続失敗'); });
  await failed.initializeEdition();
  assert.equal(failed.isLite.value, true);
  assert.match(failed.editionError.value, /接続失敗/);
});

test('lite launcher forwards args, scopes the override to its child, and returns exit status', () => {
  const directory = mkdtempSync(join(tmpdir(), 'sddirecta-launcher-'));
  const fakeNpm = join(directory, 'npm.cjs');
  writeFileSync(fakeNpm, `console.log(JSON.stringify({args:process.argv.slice(2), edition:process.env.SDDIRECTA_DEV_EDITION})); process.exit(7);`);
  const previous = process.env.SDDIRECTA_DEV_EDITION;
  const result = spawnSync(process.execPath, [resolve('scripts/dev-lite.mjs')], {
    encoding: 'utf8', timeout: 15000,
    env: { ...process.env, npm_execpath: fakeNpm, SDDIRECTA_DEV_EDITION: 'pro' },
  });
  assert.ifError(result.error);
  assert.equal(result.status, 7, result.stderr);
  assert.deepEqual(JSON.parse(result.stdout), { args: ['run', 'tauri', '--', 'dev'], edition: 'lite' });
  assert.equal(process.env.SDDIRECTA_DEV_EDITION, previous);
});
