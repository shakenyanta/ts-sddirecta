const assert = require('node:assert/strict');
const { test } = require('node:test');
const { readFileSync } = require('node:fs');
const ts = require('typescript');
const vue = require('vue');

function load(path, imports = require) {
  const exports = {};
  const code = ts.transpileModule(readFileSync(path, 'utf8'), {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
  }).outputText;
  new Function('require', 'exports', code)(imports, exports);
  return exports;
}
const report = load('src/utils/dailyReport.ts');
const { parseDailyReport, dailyReportRows } = report;

// Coordinates/dates deliberately distinguish boarding, each stop and alighting.
const point = n => Buffer.from([n, 0x35, 0x29, 0x25, 0x34, 0x40, n, 0x17, 0x11, 0x26, 0x08, n]);
function record(stops = 0) {
  const tail = Buffer.alloc(176, 0);
  tail[0] = 2; // 人数, immediately after the final alighting point.
  tail.set([0, 0, 0x12, 0x34], 1); // 料金
  tail[175] = 0x91; // last reserve byte must remain in the displayed reserve.
  return Buffer.concat([Buffer.from([0xce]), point(1),
    ...Array.from({ length: stops }, (_, i) => point(i + 2)), point(9), tail]);
}
function file(records, checksum = [0xfc, 0xe6], padding = 64) {
  return new Uint8Array(Buffer.concat([Buffer.from([0x0b, 0x12, 0x07]), ...records,
    Buffer.from(checksum), Buffer.alloc(padding, 0xff)]));
}

test('daily reports follow CE boundaries for 201/213/225/237 bytes with a variable checksum', () => {
  const bytes = file([record(0), record(1), record(2), record(3)], [0x42, 0x89]);
  const original = bytes.slice();
  const parsed = parseDailyReport(bytes);
  assert.deepEqual(parsed.records, [
    { offset: 3, length: 201, stopCount: 0 },
    { offset: 204, length: 213, stopCount: 1 },
    { offset: 417, length: 225, stopCount: 2 },
    { offset: 642, length: 237, stopCount: 3 },
  ]);
  assert.deepEqual(parsed.warnings, []);
  assert.equal(parsed.checksum, '4289');
  assert.deepEqual(bytes, original);
});

test('stop switching changes only the three alighting rows and keeps the rest aligned', () => {
  for (let count = 0; count <= 3; count++) {
    const bytes = file([record(count)]);
    const parsed = parseDailyReport(bytes);
    assert.equal(parsed.records[0].stopCount, count);
    const normal = dailyReportRows(bytes, parsed.records[0]);
    assert.equal(normal.length, 39);
    assert.equal(normal[0].content, 'CE');
    assert.equal(normal[1].content, '01352925');
    assert.equal(normal[4].content, '09352925');
    assert.equal(normal[3].detail, '2026年11月17日 08時01分');
    assert.equal(normal[6].detail, '2026年11月17日 08時09分');
    assert.equal(normal[7].content, '02');
    assert.equal(normal[8].content, '00001234');
    assert.equal(normal[38].content.length, 168);
    assert.ok(normal[38].content.endsWith('91'));
    for (let selected = 1; selected <= count; selected++) {
      const rows = dailyReportRows(bytes, parsed.records[0], selected);
      assert.deepEqual(rows.filter((_, i) => i < 4 || i > 6), normal.filter((_, i) => i < 4 || i > 6));
      assert.equal(rows[4].name, `立寄${selected}:経度`);
      assert.equal(rows[4].content, `0${selected + 1}352925`);
      assert.equal(rows[5].content, `34400${selected + 1}`);
      assert.equal(rows[6].content, `171126080${selected + 1}`);
      assert.equal(rows[6].detail, `2026年11月17日 08時0${selected + 1}分`);
    }
    assert.throws(() => dailyReportRows(bytes, parsed.records[0], count + 1), /表示位置/);
  }
});

