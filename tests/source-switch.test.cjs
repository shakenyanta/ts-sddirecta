const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { test } = require('node:test');
const ts = require('typescript');

function transpile(source) {
  return ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS } }).outputText;
}

function utility(path) {
  const exports = {};
  new Function('exports', transpile(readFileSync(path, 'utf8')))(exports);
  return exports;
}

// Exercise the actual view setup with IPC/lifecycle boundaries replaced; no WebView or SD access.
function view(invoke) {
  const source = readFileSync('src/pages/HomeView.vue', 'utf8').match(/<script setup lang="ts">([\s\S]*?)<\/script>/)[1]
    .replace(/^import .*;\r?$/gm, '');
  let mount, unmount, listener, timer, unlistened = false;
  const eventListeners = new Map();
  const keyListeners = new Set();
  const bindings = {
    ref: value => ({ value }),
    computed: get => ({ get value() { return get(); } }),
    nextTick: async () => {},
    onMounted: fn => { mount = fn; },
    onUnmounted: fn => { unmount = fn; },
    getCurrentWebview: () => ({ onDragDropEvent: async fn => { listener = fn; return () => { unlistened = true; }; } }),
    invoke,
    listen: async (name, fn) => { eventListeners.set(name, fn); return () => eventListeners.delete(name); },
    window: {
      addEventListener: (name, fn) => { if (name === 'keydown') keyListeners.add(fn); },
      removeEventListener: (name, fn) => { if (name === 'keydown') keyListeners.delete(fn); },
    },
    setInterval: fn => { timer = fn; return 1; },
    clearInterval: () => { timer = undefined; },
    ...utility('src/utils/sdDirectCsv.ts'),
    ...utility('src/utils/executablePath.ts'),
    ...utility('src/utils/sdWrite.ts'),
    createLogScroller: getPanel => utility('src/utils/logScroll.ts').createLogScroller(getPanel, {
      request: () => 1, cancel: () => {}, reducedMotion: () => false,
    }),
  };
  const api = new Function(...Object.keys(bindings), transpile(source) + `
    return { initializeSource, refreshMatchedFiles, sourceDirectory, displayedDirectory,
      sdDirectDictionary, sdloadingDictionary, sdAddressRange, matchedFiles, canSdWrite, writeFileCount, allFileCount, initializing, messages, folders, pathError,
      onSdWrite, closeWriteDialog, copyWriteLog, writeDialogOpen, writeLog, writing, pendingPlan, progress, percent, currentRun, transientNotice, operationStatus, operationDetail };
  `)(...Object.values(bindings));
  return { ...api, mount: () => mount(), unmount: () => unmount(),
    drop: paths => listener({ payload: { type: 'drop', paths } }),
    event: (name, payload) => eventListeners.get(name)?.({ payload }),
    key: (key, repeat = false) => {
      let prevented = false;
      for (const fn of keyListeners) fn({ key, repeat, preventDefault: () => { prevented = true; }, stopImmediatePropagation: () => {} });
      return prevented;
    },
    get hasTimer() { return !!timer; },
    tick: () => timer?.(), get unlistened() { return unlistened; } };
}

const csv = 'A,FILE-AA.001,00000000,000001FF';

