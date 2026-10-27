const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { test } = require('node:test');
const ts = require('typescript');

function menu(invoke) {
  const source = readFileSync('src/components/SdMenu.vue', 'utf8').match(/<script setup lang="ts">([\s\S]*?)<\/script>/)[1]
    .replace(/^import .*;\r?$/gm, '');
  const compiled = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS } }).outputText;
  let mount;
  let closes = 0;
  const props = { canWrite: true, busy: false };
  const actions = [];
  const calls = [];
  const bindings = {
    defineProps: () => props, defineEmits: () => (...args) => actions.push(args),
    ref: value => ({ value }), computed: fn => ({ get value() { return fn(); } }),
    nextTick: async () => {}, onMounted: fn => { mount = fn; }, onUnmounted: () => {},
    getName: async () => 'ts-sddirecta', getVersion: async () => '0.1.0',
    invoke: async (command, args) => {
      calls.push([command, args]);
      if (command === 'get_drives') return [];
      if (command === 'sd_admin_status') return 'idle';
      return invoke(command, args);
    },
    setInterval: () => 1, clearInterval: () => {},
    window: { addEventListener() {}, removeEventListener() {} },
  };
  const api = new Function(...Object.keys(bindings), compiled + `
    return { showAppInfo, installUpdate, closeAppInfo, infoDialog, infoMessage, appVersion, availableUpdate, installingUpdate };
  `)(...Object.values(bindings));
  api.infoDialog.value = { showModal() {}, close() { closes++; } };
  return { ...api, mount: () => mount(), props, calls, actions, closes: () => closes };
}

const update = { status: 'available', current_version: '0.1.0', version: '0.2.0', notes: 'Changes' };

test('startup and version info never check for updates; No never installs', async () => {
  const ui = menu(async () => update);
  ui.mount();
  await ui.showAppInfo('バージョン情報');
  assert.equal(ui.appVersion.value, '0.1.0');
  assert.equal(ui.calls.some(([name]) => name.includes('portable_update')), false);
  await ui.showAppInfo('アップデート確認');
  assert.match(ui.infoMessage.value, /0\.2\.0.*実行しますか/);
  ui.closeAppInfo();
  assert.equal(ui.closes(), 1);
  assert.equal(ui.calls.some(([name]) => name === 'install_portable_update'), false);
});

test('latest, unpublished and connection failure remain distinct', async () => {
  for (const [status, expected] of [['latest', /最新バージョン/], ['unpublished', /まだ公開されていません/], ['error', /確認に失敗/]]) {
    const ui = menu(async () => { if (status === 'error') throw new Error('network'); return { ...update, status }; });
    await ui.showAppInfo('アップデート確認');
    assert.match(ui.infoMessage.value, expected);
    assert.equal(ui.availableUpdate.value, null);
  }
});

test('YES installs the confirmed version, blocks closing and releases UI on failure', async () => {
  let rejectInstall;
  const pending = new Promise((_, reject) => { rejectInstall = reject; });
  const ui = menu(async (command, args) => {
    if (command === 'check_portable_update') return update;
    assert.equal(command, 'install_portable_update');
    assert.deepEqual(args, { version: '0.2.0' });
    return pending;
  });
  await ui.showAppInfo('アップデート確認');
  const task = ui.installUpdate();
  assert.equal(ui.installingUpdate.value, true);
  ui.closeAppInfo();
  assert.equal(ui.closes(), 0);
  await ui.installUpdate(); // Double-click must not start a second update.
  assert.equal(ui.calls.filter(([name]) => name === 'install_portable_update').length, 1);
  rejectInstall(new Error('invalid signature'));
  await task;
  assert.equal(ui.installingUpdate.value, false);
  assert.equal(ui.availableUpdate.value, null);
  assert.match(ui.infoMessage.value, /invalid signature/);
  ui.closeAppInfo();
  assert.equal(ui.closes(), 1);
});

test('a closed check cannot show late results and SD activity blocks checking', async () => {
  let resolveCheck;
  const response = new Promise(resolve => { resolveCheck = resolve; });
  const ui = menu(() => response);
  const task = ui.showAppInfo('アップデート確認');
  await Promise.resolve(); await Promise.resolve(); await Promise.resolve();
  ui.closeAppInfo();
  resolveCheck(update);
  await task;
  assert.equal(ui.availableUpdate.value, null);
  const busyUi = menu(() => { throw new Error('must not check'); });
  busyUi.props.busy = true;
  await busyUi.showAppInfo('アップデート確認');
  assert.equal(busyUi.calls.length, 0);
});
