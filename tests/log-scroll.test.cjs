const { test } = require('node:test');
const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const ts = require('typescript');
const exportsForScroll = {};
new Function('exports', ts.transpileModule(readFileSync('src/utils/logScroll.ts', 'utf8'), {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText)(exportsForScroll);

function setup(reducedMotion = false) {
  let id = 0, time = 0;
  const pending = new Map();
  const panel = { scrollTop: 0, scrollHeight: 1000, clientHeight: 200, scrollLeft: 75 };
  const scroller = exportsForScroll.createLogScroller(() => panel, {
    request: callback => { pending.set(++id, callback); return id; },
    cancel: id => pending.delete(id),
    reducedMotion: () => reducedMotion,
  });
  return { panel, scroller, pending,
    frame() {
      time += 16;
      const callbacks = [...pending.values()];
      pending.clear();
      callbacks.forEach(callback => callback(time));
    },
  };
}

test('burst logging coalesces frames and follows a moving target without jumps', () => {
  const ui = setup();
  for (let i = 0; i < 37; i++) ui.scroller.append();
  assert.equal(ui.pending.size, 1);
  ui.frame();
  assert.ok(ui.panel.scrollTop > 0 && ui.panel.scrollTop < 800);
  ui.scroller.onScroll(); // Programmatic scroll events do not interrupt animation.
  ui.panel.scrollHeight += 300;
  ui.scroller.append();
  assert.equal(ui.pending.size, 1);
  for (let i = 0; i < 100 && ui.pending.size; i++) {
    const before = ui.panel.scrollTop;
    ui.frame();
    assert.ok(ui.panel.scrollTop >= before && ui.panel.scrollTop <= 1100);
    ui.scroller.onScroll();
  }
  assert.equal(ui.panel.scrollTop, 1100);
  assert.equal(ui.panel.scrollLeft, 75);
  assert.equal(ui.pending.size, 0);
});

test('reading older logs pauses following until the user returns to the bottom', () => {
  const ui = setup();
  ui.scroller.append();
  ui.frame();
  ui.panel.scrollTop = 0;
  ui.scroller.onScroll();
  assert.equal(ui.pending.size, 0);
  ui.panel.scrollHeight += 500;
  ui.scroller.append();
  assert.equal(ui.pending.size, 0);
  assert.equal(ui.panel.scrollTop, 0);
  ui.panel.scrollTop = 1300;
  ui.scroller.onScroll();
  ui.panel.scrollHeight += 100;
  ui.scroller.append();
  assert.equal(ui.pending.size, 1);
  ui.frame();
  assert.ok(ui.panel.scrollTop > 1300);
});

test('delayed own scroll events after new rows arrive keep following enabled', () => {
  const ui = setup(true);
  ui.scroller.append();
  ui.frame();
  assert.equal(ui.panel.scrollTop, 800);
  ui.panel.scrollHeight += 100;
  ui.scroller.append();
  ui.scroller.onScroll();
  ui.frame();
  assert.equal(ui.panel.scrollTop, 900);
});

test('reduced motion, disposal and a new run do not leave animations running', () => {
  const ui = setup(true);
  ui.scroller.append();
  ui.frame();
  assert.equal(ui.panel.scrollTop, 800);
  assert.equal(ui.pending.size, 0);
  ui.scroller.append();
  ui.scroller.dispose();
  assert.equal(ui.pending.size, 0);
  ui.scroller.append();
  assert.equal(ui.pending.size, 0);
  ui.scroller.reset();
  assert.equal(ui.panel.scrollTop, 0);
  ui.scroller.onScroll(); // Ignore the reset's own delayed scroll event.
  ui.scroller.append();
  ui.frame();
  assert.equal(ui.panel.scrollTop, 800);
});