test('blocked operation details stay visible and final verification failure never reports success', async () => {
  let ui, release, runId;
  const waiting = new Promise(resolve => { release = resolve; });
  ui = view(backend({
    prepare_sd_write: args => ({ run_id: args.runId, drive: 'H:', total_bytes: 1024, entries: ['first', 'second'].map(filename => ({ filename, size: 512, file_size: 512 })) }),
    run_sd_write: args => { runId = args.runId; return waiting; },
    discard_sd_write: () => {},
  }));
  await ui.mount();
  const task = ui.onSdWrite('H:');
  await until(() => !!runId);
  const status = { run_id: runId, operation_id: 'worker-1', detail: 'H:を開いています', operation_ms: 7000, waiting: true };
  ui.event('sd-operation', { ...status, run_id: 'stale' });
  assert.equal(ui.operationStatus.value, null);
  ui.event('sd-operation', status);
  assert.match(ui.operationDetail.value, /H:を開いています.*デバイス応答待ち・7秒経過/);
  const progress = { run_id: runId, filename: 'first', index: 1, total: 2, completed: 0, stage: 'final_verify', processed_bytes: 2048, total_bytes: 3072, elapsed_ms: 7100, file_ms: 0, error: null };
  ui.event('sd-progress', progress);
  assert.equal(ui.percent.value, 66);
  assert.equal(ui.operationStatus.value, null);
  ui.event('sd-progress', { ...progress, stage: 'final_failed', error: '照合不一致' });
  ui.event('sd-operation', status);
  assert.equal(ui.operationStatus.value, null);
  release({ ...progress, stage: 'done', processed_bytes: 3072 });
  await task;
  assert.notEqual(ui.percent.value, 100);
  const logs = ui.writeLog.value.map(m => typeof m === 'string' ? m : m.text);
  assert.ok(logs.some(m => m.includes('最終照合失敗')));
  assert.ok(logs.includes('最終照合未完了: first, second'));
  assert.ok(logs.every(m => !m.startsWith('全件完了')));
  ui.unmount();
});
test('extension warnings retain actual filenames, stay separate from write matches, and report warning changes', async () => {
  let files = ['FILE-AA.001'];
  let warning_files = ['FILE-AA.bin', 'input\\FILE-AA.002'];
  const ui = view(backend({ find_dictionary_files: () => ({ files, warning_files, from_input: false, input_exists: true }) }));
  await ui.initializeSource();
  const warnings = () => ui.messages.value.filter(message => typeof message !== 'string');
  assert.deepEqual(warnings(), warning_files.map(filename => ({ text: `警告：拡張子が異なるファイルがあります：${filename}`, warning: true })));
  assert.deepEqual([...ui.sdloadingDictionary.value.keys()], ['FILE-AA.001']);
  await ui.refreshMatchedFiles();
  assert.equal(warnings().length, 2);
  warning_files = ['FILE-AA.txt'];
  await ui.refreshMatchedFiles();
  assert.match(warnings().at(-1).text, /FILE-AA\.txt$/);
  files = [];
  await ui.refreshMatchedFiles();
  assert.equal(ui.canSdWrite.value, false);
  assert.deepEqual(ui.matchedFiles.value, []);
  assert.equal(warnings().length, 3);
});

test('CSV completion log includes the source folder and preserves the dictionary count', async () => {
  for (const source_folder of ['EXEフォルダ', 'configフォルダ', '対象フォルダ', '内蔵CSV']) {
    const ui = view(backend({ read_startup_csv: () => ({ contents: csv, source_folder }) }));
    await ui.initializeSource();
    const index = ui.messages.value.indexOf(`sdDirectaxs.csv読込完了 (${source_folder})`);
    assert.ok(index >= 0);
    assert.equal(ui.messages.value[index + 1], 'dictionary作成：1件');
  }
});