test('date details validate BCD, calendar days and time without changing the raw contents', () => {
  for (const [raw, expected] of [
    ['1711260825', '2026年11月17日 08時25分'],
    ['2902242359', '2024年02月29日 23時59分'],
    ['0101000000', '2000年01月01日 00時00分'],
    ['3112992359', '2099年12月31日 23時59分'],
    ['2902251200', '日時不正'], ['3104261200', '日時不正'],
    ['0011260825', '日時不正'], ['1713260825', '日時不正'],
    ['1711262400', '日時不正'], ['1711260860', '日時不正'],
    ['1A11260825', '日時不正'], ['FFFFFFFFFF', '日時不正'], ['0000000000', '日時不正'],
  ]) {
    const entry = record();
    entry.set(Buffer.from(raw, 'hex'), 8);
    const bytes = file([entry]);
    const rows = dailyReportRows(bytes, parseDailyReport(bytes).records[0]);
    assert.equal(rows[3].detail, expected, raw);
    assert.equal(rows[3].content, raw);
    assert.ok(rows.filter((_, i) => i !== 3 && i !== 6 && i !== 11).every(row => row.detail === ''));
  }
  assert.ok(dailyReportRows(new Uint8Array()).every(row => row.detail === ''));
});

test('payment details use the confirmed hexadecimal codes and stay aligned with all stop counts', () => {
  for (const stops of [0, 1, 2, 3]) {
    for (const [code, expected] of [
      ['01', '現収'], ['09', '合計押無し'], ['11', '回送（メータ操作）'],
      ['19', '置換（メータ操作）'], ['20', '出庫'], ['21', '入庫'],
      ['27', '待機（メータ操作）'], ['28', '貸切（メータ操作）'],
      ['0A', '迎車キャンセル'], ['6F', '交通系IC（スイカ・パスモ）決済'],
      ['E4', 'ネット決済＋現金＋チケット'], ['FF', '未定義'],
    ]) {
      const entry = record(stops);
      // CE + 乗車/立寄/降車 + 人数1 + 料金4 + メータ外料金3 + 現金3。
      entry[36 + stops * 12] = parseInt(code, 16);
      const bytes = file([entry]);
      const parsed = parseDailyReport(bytes);
      for (let selected = 0; selected <= stops; selected++) {
        const rows = dailyReportRows(bytes, parsed.records[0], selected);
        assert.equal(rows[11].name, '■支払区分');
        assert.equal(rows[11].content, code);
        assert.equal(rows[11].detail, expected);
      }
    }
  }
});

test('CE inside ordinary data and checksum values are not used as arbitrary record separators', () => {
  const first = record(2);
  first[40] = 0xce;
  const bytes = file([first, record()], [0xfc, 0xff]);
  const parsed = parseDailyReport(bytes);
  assert.equal(parsed.records.length, 2);
  assert.equal(parsed.records[0].length, 225);
  assert.equal(parsed.checksum, 'FCFF');
  assert.deepEqual(parsed.warnings, []);
  assert.equal(parseDailyReport(file([record()], [0xce, 0x08])).checksum, 'CE08');
});

test('ambiguous boundaries, truncated records and an unknown footer stop without fabricating records', () => {
  const ambiguous = file([record(3), record()]);
  ambiguous[3 + 201] = 0xce;
  assert.equal(parseDailyReport(ambiguous).records.length, 0);
  assert.match(parseDailyReport(ambiguous).warnings[0], /複数/);
  const partial = file([record(), record().subarray(0, 80)]);
  assert.equal(parseDailyReport(partial).records.length, 1);
  assert.ok(parseDailyReport(partial).warnings.length);
  const unknownFooter = file([record()], [1, 2, 3, 4]);
  assert.equal(parseDailyReport(unknownFooter).records.length, 0);
  assert.match(parseDailyReport(unknownFooter).warnings[0], /確定できません/);
  assert.throws(() => parseDailyReport(new Uint8Array(2)), /短すぎ/);
  assert.throws(() => parseDailyReport(new Uint8Array(300)), /CEがありません/);
});

test('file end without a checksum is explicitly reported; grid contains only data fields', () => {
  const bytes = file([record(1)], [], 0);
  const parsed = parseDailyReport(bytes);
  assert.equal(parsed.records.length, 1);
  assert.equal(parsed.checksum, null);
  assert.match(parsed.warnings[0], /チェックサム領域がありません/);
  assert.equal(report.dailyReportBaseLength, 201);
  assert.ok(dailyReportRows(new Uint8Array()).every(row => row.content === ''));
});

