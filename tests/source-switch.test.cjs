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
  const keyListeners = new Set();
  const bindings = {
    ref: value => ({ value }),
    computed: get => ({ get value() { return get(); } }),
    nextTick: async () => {},
    onMounted: fn => { mount = fn; },
    onUnmounted: fn => { unmount = fn; },
    getCurrentWebview: () => ({ onDragDropEvent: async fn => { listener = fn; return () => { unlistened = true; }; } }),
    invoke,
    window: {
      addEventListener: (name, fn) => { if (name === 'keydown') keyListeners.add(fn); },
      removeEventListener: (name, fn) => { if (name === 'keydown') keyListeners.delete(fn); },
    },
    setInterval: fn => { timer = fn; return 1; },
    clearInterval: () => { timer = undefined; },
    ...utility('src/utils/sdDirectCsv.ts'),
    ...utility('src/utils/executablePath.ts'),
  };
  const api = new Function(...Object.keys(bindings), transpile(source) + `
    return { initializeSource, refreshMatchedFiles, sourceDirectory, displayedDirectory,
      sdDirectDictionary, matchedFiles, canSdWrite, initializing, messages, folders, pathError };
  `)(...Object.values(bindings));
  return { ...api, mount: () => mount(), unmount: () => unmount(),
    drop: paths => listener({ payload: { type: 'drop', paths } }),
    key: (key, repeat = false) => {
      let prevented = false;
      for (const fn of keyListeners) fn({ key, repeat, preventDefault: () => { prevented = true; } });
      return prevented;
    },
    get hasTimer() { return !!timer; },
    tick: () => timer?.(), get unlistened() { return unlistened; } };
}

const csv = 'A,FILE-AA.001,00000000,000001FF';
function backend(overrides = {}) {
  return async (command, args) => {
    if (overrides[command]) return overrides[command](args);
    switch (command) {
      case 'get_executable_directory': return 'C:\\app';
      case 'resolve_drop_directory': return args.paths[0];
      case 'check_startup_csv': return true;
      case 'read_startup_csv': return csv;
      case 'find_dictionary_files': return { files: ['FILE-AA.001'], from_input: true, input_exists: true };
      default: throw new Error(command);
    }
  };
}

test('startup and native drop share initialization and use the selected source for all IPC', async () => {
  const calls = [];
  const run = backend();
  const ui = view(async (command, args) => { calls.push([command, args]); return run(command, args); });
  await ui.mount();
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
  assert.equal(ui.sdDirectDictionary.value.size, 0);
  assert.deepEqual(ui.matchedFiles.value, []);
  assert.equal(ui.canSdWrite.value, false);
  await ui.initializeSource(['C:\\valid']);
  await ui.initializeSource(['C:\\bad']);
  assert.equal(ui.sdDirectDictionary.value.size, 0);
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