test('matched dictionary retains only matching keys and their ranges and resets on empty results or errors', async () => {
  let files = ['FILE-AA.001'];
  let fail = false;
  const ui = view(backend({
    read_startup_csv: () => csv + '\nW,FILE-AB.001,00000200,000003FF\nE\nA,IGNORED.001,0,FF',
    find_dictionary_files: () => {
      if (fail) throw new Error('cannot list folder');
      return { files, from_input: false, input_exists: false };
    },
  }));
  await ui.initializeSource();
  assert.equal(ui.sdDirectDictionary.value.size, 2);
  assert.deepEqual([...ui.sdloadingDictionary.value], [
    ['FILE-AA.001', { accessFlag: 'A', startAddress: '00000000', endAddress: '000001FF' }],
  ]);
  assert.notEqual(ui.sdloadingDictionary.value.get('FILE-AA.001'), ui.sdDirectDictionary.value.get('FILE-AA.001'));
  files = ['FILE-AB.001'];
  await ui.refreshMatchedFiles();
  assert.deepEqual([...ui.sdloadingDictionary.value], [
    ['FILE-AB.001', { accessFlag: 'W', startAddress: '00000200', endAddress: '000003FF' }],
  ]);
  files = [];
  await ui.refreshMatchedFiles();
  assert.equal(ui.sdloadingDictionary.value.size, 0);
  assert.equal(ui.sdDirectDictionary.value.size, 2);
  assert.equal(ui.canSdWrite.value, false);
  files = ['FILE-AA.001'];
  await ui.refreshMatchedFiles();
  fail = true;
  await ui.refreshMatchedFiles();
  assert.equal(ui.sdloadingDictionary.value.size, 0);
  assert.equal(ui.sdDirectDictionary.value.size, 2);
  assert.equal(ui.canSdWrite.value, false);
});

test('all-area write remains available with only R/N/0 matches and keeps selected and total counts separate', async () => {
  const flags = ['A', 'R', 'W', 'N', '0'];
  let files = ['FILE-R.001', 'FILE-N.001', 'FILE-0.001'];
  const ui = view(backend({
    read_startup_csv: () => flags.map(flag => `${flag},FILE-${flag}.001,00000000,000001FF`).join('\n')
      + '\nE\nA,IGNORED.001,0,FFFFFFFF',
    find_dictionary_files: ({ filenames }) => {
      assert.deepEqual(filenames, flags.map(flag => `FILE-${flag}.001`));
      return { files, from_input: false, input_exists: false };
    },
  }));
  await ui.initializeSource();
  assert.deepEqual([...ui.sdloadingDictionary.value.values()].map(area => area.accessFlag), ['R', 'N', '0']);
  assert.equal(ui.writeFileCount.value, 0);
  assert.equal(ui.allFileCount.value, 3);
  assert.equal(ui.canSdWrite.value, true);
  assert.equal(ui.sdDirectDictionary.value.size, 5);
  assert.deepEqual(ui.sdAddressRange.value, { startAddress: '0x00000000', endAddress: '0x000001FF' });
  assert.ok(ui.messages.value.includes('dictionary作成：5件'));
  files = flags.map(flag => `FILE-${flag}.001`);
  await ui.refreshMatchedFiles();
  assert.equal(ui.writeFileCount.value, 2);
  assert.equal(ui.allFileCount.value, 5);
  files = [];
  await ui.refreshMatchedFiles();
  assert.equal(ui.sdloadingDictionary.value.size, 0);
  assert.deepEqual([...ui.sdDirectDictionary.value.values()].map(area => area.accessFlag), flags);
  assert.equal(ui.writeFileCount.value, 0);
  assert.equal(ui.allFileCount.value, 0);
  assert.equal(ui.canSdWrite.value, false);
});

test('CSV builds the dictionary and numeric hexadecimal bounds in one pass', () => {
  const { parseSdDirectCsv } = utility('src/utils/sdDirectCsv.ts');
  const parsed = parseSdDirectCsv([
    'A,HIGH.001,0x100,0x07baffff',
    'A,LOW.001,0,ff',
    'A,MIDDLE.001,F,1000',
    'E,EEEEEEEEEEEE,EEEEEEEE EEEEEEEE,',
    'A,IGNORED.001,0,FFFFFFFF',
  ].join('\n'));
  assert.equal(parsed.dictionary.size, 3);
  assert.deepEqual(parsed.dictionary.get('HIGH.001'), { accessFlag: 'A', startAddress: '0x100', endAddress: '0x07baffff' });
  assert.deepEqual(parsed.addressRange, { startAddress: '0x00000000', endAddress: '0x07BAFFFF' });
  assert.equal(parseSdDirectCsv('').addressRange, null);
  assert.throws(() => parseSdDirectCsv('A,BAD.001,invalid,FF'), /16進数/);
});

