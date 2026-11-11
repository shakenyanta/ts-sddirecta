const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { test } = require('node:test');
const ts = require('typescript');
const bindings = {};
new Function('exports', ts.transpileModule(readFileSync('src/utils/dropHistoryState.ts', 'utf8'), {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText)(bindings);
const { parseDropHistoryState } = bindings;
const entry = { id: '00000000-0000-4000-8000-000000000001', droppedAt: 1794375128000,
  paths: ['C:\\案件 A\\FILE-AA.001', '\\\\server\\share\\FILE-BA.001'] };

test('history schema accepts empty history and preserves complete original records', () => {
  for (const entries of [[], [entry]]) {
    const state = { version: 1, entries };
    assert.deepEqual(parseDropHistoryState(JSON.stringify(state)), state);
  }
});

test('history schema rejects unsupported versions, duplicate IDs and invalid dates or paths', () => {
  for (const state of [null, {}, { version: 2, entries: [] }, { version: 1, entries: {} },
    { version: 1, entries: [entry, entry] }]) {
    assert.throws(() => parseDropHistoryState(JSON.stringify(state)));
  }
  for (const change of [{ id: 'bad' }, { droppedAt: null }, { droppedAt: -1 },
    { droppedAt: 8640000000000001 }, { droppedAt: 1.5 }, { paths: [] },
    { paths: [''] }, { paths: ['bad\0path'] }, { paths: [123] }, { paths: 'C:\\file' }]) {
    assert.throws(() => parseDropHistoryState(JSON.stringify({ version: 1, entries: [{ ...entry, ...change }] })));
  }
  assert.throws(() => parseDropHistoryState('{broken'));
});

test('history schema accepts legacy and boolean favorite states but rejects malformed favorites', () => {
  for (const favorite of [undefined, false, true]) {
    const state = { version: 1, entries: [{ ...entry, ...(favorite === undefined ? {} : { favorite }) }] };
    const parsed = parseDropHistoryState(JSON.stringify(state));
    assert.deepEqual(parsed, state);
    assert.equal(!!parsed.entries[0].favorite, favorite === true);
  }
  for (const favorite of [null, 0, 1, 'true', {}, []]) {
    assert.throws(() => parseDropHistoryState(JSON.stringify({ version: 1, entries: [{ ...entry, favorite }] })));
  }
});