function component(path, imports) {
  const { parse, compileScript } = require('@vue/compiler-sfc');
  const { descriptor } = parse(readFileSync(path, 'utf8'));
  const code = compileScript(descriptor, { id: path, inlineTemplate: true }).content;
  const exports = {};
  new Function('require', 'exports', ts.transpileModule(code, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
  }).outputText)(imports, exports);
  return exports.default;
}
const ArrowButton = component('src/components/ArrowButton.vue', require);
const gridResize = load('src/utils/gridColumnResize.ts');
const Grid = component('src/components/DailyReportGrid.vue', name =>
  name === './ArrowButton.vue' ? { default: ArrowButton } :
  name === '../utils/gridColumnResize' ? gridResize : require(name));

test('report grid keeps five columns and sequential numbers across stop changes', async () => {
  const { renderToString } = require('@vue/server-renderer');
  const bytes = file([record(3)]);
  const parsed = parseDailyReport(bytes);
  for (const stopIndex of [0, 1, 2, 3]) {
    const html = (await renderToString(vue.createSSRApp(Grid, {
      rows: dailyReportRows(bytes, parsed.records[0], stopIndex), stopIndex, stopCount: 3,
    }))).replace(/<!--[\s\S]*?-->/g, '');
    const rows = [...html.matchAll(/<tr\b[^>]*>(.*?)<\/tr>/g)].slice(1).map(match => match[1]);
    assert.equal(rows.length, 39);
    rows.forEach((row, i) => assert.ok(row.startsWith(`<td>${i + 1}</td>`)));
    assert.equal((html.match(/<th\b/g) || []).length, 5);
    rows.forEach((row, index) => {
      assert.equal((row.match(/<td\b/g) || []).length, 5);
      if (index === 3 || index === 6) assert.match(row, /2026年11月17日 08時0\d分<\/td>$/);
      else if (index === 11) assert.match(row, /カード決済<\/td>$/);
      else assert.ok(row.endsWith('<td></td>'));
    });
    const buttons = [...html.matchAll(/<button\b[^>]*>/g)].map(match => match[0]);
    assert.equal(buttons.length, 2);
    assert.equal(buttons[0].includes('disabled'), stopIndex === 0);
    assert.equal(buttons[1].includes('disabled'), stopIndex === 3);
    assert.match(html, /aria-label="次の立寄を表示"/);
  }
});

function view(invoke, pro = true) {
  const source = readFileSync('src/pages/DailyReportView.vue', 'utf8')
    .match(/<script setup lang="ts">([\s\S]*?)<\/script>/)[1].replace(/^import .*;\r?$/gm, '');
  let mount;
  const bindings = { ...vue, ...report, invoke, initializeEdition: async () => {},
    features: vue.ref({ optionPanel: pro }), defineProps: () => ({ filename: 'FILE-AT.001' }),
    onMounted: fn => { mount = fn; }, console: { warn: () => {} },
  };
  const code = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;
  const ui = new Function(...Object.keys(bindings), code + '\nreturn { report, rows, loading, error, recordIndex, stopIndex, changeRecord, changeStop };')(...Object.values(bindings));
  return { ui, mount: () => mount() };
}

test('analysis view loads once, resets stops on sales navigation, and guards navigation bounds', async () => {
  const calls = [];
  const { ui, mount } = view(async (command, args) => {
    calls.push([command, args]);
    return Array.from(file([record(3), record()]));
  });
  ui.changeRecord(1);
  assert.equal(ui.recordIndex.value, 0);
  await mount();
  ui.changeStop(3);
  assert.equal(ui.rows.value[4].name, '立寄3:経度');
  ui.changeRecord(1);
  assert.equal(ui.stopIndex.value, 0);
  assert.equal(ui.rows.value[4].name, '降車:経度');
  ui.changeStop(1);
  assert.equal(ui.stopIndex.value, 0);
  ui.changeRecord(2);
  assert.equal(ui.recordIndex.value, 1);
  ui.changeRecord(0);
  ui.changeRecord(-1);
  assert.equal(ui.recordIndex.value, 0);
  assert.deepEqual(calls, [['read_daily_report', { filename: 'FILE-AT.001' }]]);
});

test('lite never reads reports, and read failures leave a blank grid with an error', async () => {
  let calls = 0;
  const lite = view(async () => { calls++; }, false);
  await lite.mount();
  assert.equal(calls, 0);
  assert.match(lite.ui.error.value, /プロ版専用/);
  const failed = view(async () => { throw new Error('missing file'); });
  await failed.mount();
  assert.match(failed.ui.error.value, /missing file/);
  assert.equal(failed.ui.loading.value, false);
  assert.ok(failed.ui.rows.value.every(row => !row.content));
});