test('CSV preserves every access flag and includes non-A areas in the bounds', () => {
  const { parseSdDirectCsv } = utility('src/utils/sdDirectCsv.ts');
  const flags = ['A', 'R', 'W', 'N', '0', 'X'];
  const parsed = parseSdDirectCsv([
    '',
    ...flags.map((flag, index) => ` ${flag} , FILE-${flag}.001 , ${index === 0 ? '10' : '00'} , ${index === 4 ? 'FFFF' : 'FF'} `),
    '   ',
    'E',
    'A,AFTER-END.001,0,FFFFFFFF',
    'W,incomplete',
  ].join('\r\n'));
  assert.deepEqual([...parsed.dictionary.values()].map(area => area.accessFlag), flags);
  assert.equal(parsed.dictionary.size, flags.length);
  assert.deepEqual(parsed.addressRange, { startAddress: '0x00000000', endAddress: '0x0000FFFF' });
  assert.deepEqual(parseSdDirectCsv('E\nA,IGNORED.001,0,FF'), { dictionary: new Map(), addressRange: null });
  for (const flag of flags) {
    assert.throws(() => parseSdDirectCsv(`${flag},incomplete`), /ファイル名・開始アドレス・終了アドレス/);
    assert.throws(() => parseSdDirectCsv(`${flag},BAD.001,invalid,FF`), /16進数/);
  }
});

function backend(overrides = {}) {
  return async (command, args) => {
    if (overrides[command]) {
      const result = await overrides[command](args);
      if (command === 'find_dictionary_files') return { warning_files: [], ...result };
      return command === 'read_startup_csv' && typeof result === 'string'
        ? { contents: result, source_folder: 'EXEフォルダ' } : result;
    }
    switch (command) {
      case 'get_executable_directory': return 'C:\\app';
      case 'resolve_drop_directory': return args.paths[0];
      case 'check_startup_csv': return true;
      case 'read_startup_csv': return { contents: csv, source_folder: 'EXEフォルダ' };
      case 'find_dictionary_files': return { files: ['FILE-AA.001'], warning_files: [], from_input: true, input_exists: true };
      default: throw new Error(command);
    }
  };
}

test('startup and native drop share initialization and use the selected source for all IPC', async () => {
  const calls = [];
  const run = backend();
  const ui = view(async (command, args) => { calls.push([command, args]); return run(command, args); });
  await ui.mount();
  assert.deepEqual(ui.sdAddressRange.value, { startAddress: '0x00000000', endAddress: '0x000001FF' });
  assert.equal(ui.displayedDirectory.value, 'C:\\app\\input');
  ui.drop(['C:\\日本語 folder']);
  for (let i = 0; i < 10 && ui.initializing.value; i++) await new Promise(setImmediate);
  assert.equal(ui.displayedDirectory.value, 'C:\\日本語 folder\\input');
  assert.deepEqual(ui.matchedFiles.value, ['FILE-AA.001']);
  assert.equal(ui.sdDirectDictionary.value.size, 1);
  assert.equal(ui.canSdWrite.value, true);
  assert.deepEqual(ui.folders.value, ['C:\\', '日本語 folder', 'input']);
  for (const command of ['check_startup_csv', 'read_startup_csv', 'find_dictionary_files']) {
    assert.equal(calls.filter(call => call[0] === command).at(-1)[1].directory, 'C:\\日本語 folder');
  }
  ui.unmount();
  assert.equal(ui.unlistened, true);
});

