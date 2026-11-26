<script setup lang="ts">
import { computed, nextTick, onMounted, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
const props = defineProps<{ directory: string }>();
const rows = ref<{name: string; text: string; size: number; issues: number}[]>([]);
const selected = ref(-1);
const preview = ref<HTMLElement | null>(null);
const mode = ref('standard');
const showVariables = ref(false);
const fontSize = ref(13);
const status = ref('');
const sourceSize = ref(0);
const fileBqReceiptIfIndentEnabled = computed(() => mode.value === 'if');
const raw = computed(() => rows.value[selected.value]?.text || '');
const analysis = computed(() => analyzeFileBqIfPairing(raw.value));
const html = computed(() => highlightFileBqReceiptText(formatFileBqReceiptText(raw.value), analysis.value.errorOrdinals));
const total = computed(() => rows.value.reduce((sum,r) => sum+r.size,0));
const variables = computed(() => {
 const counts = new Map<string,number>();
 for (const name of raw.value.match(/[A-Za-z]\d{3}/g) || []) if (!/^N/i.test(name)) counts.set(name,(counts.get(name)||0)+1);
 return [...counts].sort(([a],[b]) => a.localeCompare(b));
});
const fileBqTokenPattern = /<%[^>]*%>|<%|%>/g;
const fileBqIfPairTokenPattern = /<%IF[^>]*%>|<%EE%>|<%EI%>/g;
type FileBqPairingIssue = {
  ordinal: number
  message: string
}

function analyzeFileBqIfPairing(sourceText: string) {
  const normalized = sourceText.replace(/\r\n/g, '\n')
  const lines = normalized.split('\n')
  const openStack: Array<{ token: string; line: number; ordinal: number }> = []
  const issues: FileBqPairingIssue[] = []
  const errorOrdinals = new Set<number>()
  let pairTokenOrdinal = 0

  lines.forEach((lineText, index) => {
    const lineNumber = index + 1
    const tokens = lineText.match(fileBqIfPairTokenPattern) ?? []

    for (const token of tokens) {
      const currentOrdinal = pairTokenOrdinal
      pairTokenOrdinal += 1

      if (token === '<%EE%>') {
        if (openStack.length === 0) {
          issues.push({
            ordinal: currentOrdinal,
            message: `${lineNumber}行目: <%EE%> に対応する <%IF...%> がありません`,
          })
          errorOrdinals.add(currentOrdinal)
        }
        continue
      }

      if (token === '<%EI%>') {
        if (openStack.length === 0) {
          issues.push({
            ordinal: currentOrdinal,
            message: `${lineNumber}行目: <%EI%> に対応する <%IF...%> がありません`,
          })
          errorOrdinals.add(currentOrdinal)
          continue
        }
        openStack.pop()
        continue
      }

      openStack.push({ token, line: lineNumber, ordinal: currentOrdinal })
    }
  })

  while (openStack.length > 0) {
    const unmatched = openStack.pop()
    if (!unmatched) {
      break
    }
    issues.push({
      ordinal: unmatched.ordinal,
      message: `${unmatched.line}行目: ${unmatched.token} に対応する <%EI%> がありません`,
    })
    errorOrdinals.add(unmatched.ordinal)
  }

  return {
    issueCount: issues.length,
    issues: issues.slice(0, 12),
    errorOrdinals,
  }
}

function formatFileBqReceiptText(sourceText: string): string {
  if (!fileBqReceiptIfIndentEnabled.value) {
    return sourceText
  }

  const lines = sourceText
    .replace(/\r\n/g, '\n')
    .split('\n')
    .flatMap((rawLine) => {
      const line = rawLine.trimStart()
      return line
        .replace(/([^\n])(<%EE%>)/g, '$1\n$2')
        .replace(/(<%EE%>)(?=<%)/g, '$1\n')
        .split('\n')
    })
  const formattedLines: string[] = []
  const ifIndentStack: number[] = []
  const ifTokenPattern = /<%IF[^>]*%>|<%EE%>|<%EI%>/g

  for (const line of lines) {
    if (line.length === 0) {
      formattedLines.push(ifIndentStack.length > 0 ? '  '.repeat(ifIndentStack.length) : '')
      continue
    }

    const currentIndent = line.startsWith('<%EE%>')
      ? (ifIndentStack[ifIndentStack.length - 1] ?? 0)
      : ifIndentStack.length
    const lineOutput: string[] = []
    const inlineCloseLines: string[] = []
    let segmentIndent = currentIndent
    let displayLine = ''
    let lastIndex = 0
    let inlineIfDepth = 0

    line.replace(ifTokenPattern, (token, offset) => {
      displayLine += line.slice(lastIndex, offset)

      if (token === '<%EE%>') {
        displayLine += token
      } else if (token.startsWith('<%IF')) {
        const shouldBreakBeforeIf = displayLine.trim().length > 0
        if (shouldBreakBeforeIf) {
          lineOutput.push(`${'  '.repeat(segmentIndent)}${displayLine}`)
          displayLine = ''
          segmentIndent = ifIndentStack.length
        }
        displayLine += token
        ifIndentStack.push(segmentIndent)
        inlineIfDepth += 1
      } else if (inlineIfDepth > 0) {
        const closeIndent = ifIndentStack[ifIndentStack.length - 1] ?? segmentIndent
        if (displayLine.trim().length > 0 && closeIndent < segmentIndent) {
          lineOutput.push(`${'  '.repeat(segmentIndent)}${displayLine}`)
          displayLine = ''
          segmentIndent = closeIndent
        }
        displayLine += token
        ifIndentStack.pop()
        inlineIfDepth -= 1
      } else if (displayLine.trim().length === 0) {
        const ifIndent = ifIndentStack.pop() ?? 0
        lineOutput.push(`${'  '.repeat(ifIndent)}${token}`)
      } else if (ifIndentStack.length > 0) {
        const ifIndent = ifIndentStack.pop() ?? 0
        inlineCloseLines.push(`${'  '.repeat(ifIndent)}${token}`)
      } else {
        displayLine += token
      }

      lastIndex = offset + token.length
      return token
    })

    displayLine += line.slice(lastIndex)
    if (displayLine.length > 0) {
      lineOutput.push(`${'  '.repeat(segmentIndent)}${displayLine}`)
    }
    formattedLines.push(...lineOutput)
    formattedLines.push(...inlineCloseLines)
  }

  return formattedLines.join('\n')
}

function escapeHtml(value: string): string {
  return value
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
}

function getFileBqTokenClass(tokenBody: string): string {
  if (tokenBody.startsWith('FN')) return 'token-fn'
  if (tokenBody.startsWith('IM')) return 'token-im'
  if (tokenBody.startsWith('IF')) return 'token-if'
  if (tokenBody === 'EE') return 'token-ee'
  if (tokenBody === 'EI') return 'token-ei'
  if (tokenBody.startsWith('M')) return 'token-m'
  if (tokenBody.startsWith('S')) return 'token-s'
  if (tokenBody.startsWith('F')) return 'token-f'
  if (tokenBody.startsWith('L')) return 'token-l'
  return 'token-unknown'
}

function getFileBqTokenSegments(tokenBody: string): Array<{ text: string; className: string }> {
  if (tokenBody.startsWith('IF') && tokenBody.length > 2) {
    const tail = tokenBody.slice(2)
    return [
      { text: 'IF', className: 'token-if' },
      { text: tail, className: getFileBqTokenClass(tail) },
    ]
  }

  return [{ text: tokenBody, className: getFileBqTokenClass(tokenBody) }]
}

function renderFileBqToken(token: string, isErrorToken = false, errorOrdinal: number | null = null): string {
  const errorClass = isErrorToken ? ' token-error' : ''
  const errorAttrs = isErrorToken
    ? ` title="IF系不整合" aria-label="IF系不整合" data-file-bq-error-ordinal="${errorOrdinal ?? ''}"`
    : ''

  if (token === '<%' || token === '%>') {
    return `<span class="file-bq-token token-delimiter${errorClass}"${errorAttrs}>${escapeHtml(token)}</span>`
  }

  if (token.startsWith('<%') && token.endsWith('%>') && token.length > 4) {
    const tokenBody = token.slice(2, -2)
    const segments = getFileBqTokenSegments(tokenBody)
    return [
      `<span class="file-bq-token token-delimiter${errorClass}"${errorAttrs}>&lt;%</span>`,
      ...segments.map(segment => `<span class="file-bq-token ${segment.className}${errorClass}"${errorAttrs}>${escapeHtml(segment.text)}</span>`),
      `<span class="file-bq-token token-delimiter${errorClass}"${errorAttrs}>%&gt;</span>`,
    ].join('')
  }

  return `<span class="file-bq-token token-unknown${errorClass}"${errorAttrs}>${escapeHtml(token)}</span>`
}

function getFileBqIndentLevelClass(levelIndex: number): string {
  return `indent-level-${(levelIndex % 6) + 1}`
}

function renderFileBqIndent(indentLevel: number): string {
  if (indentLevel <= 0) {
    return ''
  }

  return Array.from({ length: indentLevel }, (_, index) =>
    `<span class="file-bq-indent ${getFileBqIndentLevelClass(index)}">&nbsp;&nbsp;</span>`
  ).join('')
}

function highlightFileBqReceiptLine(
  lineText: string,
  errorOrdinals: ReadonlySet<number>,
  pairTokenOrdinal: number
): { html: string; nextOrdinal: number } {
  let content = lineText
  let result = ''

  if (fileBqReceiptIfIndentEnabled.value) {
    const indentMatch = lineText.match(/^(  )+/)
    if (indentMatch) {
      const indentText = indentMatch[0]
      result += renderFileBqIndent(indentText.length / 2)
      content = lineText.slice(indentText.length)
    }
  }

  let lastIndex = 0

  content.replace(fileBqTokenPattern, (token, offset) => {
    result += escapeHtml(content.slice(lastIndex, offset))
    const isPairToken = token.startsWith('<%IF') || token === '<%EE%>' || token === '<%EI%>'
    const isErrorToken = isPairToken && errorOrdinals.has(pairTokenOrdinal)
    result += renderFileBqToken(token, isErrorToken, isErrorToken ? pairTokenOrdinal : null)
    if (isPairToken) {
      pairTokenOrdinal += 1
    }
    lastIndex = offset + token.length
    return token
  })

  result += escapeHtml(content.slice(lastIndex))
  return { html: result, nextOrdinal: pairTokenOrdinal }
}

function highlightFileBqReceiptText(sourceText: string, errorOrdinals: ReadonlySet<number>): string {
  const normalized = sourceText.replace(/\r\n/g, '\n')
  const lines = normalized.split('\n')
  let pairTokenOrdinal = 0

  return lines.map(lineText => {
    const highlighted = highlightFileBqReceiptLine(lineText, errorOrdinals, pairTokenOrdinal)
    pairTokenOrdinal = highlighted.nextOrdinal
    return highlighted.html
  }).join('\n')
}


onMounted(async () => {
 try {
 const data = await invoke<{ rows: { name: string; bytes: number[] }[]; source_size: number; reused: boolean }>('read_receipts', { directory: props.directory });
 const decoder = new TextDecoder('shift-jis');
 rows.value = data.rows.map(row => { const text = decoder.decode(new Uint8Array(row.bytes)); return { name: row.name, text, size: row.bytes.length, issues: analyzeFileBqIfPairing(text).issueCount }; });
 selected.value = rows.value.length > 0 ? 0 : -1;
 sourceSize.value = data.source_size;
 status.value = `${data.reused ? '既存の BQ_TXT を再利用しました' : 'BQ_TXT を作成しました'} (${data.rows.length} 件)`;
 } catch (error) { status.value = String(error); }
});
function keydown(event: KeyboardEvent) {
 if (!['ArrowDown','ArrowUp'].includes(event.key) || !rows.value.length) return;
 event.preventDefault(); selected.value = Math.max(0,Math.min(rows.value.length-1,selected.value+(event.key === 'ArrowDown' ? 1 : -1)));
}
async function scrollToIssue(ordinal: number) {
 await nextTick();
 preview.value?.querySelector<HTMLElement>(`[data-file-bq-error-ordinal="${ordinal}"]`)
  ?.scrollIntoView({ behavior: 'smooth', block: 'center', inline: 'nearest' });
}
</script>

<template>
 <main class="receipt">
  <header><strong>FILE-BQレシート</strong><span role="status">{{ status }}</span>
   <div class="mode-switch" role="group" aria-label="表示切替">
    <button type="button" :aria-pressed="mode === 'standard'" @click="mode = 'standard'">標準表示</button>
    <button type="button" :aria-pressed="mode === 'if'" @click="mode = 'if'">IF整形</button>
    <button type="button" :aria-pressed="showVariables" aria-controls="receipt-variables" @click="showVariables = !showVariables">変数一覧</button>
   </div>
   <div class="mode-switch font-size-control" role="group" aria-label="レシート本文の文字サイズ">
    <button type="button" aria-label="文字を小さくする" title="文字を小さくする" :disabled="fontSize <= 10" @click="fontSize--">−</button>
    <button type="button" aria-label="文字サイズを標準の13pxに戻す" title="標準の13pxに戻す" @click="fontSize = 13">{{ fontSize }}px</button>
    <button type="button" aria-label="文字を大きくする" title="文字を大きくする" :disabled="fontSize >= 24" @click="fontSize++">＋</button>
   </div>
  </header>
  <div class="panes">
   <section class="list">
    <div class="grid" tabindex="0" @keydown="keydown">
     <table><thead><tr><th></th><th>ファイル名</th><th>容量</th><th>状態</th></tr></thead>
      <tbody><tr v-for="(row,index) in rows" :key="row.name" :class="{ selected: index === selected }" @click="selected = index">
       <td>{{ index + 1 }}</td><td class="name" :title="row.name">{{ row.name }}</td><td>{{ row.size }}</td><td :class="{ error: row.issues }">{{ row.issues ? 'NG' : 'OK' }}</td>
      </tr></tbody></table>
    </div>
    <footer>{{ rows.length }} 行 / 合計 {{ total }}バイト ({{ sourceSize ? Math.round(total/sourceSize*100) : 0 }}%)</footer>
   </section>
   <section class="preview" aria-label="レシート本文">
    <div v-if="analysis.issues.length" class="diagnostics" role="alert">
     <button v-for="issue in analysis.issues" :key="issue.ordinal" type="button" class="diagnostic" @click="scrollToIssue(issue.ordinal)">{{ issue.message }}</button>
    </div>
    <pre ref="preview" class="file-bq-highlighted" :style="{ fontSize: `${fontSize}px` }" v-html="html"></pre>
   </section>
   <section v-show="showVariables" id="receipt-variables" class="variables" aria-label="変数一覧">
    <table><thead><tr><th>No</th><th>変数名</th><th>使用数</th><th>値</th></tr></thead><tbody><tr v-for="([name,count],index) in variables" :key="name"><td>{{ index+1 }}</td><td>{{ name }}</td><td>{{ count }}</td><td></td></tr></tbody></table>
   </section>
  </div>
 </main>
</template>

<style scoped>
.receipt { height:100%; display:flex; flex-direction:column; font:12px 'Meiryo',sans-serif; color:#192d46; }
header { display:flex; flex-wrap:wrap; align-items:center; gap:12px; padding:8px; border-bottom:1px solid #bbb; min-height:40px; }
header span { flex:1; font-size:11px; overflow-wrap:anywhere; }
.mode-switch { display:flex; flex-shrink:0; margin-left:auto; }
.mode-switch button { border:1px solid #c7d5e5; padding:4px 10px; background:white; color:inherit; font:inherit; cursor:pointer; }
.mode-switch button + button { margin-left:-1px; }
.mode-switch button:first-child { border-radius:16px 0 0 16px; }
.mode-switch button:last-child { border-radius:0 16px 16px 0; }
.mode-switch button:hover { background:#eff4fa; }
.mode-switch button[aria-pressed="true"] { background:#c8e1ff; font-weight:600; }
.mode-switch button:focus-visible { outline:2px solid #2875b9; outline-offset:2px; position:relative; z-index:1; }
.mode-switch button:disabled { opacity:0.45; cursor:default; background:white; }
.font-size-control { margin-left:0; }
.panes { display:flex; flex:1; min-height:0; }
.list { width:34%; min-width:240px; max-width:40%; flex-shrink:0; box-sizing:border-box; resize:horizontal; overflow:auto; display:flex; flex-direction:column; border-right:3px solid #7db1d4; }
.variables { width:20%; min-width:180px; max-width:40%; flex-shrink:0; box-sizing:border-box; resize:horizontal; overflow:auto; border-left:3px solid #7db1d4; background:#f4f7fa; }
.variables table { table-layout:fixed; min-width:180px; }
.variables th:first-child { width:28px; }
.variables th:nth-child(2) { width:40px; }
.variables th:nth-child(3) { width:42px; }
.variables th:last-child { width:42px; }
.grid { flex:1; overflow:auto; }
.grid table { table-layout:fixed; }
.grid th:first-child { width:28px; }
.grid th:nth-child(3) { width:48px; }
.grid th:last-child { width:40px; }
table { border-collapse:collapse; width:100%; white-space:nowrap; font:11px/16px 'Yu Gothic UI',Meiryo,sans-serif; }
th,td { height:18px; border:1px solid #ccc; padding:0 3px; text-align:left; vertical-align:middle; }
th { height:24px; font-weight:600; }
th { position:sticky; top:0; background:#eee; }
tbody tr:nth-child(even) { background:#eff4fa; }
tbody tr.selected { background:#c8e1ff; }
.name { white-space:pre; overflow:hidden; text-overflow:ellipsis; font-family:'BIZ UDGothic','BIZ UDゴシック','MS Gothic',monospace; }
footer { font-size:10px; background:#eef3fa; }
.preview { flex:1; min-width:0; overflow:auto; background:#f4f7fa; }
pre { padding:8px; font:13px/1.3 'BIZ UDGothic','BIZ UDゴシック','MS Gothic',monospace; white-space:pre; }
.diagnostics { display:flex; flex-direction:column; gap:4px; padding:8px 10px; border-bottom:1px solid #f1aaa4; background:#ffe5e2; }
.diagnostic { border:0; padding:0; background:transparent; color:#8f1d14; font:inherit; font-size:11px; line-height:1.35; text-align:left; cursor:pointer; }
.diagnostic:hover { text-decoration:underline; }
.error { color:#c22; }
.file-bq-highlighted {
  background:
    linear-gradient(180deg, #eef5fb, #ffffff 22%);
}

.file-bq-highlighted :deep(.file-bq-indent) {
  display: inline-block;
  width: 2ch;
  min-width: 2ch;
  box-sizing: border-box;
  border-left: 2px solid transparent;
  border-right: 1px solid rgba(255, 255, 255, 0.65);
}

.file-bq-highlighted :deep(.file-bq-indent.indent-level-1) {
  border-left-color: #7aa6ff;
  background: linear-gradient(90deg, rgba(122, 166, 255, 0.3), rgba(122, 166, 255, 0.08));
}

.file-bq-highlighted :deep(.file-bq-indent.indent-level-2) {
  border-left-color: #53b6b1;
  background: linear-gradient(90deg, rgba(83, 182, 177, 0.28), rgba(83, 182, 177, 0.08));
}

.file-bq-highlighted :deep(.file-bq-indent.indent-level-3) {
  border-left-color: #9d88e6;
  background: linear-gradient(90deg, rgba(157, 136, 230, 0.26), rgba(157, 136, 230, 0.08));
}

.file-bq-highlighted :deep(.file-bq-indent.indent-level-4) {
  border-left-color: #f2a65a;
  background: linear-gradient(90deg, rgba(242, 166, 90, 0.28), rgba(242, 166, 90, 0.08));
}

.file-bq-highlighted :deep(.file-bq-indent.indent-level-5) {
  border-left-color: #e97a9b;
  background: linear-gradient(90deg, rgba(233, 122, 155, 0.26), rgba(233, 122, 155, 0.08));
}

.file-bq-highlighted :deep(.file-bq-indent.indent-level-6) {
  border-left-color: #7cb342;
  background: linear-gradient(90deg, rgba(124, 179, 66, 0.26), rgba(124, 179, 66, 0.08));
}

.file-bq-token,
.file-bq-highlighted :deep(.file-bq-token) {
  font-weight: 700;
  display: inline-block;
  border-radius: 4px;
  padding: 0 2px;
  border: 1px solid transparent;
  line-height: 1.08;
}

.file-bq-highlighted :deep(.file-bq-token) {
  font-weight: 400;
  display: inline;
  padding: 0;
  border: none;
  border-radius: 0;
  line-height: inherit;
}

.file-bq-token.token-delimiter,
.file-bq-highlighted :deep(.file-bq-token.token-delimiter) {
  color: #5d6874;
  background: #edf1f5;
  border-color: #c8d1da;
}

.file-bq-token.token-fn,
.file-bq-highlighted :deep(.file-bq-token.token-fn) {
  color: #00448a;
  background: #d8ebff;
  border-color: #8cbdf2;
}

.file-bq-token.token-m,
.file-bq-highlighted :deep(.file-bq-token.token-m) {
  color: #5e22c7;
  background: #eadcff;
  border-color: #bc97f2;
}

.file-bq-token.token-s,
.file-bq-highlighted :deep(.file-bq-token.token-s) {
  color: #005f89;
  background: #d9f1ff;
  border-color: #8ec6e8;
}

.file-bq-token.token-f,
.file-bq-highlighted :deep(.file-bq-token.token-f) {
  color: #7c3200;
  background: #ffe7d6;
  border-color: #efb189;
}

.file-bq-token.token-l,
.file-bq-highlighted :deep(.file-bq-token.token-l) {
  color: #8c0f4a;
  background: #ffddeb;
  border-color: #ef9dbc;
}

.file-bq-token.token-im,
.file-bq-highlighted :deep(.file-bq-token.token-im) {
  color: #734600;
  background: #ffedc7;
  border-color: #e8be72;
}

/* datagrid 内に v-html で挿入された token スパンにも色を適用する */
.file-bq-grid :deep(.file-bq-token.token-delimiter) {
  color: #5d6874;
  background: #edf1f5;
  border-color: #c8d1da;
}
.file-bq-grid :deep(.file-bq-token.token-fn) {
  color: #00448a;
  background: #d8ebff;
  border-color: #8cbdf2;
}
.file-bq-grid :deep(.file-bq-token.token-m) {
  color: #5e22c7;
  background: #eadcff;
  border-color: #bc97f2;
}
.file-bq-grid :deep(.file-bq-token.token-s) {
  color: #005f89;
  background: #d9f1ff;
  border-color: #8ec6e8;
}
.file-bq-grid :deep(.file-bq-token.token-f) {
  color: #7c3200;
  background: #ffe7d6;
  border-color: #efb189;
}
.file-bq-grid :deep(.file-bq-token.token-l) {
  color: #8c0f4a;
  background: #ffddeb;
  border-color: #ef9dbc;
}
.file-bq-grid :deep(.file-bq-token.token-im) {
  color: #734600;
  background: #ffedc7;
  border-color: #e8be72;
}
.file-bq-grid :deep(.file-bq-token.token-unknown) {
  color: #b42318;
  background: #ffe2df;
  border-color: #f1aaa4;
}

.file-bq-token.token-if,
.file-bq-highlighted :deep(.file-bq-token.token-if) {
  color: #2f5f12;
  background: #e1f4d6;
  border-color: #a5cf8b;
  box-shadow: inset 0 -1px 0 rgba(39, 79, 16, 0.18);
  text-decoration: underline;
  text-decoration-thickness: 1.5px;
  text-underline-offset: 1px;
}

.file-bq-token.token-ee,
.file-bq-highlighted :deep(.file-bq-token.token-ee) {
  color: #1f469f;
  background: #dce6ff;
  border-color: #98afef;
  box-shadow: inset 0 -1px 0 rgba(26, 55, 121, 0.18);
}

.file-bq-token.token-ei,
.file-bq-highlighted :deep(.file-bq-token.token-ei) {
  color: #76222b;
  background: #ffe0e4;
  border-color: #e9a2ab;
  box-shadow: inset 0 -1px 0 rgba(95, 25, 35, 0.18);
  text-decoration: underline;
  text-decoration-thickness: 1.5px;
  text-underline-offset: 1px;
}

.file-bq-token.token-unknown,
.file-bq-highlighted :deep(.file-bq-token.token-unknown) {
  color: #b42318;
  background: #ffe2df;
  border-color: #f1aaa4;
}

.file-bq-highlighted :deep(.file-bq-token.token-error),
.file-bq-highlighted :deep(.file-bq-token.token-delimiter.token-error),
.file-bq-highlighted :deep(.file-bq-token.token-if.token-error),
.file-bq-highlighted :deep(.file-bq-token.token-ee.token-error),
.file-bq-highlighted :deep(.file-bq-token.token-ei.token-error) {
  background: #ffe5e2;
  color: #8f1d14;
  box-shadow: inset 0 -2px 0 #d92d20;
}


</style>
