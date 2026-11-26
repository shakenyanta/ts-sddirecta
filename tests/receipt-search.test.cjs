const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { test } = require('node:test');
const ts = require('typescript');
const utility = {};
new Function('exports', ts.transpileModule(readFileSync('src/utils/receiptSearch.ts', 'utf8'), {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText)(utility);
const { highlightReceiptSearch } = utility;

test('matches complete receipt tokens across syntax tags and escaped delimiters', () => {
  const html = '<span>&lt;%</span><span>IF</span><span>M001</span><span>%&gt;</span>';
  const result = highlightReceiptSearch(html, '<%IFM001%>');
  assert.equal(result.count, 1);
  assert.equal((result.html.match(/data-receipt-search-index="0"/g) || []).length, 4);
  assert.equal(result.html.replace(/<[^>]+>/g, ''), '&lt;%IFM001%&gt;');
});

test('performs literal case-insensitive matching, including punctuation and Japanese text', () => {
  const result = highlightReceiptSearch('M001 m001 M002', 'm001');
  assert.equal(result.count, 2);
  assert.match(result.html, /data-receipt-search-index="1">m001/);
  assert.equal(highlightReceiptSearch('.* [x] .*', '.*').count, 2);
  assert.equal(highlightReceiptSearch('領収書 領収書', '領収書').count, 2);
  assert.equal(highlightReceiptSearch('a &amp; b &lt;c&gt;', '&').count, 1);
});

test('does not search attributes or introduce markup from receipt text', () => {
  const html = '<span title="M001">&lt;img src=x&gt; &amp;lt;</span>';
  assert.equal(highlightReceiptSearch(html, 'M001').count, 0);
  const result = highlightReceiptSearch(html, '<img');
  assert.equal(result.count, 1);
  assert.doesNotMatch(result.html, /<img/);
  assert.match(result.html, /&amp;lt;/);
});

test('preserves variable selection markup and leaves unmatched/empty searches unchanged', () => {
  const html = '<mark class="selected-variable">M001</mark>';
  const result = highlightReceiptSearch(html, 'M001');
  assert.match(result.html, /<mark class="selected-variable"><mark class="receipt-search-match"/);
  for (const query of ['', 'absent']) assert.deepEqual(highlightReceiptSearch(html, query), { html, count: 0 });
});

test('tracks adjacent matches across empty and multiple text segments', () => {
  const result = highlightReceiptSearch('a<span></span>b<span>a</span>b', 'ab');
  assert.equal(result.count, 2);
  assert.equal((result.html.match(/data-receipt-search-index="0"/g) || []).length, 2);
  assert.equal((result.html.match(/data-receipt-search-index="1"/g) || []).length, 2);
});