test('missing or malformed CSV clears previous dictionary, matches, and write availability', async () => {
  const ui = view(backend({
    check_startup_csv: ({ directory }) => directory !== 'C:\\missing',
    read_startup_csv: ({ directory }) => directory === 'C:\\bad' ? 'A,incomplete' : csv,
  }));
  await ui.initializeSource(['C:\\valid']);
  assert.equal(ui.canSdWrite.value, true);
  await ui.initializeSource(['C:\\missing']);
  assert.equal(ui.sdAddressRange.value, null);
  assert.equal(ui.sdDirectDictionary.value.size, 0);
  assert.equal(ui.sdloadingDictionary.value.size, 0);
  assert.deepEqual(ui.matchedFiles.value, []);
  assert.equal(ui.canSdWrite.value, false);
  await ui.initializeSource(['C:\\valid']);
  await ui.initializeSource(['C:\\bad']);
  assert.equal(ui.sdAddressRange.value, null);
  assert.equal(ui.sdDirectDictionary.value.size, 0);
  assert.equal(ui.sdloadingDictionary.value.size, 0);
  assert.equal(ui.canSdWrite.value, false);
});

test('old CSV response cannot replace a newer dropped source', async () => {
  let release;
  const waiting = new Promise(resolve => { release = resolve; });
  const ui = view(backend({ read_startup_csv: ({ directory }) => directory === 'C:\\old' ? waiting : csv }));
  const old = ui.initializeSource(['C:\\old']);
  await new Promise(setImmediate);
  await ui.initializeSource(['C:\\new']);
  release('A,OLD.001,0,1');
  await old;
  assert.equal(ui.sourceDirectory.value, 'C:\\new');
  assert.deepEqual([...ui.sdDirectDictionary.value.keys()], ['FILE-AA.001']);
  assert.deepEqual(ui.sdAddressRange.value, { startAddress: '0x00000000', endAddress: '0x000001FF' });
  assert.equal(ui.initializing.value, false);
});

test('old matching response cannot replace matches or input state after switching source', async () => {
  let release, delay = false;
  const waiting = new Promise(resolve => { release = resolve; });
  const ui = view(backend({
    find_dictionary_files: ({ directory }) => directory === 'C:\\old' && delay ? waiting
      : { files: ['FILE-AA.001'], from_input: false, input_exists: false },
  }));
  await ui.initializeSource(['C:\\old']);
  delay = true;
  const old = ui.refreshMatchedFiles();
  await ui.initializeSource(['C:\\new']);
  release({ files: ['OLD.001'], from_input: true, input_exists: true });
  await old;
  assert.equal(ui.displayedDirectory.value, 'C:\\new');
  assert.deepEqual(ui.matchedFiles.value, ['FILE-AA.001']);
});

test('drop resolution failure clears old write state and reports the error', async () => {
  const ui = view(backend({ resolve_drop_directory: () => { throw new Error('different folders'); } }));
  await ui.initializeSource();
  assert.equal(ui.canSdWrite.value, true);
  await ui.initializeSource(['C:\\one', 'C:\\two']);
  assert.equal(ui.canSdWrite.value, false);
  assert.deepEqual(ui.matchedFiles.value, []);
  assert.match(ui.pathError.value, /different folders/);
});

test('F5 reloads CSV and matches in the selected source without a polling timer or page reload', async () => {
  let updated = false;
  const calls = [];
  const run = backend({
    read_startup_csv: () => updated ? 'A,NEW.001,0,1' : csv,
    find_dictionary_files: () => ({ files: [updated ? 'NEW.001' : 'FILE-AA.001'], from_input: true, input_exists: true }),
  });
  const ui = view(async (command, args) => { calls.push([command, args]); return run(command, args); });
  await ui.mount();
  ui.drop(['C:\\selected']);
  for (let i = 0; i < 10 && ui.initializing.value; i++) await new Promise(setImmediate);
  assert.equal(ui.displayedDirectory.value, 'C:\\selected\\input');
  assert.equal(ui.hasTimer, false);
  const before = calls.length;
  updated = true;
  assert.equal(ui.key('F5'), true);
  assert.equal(ui.canSdWrite.value, false);
  for (let i = 0; i < 10 && ui.initializing.value; i++) await new Promise(setImmediate);
  assert.equal(ui.sourceDirectory.value, 'C:\\selected');
  assert.equal(ui.displayedDirectory.value, 'C:\\selected\\input');
  assert.deepEqual([...ui.sdDirectDictionary.value.keys()], ['NEW.001']);
  assert.deepEqual(ui.sdAddressRange.value, { startAddress: '0x00000000', endAddress: '0x00000001' });
  assert.deepEqual(ui.matchedFiles.value, ['NEW.001']);
  assert.equal(ui.canSdWrite.value, true);
  assert.deepEqual(calls.slice(before).map(([command]) => command), ['check_startup_csv', 'read_startup_csv', 'find_dictionary_files']);
  for (const [, args] of calls.slice(before)) assert.equal(args.directory, 'C:\\selected');
  ui.unmount();
  assert.equal(ui.key('F5'), false);
});

