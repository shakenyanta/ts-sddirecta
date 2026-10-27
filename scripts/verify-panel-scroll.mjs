// Render the built app in Chromium with mocked IPC. Never access an SD card.
// Usage: node scripts/verify-panel-scroll.mjs [path-to-chrome-or-edge]
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile, mkdtemp } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve, extname } from 'node:path';
import { spawn } from 'node:child_process';

const browser = process.argv[2] || 'C:/Program Files/Google/Chrome/Application/chrome.exe';
const root = resolve('dist');
const csv = await readFile('config/sdDirectaxs.csv', 'utf8');
const bootstrap = `
const callbacks = new Map(); let next = 1;
window.__TAURI_INTERNALS__ = {
  metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main' } },
  transformCallback(fn) { const id = next++; callbacks.set(id, fn); return id; },
  async invoke(command) {
    if (command.startsWith('plugin:event|')) return next++;
    if (command === 'get_executable_directory') return ${JSON.stringify('C:\\source\\repos\\electron3\\ts-sddirecta\\src-tauri\\target\\debug')};
    if (command === 'sd_admin_status') return 'ready';
    if (command === 'get_drives') return [{ drive: 'E:', type: 'Removable', size: 0 }, { drive: 'H:', type: 'Removable', size: 512 * 1024 * 1024 }];
    if (command === 'check_startup_csv') return true;
    if (command === 'read_startup_csv') return { contents: ${JSON.stringify(csv)}, source_folder: 'test CSV' };
    if (command === 'find_dictionary_files') return { files: [], from_input: false, input_exists: true,
      warning_files: Array.from({length: 45}, (_, i) => ${JSON.stringify('input\\IM.')} + String(i + 1).padStart(3, '0')) };
    throw Error('Unexpected mock IPC: ' + command);
  }
};
window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
const delay = ms => new Promise(r => setTimeout(r, ms));
const render = () => new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));
window.addEventListener('load', async () => {
  await delay(600);
  const panel = document.querySelector('.text-messages');
  const results = [];
  for (const [width, height] of [[300, 292], [280, 300], [360, 360], [301, 291.5]]) {
    const outer = document.querySelector('.panel-outer');
    outer.style.width = width + 'px'; outer.style.height = height + 'px';
    await delay(150);
    await render();
    panel.scrollTop = panel.scrollHeight;
    const top = panel.scrollTop;
    const before = { scroll: panel.scrollHeight, client: panel.clientHeight, rows: document.querySelector('.panel-base').style.gridTemplateRows };
    outer.style.height = (height - 5) + 'px';
    await delay(750); // Includes periodic drive/admin refresh and ResizeObserver.
    await render();
    const bottom = panel.getBoundingClientRect().bottom;
    const lastBottom = panel.lastElementChild.getBoundingClientRect().bottom;
    const header = document.querySelector('.text-header').getBoundingClientRect();
    const parent = panel.parentElement;
    results.push({ width, height, top, after: panel.scrollTop, before, scroll: panel.scrollHeight, client: panel.clientHeight,
      rows: document.querySelector('.panel-base').style.gridTemplateRows, lastBottom, bottom,
      atBottom: Math.abs(panel.scrollHeight - panel.clientHeight - panel.scrollTop) <= 1,
      lastVisible: lastBottom <= bottom && lastBottom > panel.getBoundingClientRect().top,
      headerVisible: header.top >= parent.getBoundingClientRect().top,
      parentScroll: parent.scrollTop, parentOverflow: getComputedStyle(parent).overflowY });
    panel.scrollTop = Math.floor(top / 2);
    const manual = panel.scrollTop;
    outer.style.height = height + 'px';
    await delay(750);
    await render();
    results.at(-1).manualStable = Math.abs(manual - panel.scrollTop) <= 1;
  }
  document.body.dataset.panelAudit = JSON.stringify(results);
});`;

const server = createServer(async (req, res) => {
  try {
    const pathname = new URL(req.url, 'http://localhost').pathname;
    const file = pathname === '/' ? join(root, 'index.html') : resolve(root, '.' + pathname);
    if (!file.startsWith(root + '/') && !file.startsWith(root + '\\')) {
      res.writeHead(403).end(); return;
    }
    let contents = await readFile(file);
    if (extname(file) === '.html') contents = contents.toString().replace('<head>', `<head><script>${bootstrap}</script>`);
    res.setHeader('Content-Type', ({ '.html': 'text/html; charset=utf-8', '.js': 'text/javascript', '.css': 'text/css', '.png': 'image/png' })[extname(file)] || 'application/octet-stream');
    res.end(contents);
  } catch { res.writeHead(404).end(); }
});
await new Promise(r => server.listen(0, '127.0.0.1', r));
const profile = await mkdtemp(join(tmpdir(), 'sddirect-panel-browser-'));
const url = `http://127.0.0.1:${server.address().port}/`;
const child = spawn(browser, [
  '--headless', '--disable-gpu', '--no-first-run', '--disable-background-networking',
  '--no-default-browser-check', '--disable-background-timer-throttling',
  '--disable-renderer-backgrounding', `--user-data-dir=${profile}`, '--remote-debugging-port=0', url,
], { windowsHide: true, stdio: 'ignore' });
let socket;
const delay = ms => new Promise(r => setTimeout(r, ms));
try {
  let port;
  for (let i = 0; i < 100 && !port; i++) {
    try { port = (await readFile(join(profile, 'DevToolsActivePort'), 'utf8')).split('\n')[0]; }
    catch { await delay(100); }
  }
  assert.ok(port, 'Browser did not start');
  let target;
  for (let i = 0; i < 100 && !target; i++) {
    const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
    target = targets.find(t => t.type === 'page' && t.url.startsWith(url));
    if (!target) await delay(100);
  }
  assert.ok(target, 'Browser did not load the test app');
  socket = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => { socket.onopen = resolve; socket.onerror = reject; });
  let sequence = 0;
  const pending = new Map();
  socket.onmessage = event => {
    const response = JSON.parse(event.data);
    if (response.id) { pending.get(response.id)?.(response); pending.delete(response.id); }
  };
  const evaluate = expression => new Promise(resolve => {
    const id = ++sequence;
    pending.set(id, resolve);
    socket.send(JSON.stringify({ id, method: 'Runtime.evaluate', params: { expression, returnByValue: true } }));
  });
  let encoded;
  for (let i = 0; i < 120 && !encoded; i++) {
    const response = await evaluate('document.body.dataset.panelAudit');
    encoded = response.result?.result?.value;
    if (!encoded) await delay(250);
  }
  assert.ok(encoded, 'Browser did not finish the panel audit');
  const results = JSON.parse(encoded);
  console.log(JSON.stringify(results, null, 2));
  for (const result of results) {
    assert.ok(result.atBottom && result.lastVisible && result.headerVisible && result.manualStable,
      JSON.stringify(result));
    assert.equal(result.parentScroll, 0);
    assert.equal(result.parentOverflow, 'hidden');
    // The menu may wrap after the first frame. A changed viewport must still
    // keep its last row visible; an unchanged viewport must keep its position.
    if (result.before.client === result.client) {
      assert.ok(Math.abs(result.top - result.after) <= 1, JSON.stringify(result));
    }
  }
} finally { socket?.close(); child.kill(); server.close(); }
