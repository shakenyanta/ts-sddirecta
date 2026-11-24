const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { test } = require('node:test');
const ts = require('typescript');
const bindings = {};
new Function('exports', ts.transpileModule(readFileSync('src/utils/fileIconCache.ts', 'utf8'), {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText)(bindings);
const { FileIconCache } = bindings;
const file = { filename: 'App.exe', modifiedAt: 2000, size: 512 };

test('icons use full Windows paths and normalize equivalent drive and UNC spellings', () => {
  const cache = new FileIconCache();
  cache.set('C:\\app', file, 'drive');
  assert.equal(cache.get('c:/APP/', { ...file, filename: 'app.EXE' }), 'drive');
  assert.equal(cache.get(String.raw`\\?\C:\app`, file), 'drive');
  assert.equal(cache.get('D:\\app', file), undefined);
  assert.equal(cache.get('C:\\app\\input', file), undefined);
  cache.set(String.raw`\\?\UNC\server\share`, file, 'unc');
  assert.equal(cache.get(String.raw`\\SERVER\share` + '\\', file), 'unc');
});

test('changes to either timestamp or size invalidate icons and unknown metadata is not cached', () => {
  const cache = new FileIconCache();
  for (const changed of [{ modifiedAt: 3000 }, { size: 1024 }, { size: null }, { modifiedAt: null }]) {
    cache.set('C:\\app', file, 'old');
    assert.equal(cache.get('C:\\app', { ...file, ...changed }), undefined);
    assert.equal(cache.get('C:\\app', file), undefined);
  }
  for (const unknown of [{ ...file, modifiedAt: null }, { ...file, size: null }]) {
    cache.set('C:\\app', unknown, 'unknown');
    assert.equal(cache.get('C:\\app', unknown), undefined);
  }
  cache.set('C:\\app', file, '');
  assert.equal(cache.get('C:\\app', file), undefined);
});

test('cache evicts the least recently used entry and replaces old versions of a path', () => {
  const cache = new FileIconCache(2);
  const second = { ...file, filename: 'second.exe' };
  const third = { ...file, filename: 'third.exe' };
  cache.set('C:\\app', file, 'one');
  cache.set('C:\\app', second, 'two');
  assert.equal(cache.get('C:\\app', file), 'one');
  cache.set('C:\\app', third, 'three');
  assert.equal(cache.get('C:\\app', second), undefined);
  cache.set('C:\\app', { ...file, size: 1024 }, 'updated');
  assert.equal(cache.get('C:\\app', { ...file, size: 1024 }), 'updated');
  assert.equal(cache.get('C:\\app', third), 'three');
});