test('F5 prevents default reload while busy and ignores key repeats', async () => {
  let release, delay = false, reads = 0;
  const waiting = new Promise(resolve => { release = resolve; });
  const ui = view(backend({ read_startup_csv: () => { reads++; return delay ? waiting : csv; } }));
  await ui.mount();
  assert.equal(ui.key('F4'), false);
  assert.equal(ui.key('F5', true), true);
  assert.equal(reads, 1);
  delay = true;
  assert.equal(ui.key('F5'), true);
  await new Promise(setImmediate);
  assert.equal(ui.key('F5'), true);
  assert.equal(reads, 2);
  release(csv);
  for (let i = 0; i < 10 && ui.initializing.value; i++) await new Promise(setImmediate);
  assert.equal(ui.sourceDirectory.value, 'C:\\app');
  ui.unmount();
});

async function until(check) {
  for (let i = 0; i < 30 && !check(); i++) await new Promise(setImmediate);
  assert.ok(check(), 'expected state was not reached');
}

test('write workflow opens progress immediately, blocks target changes, rejects stale events, and retains results', async () => {
  let ui, runArgs, release;
  const waiting = new Promise(resolve => { release = resolve; });
  const entry = { filename: 'FILE-AA.001', address: 0, size: 512, file_size: 768 };
  const calls = [];
  ui = view(backend({
    find_dictionary_files: () => ({ files: ['FILE-AA.001'], warning_files: ['input\\IM.016'], from_input: true, input_exists: true }),
    prepare_sd_write: args => ({ run_id: args.runId, drive: args.drive, directory: args.directory, capacity: 2048, total_bytes: 512, entries: [entry] }),
    run_sd_write: args => { runArgs = args; calls.push('run'); return waiting; },
    discard_sd_write: () => { calls.push('discard'); },
  }));
  await ui.mount();
  const task = ui.onSdWrite('E:');
  await until(() => !!runArgs);
  assert.equal(ui.writing.value, true);
  assert.equal(ui.canSdWrite.value, false);
  assert.equal(ui.writeDialogOpen.value, true);
  ui.closeWriteDialog();
  assert.equal(ui.writeDialogOpen.value, true);
  const source = ui.sourceDirectory.value;
  ui.drop(['C:\\other']);
  assert.equal(ui.sourceDirectory.value, source);
  assert.equal(ui.key('F5'), true);
  assert.match(ui.transientNotice.value, /再読込できません/);
  await ui.onSdWrite('E:');
  assert.deepEqual(calls, ['run']);
  const event = { run_id: runArgs.runId, filename: entry.filename, index: 1, total: 1, completed: 0, stage: 'verify', processed_bytes: 512, total_bytes: 1024, elapsed_ms: 100, file_ms: 100, error: null };
  ui.event('sd-progress', { ...event, run_id: 'stale', stage: 'done' });
  assert.equal(ui.progress.value, null);
  ui.event('sd-progress', event);
  assert.equal(ui.percent.value, 50);
  ui.event('sd-progress', { ...event, stage: 'file_done', completed: 1, processed_bytes: 1024, file_ms: 253 });
  assert.equal(ui.percent.value, 99);
  const done = { ...event, stage: 'done', filename: '', completed: 1, processed_bytes: 1024, elapsed_ms: 2540 };
  ui.event('sd-progress', done);
  release(done);
  await task;
  assert.equal(ui.writing.value, false);
  assert.equal(ui.percent.value, 100);
  assert.equal(ui.writeDialogOpen.value, true);
  assert.ok(ui.writeLog.value.some(m => typeof m === 'string' && m.startsWith('全件完了')));
  ui.closeWriteDialog();
  assert.equal(ui.writeDialogOpen.value, false);
  assert.equal(ui.canSdWrite.value, true);
  assert.equal(ui.hasTimer, false);
  assert.equal(ui.writeLog.value.filter(m => typeof m === 'string' && m.startsWith('全件完了')).length, 1);
  assert.ok(ui.writeLog.value.some(m => typeof m === 'object' && /末尾256\(0x100\)バイトを無視/.test(m.text)));
  assert.ok(ui.writeLog.value.some(m => typeof m === 'string' && m === '1/1 FILE-AA.001 253ms ✓'));
  assert.ok(ui.writeLog.value.some(m => typeof m === 'string' && m === '全件完了 1/1 2.54秒'));
  const textOf = m => typeof m === 'string' ? m : m.text;
  const panelLog = ui.messages.value.map(textOf);
  const popupLog = ui.writeLog.value.map(textOf);
  for (const pattern of [/SD書込の準備を開始します/, /書込前：対象情報を更新/, /sdDirectaxs.csv読込完了/, /dictionary作成/, /inputフォルダあり/, /inputフォルダへ移動/, /拡張子が異なる/]) {
    assert.ok(panelLog.some(text => pattern.test(text)), `missing panel message: ${pattern}`);
    assert.ok(popupLog.every(text => !pattern.test(text)), `preparation leaked into popup: ${pattern}`);
  }
  assert.ok(popupLog.includes('SD書込先: E: / 対象: 1件 / 512(0x200)バイト'));
  assert.ok(popupLog.every(text => !panelLog.includes(text)), 'popup log must not be duplicated in panel_text');
  ui.unmount();
});

