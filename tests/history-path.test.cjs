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

test('fixed folders use the requested labels and colors and replace legacy duplicates', () => {
  const { fixedHistoryPathRules: fixed, editableHistoryPathRules: editable, historyPathsColorStyle: style } = bindings;
  assert.deepEqual(fixed.map(rule => [rule.label, rule.color]), [
    ['TS-02各社', 'blue'], ['TS-01各社', 'green'], ['TS-02開発', 'orange'], ['TS-01開発', 'purple'],
  ]);
  assert.deepEqual(validate(fixed), fixed);
  for (const rule of fixed) {
    assert.deepEqual(display(rule.folder + '\\file.hex'), { label: rule.label, path: 'file.hex' });
    assert.deepEqual(style([rule.folder + '\\file.hex'], [...fixed, ...defaults]), bindings.historyPathColorStyle(rule.color));
    const legacy = { label: '旧名', folder: rule.folder.toUpperCase().replaceAll('\\', '/') + '/', color: 'red' };
    assert.deepEqual(editable([legacy, ...defaults]), defaults);
  }
});

test('eight color presets persist and old rules remain compatible', () => {
  const { historyPathColors: colors, historyPathColorStyle: style, historyPathsColorStyle: pathsStyle } = bindings;
  assert.equal(colors.length, 8);
  assert.equal(new Set(colors.map(color => color.id)).size, 8);
  for (const color of colors) {
    const rules = validate([{ label: '作業', folder: 'C:\\work', color: color.id }]);
    assert.deepEqual(parse(JSON.stringify({ version: 1, rules })), rules);
    assert.deepEqual(pathsStyle(['C:\\work\\file'], rules), { color: color.color, backgroundColor: color.backgroundColor });
  }
  assert.deepEqual(pathsStyle([base], defaults), style('blue'));
  for (const color of ['invalid', '', null, 1, '#fff']) {
    assert.throws(() => validate([{ label: '作業', folder: 'C:\\work', color }]));
  }
});

test('no color persists and overrides a colored parent without changing shortening', () => {
  const { historyPathColorStyle: style, historyPathsColorStyle: pathsStyle } = bindings;
  const rules = validate([
    { label: '親', folder: 'C:\\work', color: 'green' },
    { label: '子', folder: 'C:\\work\\child', color: 'none' },
  ]);
  const restored = parse(JSON.stringify({ version: 1, rules }));
  assert.deepEqual(restored, rules);
  assert.deepEqual(style('none'), {});
  assert.deepEqual(pathsStyle(['C:\\work\\child\\file'], restored), {});
  assert.deepEqual(pathsStyle(['C:\\work\\child\\a', 'C:\\work\\child\\b'], restored), {});
  assert.deepEqual(pathsStyle(['C:\\work\\a', 'C:\\work\\child\\b'], restored), {});
  assert.deepEqual(pathsStyle(['C:\\work\\a'], restored), style('green'));
  assert.deepEqual(display('C:\\work\\child\\file', restored), { label: '子', path: 'file' });
});

test('row colors use the deepest rule and avoid coloring mixed or unrelated entries', () => {
  const { historyPathColorStyle: style, historyPathsColorStyle: pathsStyle } = bindings;
  const rules = validate([
    { label: '親', folder: 'C:\\work', color: 'green' },
    { label: '子', folder: 'C:\\work\\child', color: 'red' },
  ]);
  assert.deepEqual(pathsStyle(['c:/WORK/child/file'], rules), style('red'));
  assert.deepEqual(pathsStyle(['C:\\work\\a', 'C:\\work\\b'], rules), style('green'));
  assert.deepEqual(pathsStyle(['C:\\work\\a', 'C:\\work\\child\\b'], rules), {});
  assert.deepEqual(pathsStyle(['C:\\work\\a', 'D:\\other'], rules), {});
  assert.deepEqual(pathsStyle(['D:\\other'], rules), {});
  assert.deepEqual(pathsStyle([], rules), {});
});

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

  assert.throws(() => validate([{ label: '作業', folder: '' }], 4), /5行目/);
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
