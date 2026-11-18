const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { test } = require('node:test');
const ts = require('typescript');
const bindings = {};
new Function('exports', ts.transpileModule(readFileSync('src/utils/historyPath.ts', 'utf8'), {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText)(bindings);
const { defaultHistoryPathRules: defaults, historyDisplayPath: display, validateHistoryPathRules: validate, parseHistoryPathRules: parse } = bindings;
const base = defaults[0].folder;

test('history paths label complete descendants and leave unrelated locations intact', () => {
  assert.deepEqual(display(base + '\\案件 A\\FILE-AA.001'), { label: '共有', path: '案件 A\\FILE-AA.001' });
  assert.deepEqual(display(base.toUpperCase() + '\\案件 B'), { label: '共有', path: '案件 B' });
  assert.deepEqual(display(base.replaceAll('\\', '/') + '/案件 C/FILE-BA.001'), { label: '共有', path: '案件 C/FILE-BA.001' });
  assert.deepEqual(display(base), { label: '共有', path: '.' });
  assert.deepEqual(display(base + '\\'), { label: '共有', path: '.' });
  assert.deepEqual(display(String.raw`C:\Users\SYSTEM-PC7\source\ts02`), { label: 'ローカル', path: String.raw`source\ts02` });
  for (const path of [base + '旧\\案件 A', String.raw`\\130.2.1.174\disk\保存ファイル\案件 A`,
    String.raw`\\130.2.1.173\disk`, String.raw`C:\保存ファイル\案件 A`, '', '案件 A']) {
    assert.deepEqual(display(path), { label: '', path });
  }
});

test('deepest rule wins in either order, with drive roots and disabling supported', () => {
  const rules = validate([{ label: 'ドライブ', folder: 'C:\\' }, { label: '作業', folder: 'C:/work/' }]);
  for (const ordered of [rules, [...rules].reverse()]) {
    assert.deepEqual(display('c:\\WORK\\日本語\\file', ordered), { label: '作業', path: '日本語\\file' });
    assert.deepEqual(display('C:\\elsewhere', ordered), { label: 'ドライブ', path: 'elsewhere' });
    assert.deepEqual(display('C:\\', ordered), { label: 'ドライブ', path: '.' });
  }
  assert.deepEqual(display(base, []), { label: '', path: base });
  assert.deepEqual(parse(JSON.stringify({ version: 1, rules })), rules);
  assert.deepEqual(parse('{"version":1,"rules":[]}'), []);
});

test('rule validation rejects malformed, ambiguous or relative paths without modifying the draft', () => {
  const valid = { label: '作業', folder: 'C:\\work' };
  for (const folder of ['', 'work', 'C:', '\\\\server', 'C:\\work\\..\\other', 'C:\\work\\.\\other',
    'C:\\a*', 'C:\\a:b', 'C:\\work\\\\other']) assert.throws(() => validate([{ ...valid, folder }]));
  for (const label of ['', '[作業]', '改\n行']) assert.throws(() => validate([{ ...valid, label }]));
  for (const rules of [null, {}, [null], [{ label: 1, folder: 'C:\\' }], [valid, { ...valid, label: '別名', folder: 'c:/WORK/' }],
    [valid, { ...valid, folder: 'D:\\work' }]]) assert.throws(() => validate(rules));
  for (const raw of ['{', 'null', '{"version":2,"rules":[]}', '{"version":1}']) assert.throws(() => parse(raw));
  assert.deepEqual(validate(defaults), defaults);
  const draft = [{ label: '  作業  ', folder: 'C:/work/' }];
  assert.deepEqual(validate(draft), [valid]);
  assert.equal(draft[0].folder, 'C:/work/');
});