test('empty plans and preparation failures keep a result popup without writing', async () => {
  for (const fail of [false, true]) {
    let ran = false;
    const ui = view(backend({
      prepare_sd_write: args => { if (fail) throw new Error('SD未接続'); return { run_id: args.runId, entries: [] }; },
      run_sd_write: () => { ran = true; }, discard_sd_write: () => {},
    }));
    await ui.mount();
    await ui.onSdWrite('E:');
    assert.equal(ran, false);
    assert.equal(ui.writing.value, false);
    assert.equal(ui.writeDialogOpen.value, true);
    assert.ok(ui.messages.value.every(m => typeof m === 'string' ? !/SD未接続|書込対象なし/.test(m) : !/SD未接続|書込対象なし/.test(m.text)));
    assert.ok(ui.writeLog.value.some(m => fail ? typeof m === 'object' && /SD未接続/.test(m.text) : typeof m === 'string' && /書込対象なし/.test(m)));
    ui.closeWriteDialog();
    assert.equal(ui.writeDialogOpen.value, false);
    ui.unmount();
  }
});

test('failed verification preserves progress and identifies the failed and unexecuted files', async () => {
  let ui;
  ui = view(backend({
    prepare_sd_write: args => ({ run_id: args.runId, drive: 'E:', total_bytes: 1536, entries: ['a','b','c'].map(filename => ({ filename, size: 512, file_size: 512 })) }),
    run_sd_write: args => {
      ui.event('sd-progress', { run_id: args.runId, filename: 'b', index: 2, total: 3, completed: 1, stage: 'failed', processed_bytes: 1536, total_bytes: 3072, elapsed_ms: 1000, file_ms: 253, error: '照合不一致' });
      throw new Error('照合不一致');
    }, discard_sd_write: () => {},
  }));
  await ui.mount();
  const task = ui.onSdWrite('E:');
  await task;
  assert.equal(ui.writing.value, false);
  assert.equal(ui.percent.value, 50);
  assert.equal(ui.writeDialogOpen.value, true);
  assert.ok(ui.writeLog.value.some(m => typeof m === 'object' && /照合不一致/.test(m.text)));
  assert.equal(ui.hasTimer, false);
  assert.ok(ui.writeLog.value.some(m => typeof m === 'object' && /b: 一部だけ/.test(m.text)));
  assert.ok(ui.writeLog.value.includes('未実行: c'));
  assert.ok(ui.messages.value.every(m => typeof m === 'string' ? !/未実行|正常完了/.test(m) : !/照合不一致|一部だけ/.test(m.text)));
  ui.unmount();
});

