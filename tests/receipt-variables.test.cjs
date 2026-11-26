const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { test } = require('node:test');
const ts = require('typescript');
const vue = require('vue');
const searchUtility = {};
new Function('exports', ts.transpileModule(readFileSync('src/utils/receiptSearch.ts', 'utf8'), {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText)(searchUtility);

// Run the view's real setup/reactivity with only IPC and mounting replaced.
async function receipt(t, texts) {
  const source = readFileSync('src/pages/ReceiptView.vue', 'utf8')
    .match(/<script setup lang="ts">([\s\S]*?)<\/script>/)[1];
  const js = ts.transpileModule(source + `
    return { selected, selectedVariable, preview, mode, html, variables, selectVariable,
      searchQuery, currentSearchMatch, searchResult, moveSearchMatch };
  `, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS } }).outputText;
  let mount;
  const scope = vue.effectScope();
  t.after(() => scope.stop());
  const ui = scope.run(() => new Function('require', 'exports', 'defineProps', js)(
    name => name === 'vue'
      ? { ...vue, onMounted: callback => { mount = callback; } }
      : name === '../utils/receiptSearch' ? searchUtility : { invoke: async () => ({
          rows: texts.map((text, index) => ({ name: `${index}.txt`, bytes: [...Buffer.from(text)] })),
          source_size: 100,
          reused: false,
        }) },
    {},
    () => ({ directory: 'fixture' }),
  ));
  await mount();
  await vue.nextTick();
  return ui;
}

for (const mode of ['standard', 'if']) {
  test(`variable selection highlights all occurrences and scrolls in ${mode} mode`, async t => {
    const ui = await receipt(t, ['<%IFM001%><%M001:04D%> M001 <%M002%><%EI%>']);
    ui.mode.value = mode;
    const scrolls = [];
    ui.preview.value = { querySelectorAll: () => [], querySelector(selector) {
      assert.equal(selector, '.selected-variable');
      assert.match(ui.html.value, /<mark class="selected-variable">/);
      return { scrollIntoView: options => scrolls.push(options) };
    } };

    await ui.selectVariable('M001');
    assert.equal(ui.selectedVariable.value, 'M001');
    assert.equal((ui.html.value.match(/<mark class="selected-variable">M001<\/mark>/g) || []).length, 3);
    assert.doesNotMatch(ui.html.value, /<mark[^>]*>M002/);
    assert.match(ui.html.value, /token-if/);
    assert.equal(scrolls.length, 1);
    assert.equal(scrolls[0].block, 'center');

    await ui.selectVariable('M002');
    assert.equal((ui.html.value.match(/<mark class="selected-variable">M002<\/mark>/g) || []).length, 1);
    assert.doesNotMatch(ui.html.value, /<mark[^>]*>M001/);
    assert.equal(scrolls.length, 2);
  });
}

test('changing receipts clears selection even when the next receipt uses the same variable', async t => {
  const ui = await receipt(t, ['<%M001%>', '<%M001%>']);
  await ui.selectVariable('M001');
  ui.selected.value = 1;
  await vue.nextTick();
  assert.equal(ui.selectedVariable.value, null);
  assert.doesNotMatch(ui.html.value, /<mark/);
});

test('highlighting keeps receipt HTML escaped and IF diagnostics intact', async t => {
  const ui = await receipt(t, ['<img src=x onerror=alert(1)> M001 <%IFM001%>']);
  await ui.selectVariable('M001');
  assert.match(ui.html.value, /&lt;img src=x onerror=alert\(1\)&gt;/);
  assert.doesNotMatch(ui.html.value, /<img/);
  assert.match(ui.html.value, /token-error/);
  assert.match(ui.html.value, /data-file-bq-error-ordinal="0"/);
  assert.equal((ui.html.value.match(/<mark class="selected-variable">M001<\/mark>/g) || []).length, 2);
});

test('empty receipts and absent preview do not cause selection errors', async t => {
  const ui = await receipt(t, []);
  await ui.selectVariable('M001');
  assert.equal(ui.html.value, '');
  assert.deepEqual(ui.variables.value, []);
});

test('search counts, wraps navigation, refreshes after receipt changes, and clears', async t => {
  const ui = await receipt(t, ['<%M001%> M001', '<%M002%>']);
  ui.searchQuery.value = 'M001';
  await vue.nextTick();
  assert.equal(ui.searchResult.value.count, 2);
  assert.equal(ui.currentSearchMatch.value, 0);
  ui.moveSearchMatch(1);
  assert.equal(ui.currentSearchMatch.value, 1);
  ui.moveSearchMatch(1);
  assert.equal(ui.currentSearchMatch.value, 0);
  ui.moveSearchMatch(-1);
  assert.equal(ui.currentSearchMatch.value, 1);
  await ui.selectVariable('M001');
  assert.match(ui.html.value, /selected-variable/);
  assert.match(ui.html.value, /receipt-search-match/);
  ui.mode.value = 'if';
  await vue.nextTick();
  assert.equal(ui.searchResult.value.count, 2);
  ui.selected.value = 1;
  await vue.nextTick();
  assert.equal(ui.searchResult.value.count, 0);
  assert.equal(ui.currentSearchMatch.value, -1);
  ui.moveSearchMatch(1);
  assert.equal(ui.currentSearchMatch.value, -1);
  ui.searchQuery.value = '';
  await vue.nextTick();
  assert.doesNotMatch(ui.html.value, /receipt-search-match/);
});