test('size comments distinguish truncation, padding, zero-byte fill, and exact size', () => {
  const { sizeComment, writePercent, formatByteCount } = utility('src/utils/sdWrite.ts');
  for (const [value, expected] of [[0, '0(0x0)'], [1235, '1,235(0x4d3)'], [5539840, '5,539,840(0x548800)'], [89694, '89,694(0x15e5e)']]) {
    assert.equal(formatByteCount(value), expected);
  }
  assert.equal(sizeComment({ filename: 'FILE-BT.001', file_size: 64301, size: 65536 }), 'FILE-BT.001: ファイル64,301(0xfb2d)バイトを書き込み、残り1,235(0x4d3)バイトを0xFFで埋めます。');
  assert.equal(sizeComment({ filename: 'FILE-BR.001', file_size: 3783, size: 3072 }), '注意: FILE-BR.001 は領域サイズを超えています。ファイル: 3,783(0xec7)バイト、領域: 3,072(0xc00)バイト。末尾711(0x2c7)バイトを無視して書き込みます。');
  assert.equal(sizeComment({ filename: 'empty', file_size: 0, size: 1024 }), 'empty: 0(0x0)バイトのため、対象領域1,024(0x400)バイト全体を0xFFで埋めます。');
  const entry = { filename: 'a', size: 1024 };
  assert.match(sizeComment({ ...entry, file_size: 1536 }), /末尾512\(0x200\)バイトを無視/);
  assert.match(sizeComment({ ...entry, file_size: 512 }), /残り512\(0x200\)バイトを0xFF/);
  assert.match(sizeComment({ ...entry, file_size: 0 }), /全体を0xFF/);
  assert.equal(sizeComment({ ...entry, file_size: 1024 }), null);
  assert.equal(writePercent({ stage: 'verify', processed_bytes: 2048, total_bytes: 2048 }), 99);
  assert.equal(writePercent({ stage: 'done', processed_bytes: 2048, total_bytes: 2048 }), 100);
  assert.equal(writePercent({ stage: 'prepare', processed_bytes: 0, total_bytes: 0 }), 0);
});

test('shutdown waiting keeps target changes disabled and reports wait failures', async () => {
  const calls = [];
  const run = backend();
  const ui = view(async (command, args) => { calls.push(command); return run(command, args); });
  await ui.mount();
  assert.equal(ui.canSdWrite.value, true);
  const source = ui.sourceDirectory.value;
  const before = calls.length;
  ui.event('sd-shutdown-started');
  assert.equal(ui.canSdWrite.value, false);
  ui.drop(['C:\\other']);
  assert.equal(ui.key('F5'), true);
  await ui.onSdWrite('E:');
  assert.equal(ui.sourceDirectory.value, source);
  assert.equal(calls.length, before);
  assert.match(ui.transientNotice.value, /終了処理中/);
  ui.event('sd-shutdown-error', '終了確認失敗');
  assert.ok(ui.messages.value.some(m => typeof m === 'object' && /終了確認失敗/.test(m.text)));
  assert.equal(ui.canSdWrite.value, false);
  ui.unmount();
});
