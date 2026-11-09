<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch, watchEffect } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { formatFileTimestamp } from '../utils/fileTimestamp';
import { formatByteCount } from '../utils/sdWrite';

const props = defineProps<{ files: {
    filename: string; description: string; individualInfo: string; modifiedAt?: number | null; size?: number | null; csvSize?: number;
    updateNo?: string; updateVer?: string; condition?: string; carType?: string;
}[]; directory: string; disabled?: boolean }>();
const emit = defineEmits<{ openFile: [filename: string] }>();
const selectedRow = ref<number | null>(null);
watch(() => props.files, () => { selectedRow.value = null; });

const imPreview = ref<{ x: number; y: number; error: string } | null>(null);
const imCanvas = ref<HTMLCanvasElement | null>(null);
let hoverTimer: ReturnType<typeof setTimeout> | undefined;
let hoverRequest = 0;
const isImFile = (filename: string) => /^IM(?:\.[^./\\:]+)?$/i.test(filename);

function hideImPreview() {
    clearTimeout(hoverTimer);
    hoverRequest++;
    imPreview.value = null;
}

function hoverCell(event: MouseEvent, column: string, filename: string) {
    hideImPreview();
    if (column !== '個別情報' || !isImFile(filename) || props.disabled || !props.directory) return;
    const request = hoverRequest;
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
    const x = Math.max(8, Math.min(rect.left, window.innerWidth - 402));
    const y = rect.bottom + 208 <= window.innerHeight ? rect.bottom + 6 : Math.max(8, rect.top - 206);
    hoverTimer = setTimeout(async () => {
        try {
            const bytes = await invoke<number[]>('read_im_preview', { directory: props.directory, filename });
            if (request !== hoverRequest) return;
            imPreview.value = { x, y, error: '' };
            await nextTick();
            if (request !== hoverRequest) return;
            const context = imCanvas.value?.getContext('2d');
            if (!context) throw new Error('IM画像を描画できませんでした');
            const image = context.createImageData(384, 192);
            for (let pixel = 0; pixel < 384 * 192; pixel++) {
                const color = (bytes[pixel >> 3] & (0x80 >> (pixel % 8))) !== 0 ? 0 : 255;
                image.data.set([color, color, color, 255], pixel * 4);
            }
            context.putImageData(image, 0, 0);
        } catch (error) {
            if (request === hoverRequest) imPreview.value = { x, y, error: String(error) };
        }
    }, 350);
}

watch(() => [props.files, props.directory, props.disabled], hideImPreview);
onBeforeUnmount(hideImPreview);

function selectFile(index: number) {
    if (!props.disabled) selectedRow.value = index;
}

function openFile(index: number) {
    if (props.disabled) return;
    selectedRow.value = index;
    emit('openFile', props.files[index].filename);
}
const grid = ref<HTMLElement | null>(null);
const filenameWidth = ref(68);
const resizedWidths = ref<number[] | null>(null);
const resizing = ref(false);
let resizeState: { index: number; pointerId: number; startX: number; startWidth: number } | null = null;

function startResize(event: PointerEvent, index: number) {
    if (event.button !== 0 || resizeState) return;
    const handle = event.currentTarget as HTMLElement;
    const header = handle.closest('tr');
    if (!header) return;
    // 自動配分されていた説明列も実際の幅で固定し、ほかの列の幅を保つ。
    const widths = Array.from(header.cells, cell => cell.getBoundingClientRect().width);
    resizedWidths.value = widths;
    resizeState = { index, pointerId: event.pointerId, startX: event.clientX, startWidth: widths[index] };
    resizing.value = true;
    handle.setPointerCapture(event.pointerId);
    event.preventDefault();
}

function resizeColumn(event: PointerEvent) {
    if (!resizeState || event.pointerId !== resizeState.pointerId || !resizedWidths.value) return;
    resizedWidths.value[resizeState.index] = Math.max(24,
        resizeState.startWidth + event.clientX - resizeState.startX);
}

function stopResize(event: PointerEvent) {
    if (!resizeState || event.pointerId !== resizeState.pointerId) return;
    resizeState = null;
    resizing.value = false;
    const handle = event.currentTarget as HTMLElement;
    if (handle.hasPointerCapture(event.pointerId)) handle.releasePointerCapture(event.pointerId);
}

onMounted(() => {
    const context = document.createElement('canvas').getContext('2d');
    if (!context || !grid.value) return;
    context.font = getComputedStyle(grid.value).font;

    watchEffect(() => {
        let width = 68;
        for (const file of props.files) {
            // 左右の余白6pxと罫線1pxも含め、端数を切り上げる。
            width = Math.max(width, Math.ceil(context.measureText(file.filename).width) + 7);
        }
        filenameWidth.value = width;
    });
});

const columns = computed(() => [
    { label: 'No', width: 24 },
    { label: 'ファイル名', width: filenameWidth.value },
    { label: '説明' },
    { label: '個別情報', width: 168 },
    { label: '更新No', width: 48 },
    { label: '更新Ver', width: 68 },
    { label: '条件', width: 30 },
    { label: '車種', width: 40 },
].map((column, index) => ({ ...column, width: resizedWidths.value?.[index] ?? column.width })));
// 説明列に最低80pxを残し、収まらない場合は横スクロールする。
const tableMinWidth = computed(() => Math.max(520,
    columns.value.reduce((width, column) => width + (column.width ?? 80), 0),
));
const contentStyle = computed(() => resizedWidths.value
    ? { width: `${resizedWidths.value.reduce((sum, width) => sum + width, 0)}px` }
    : { minWidth: `${tableMinWidth.value}px` });
// 表と余白の縦罫線は、同じ列定義から幅を決める。
const backgroundColumns = computed(() => columns.value
    .map(column => column.width ? `${column.width}px` : 'minmax(0, 1fr)').join(' '));
</script>

<template>
    <div ref="grid" class="option-grid" :class="{ resizing }">
        <div class="grid-scroll" @scroll="hideImPreview">
        <div class="grid-content" :style="contentStyle">
        <table aria-label="ファイル情報">
            <colgroup>
                <col v-for="column in columns" :key="column.label" :style="{ width: column.width ? `${column.width}px` : undefined }">
            </colgroup>
            <thead>
                <tr>
                    <th v-for="(column, index) in columns" :key="column.label" scope="col">
                        <div class="header-content"><span>{{ column.label }}</span></div>
                        <span class="column-resizer" aria-hidden="true" :title="`${column.label}の列幅を調整`"
                            @pointerdown.stop="startResize($event, index)"
                            @pointermove="resizeColumn"
                            @pointerup="stopResize"
                            @pointercancel="stopResize"
                            @lostpointercapture="stopResize"></span>
                    </th>
                </tr>
            </thead>
            <tbody>
                <tr v-for="(file, index) in files" :key="index">
                    <td v-for="column in columns" :key="column.label"
                        :class="{ 'filename-cell': column.label === 'ファイル名', 'selected-cell': column.label === 'ファイル名' && selectedRow === index }"
                        @mouseenter="hoverCell($event, column.label, file.filename)"
                        @mouseleave="hideImPreview"
                        @click="column.label === 'ファイル名' && selectFile(index)"
                        @dblclick="column.label === 'ファイル名' && openFile(index)"
                        :title="column.label === '個別情報' ? (isImFile(file.filename) ? undefined : file.individualInfo || undefined) : column.label === '説明' ? file.description || undefined : column.label === 'ファイル名' ? [file.filename, formatFileTimestamp(file.modifiedAt), file.size == null ? '' : `${formatByteCount(file.size)}バイト`, file.size != null && file.csvSize != null && file.size !== file.csvSize ? `[CSV: ${formatByteCount(file.csvSize)}バイト]` : ''].filter(Boolean).join(' ') : undefined">
                        <template v-if="column.label === 'No'">{{ index + 1 }}</template>
                        <template v-else-if="column.label === 'ファイル名'">{{ file.filename }}</template>
                        <template v-else-if="column.label === '説明'">{{ file.description }}</template>
                        <template v-else-if="column.label === '個別情報'">{{ file.individualInfo }}</template>
                        <template v-else-if="column.label === '更新No'">{{ file.updateNo }}</template>
                        <template v-else-if="column.label === '更新Ver'">{{ file.updateVer }}</template>
                        <template v-else-if="column.label === '条件'">{{ file.condition }}</template>
                        <template v-else-if="column.label === '車種'">{{ file.carType }}</template>
                    </td>
                </tr>
            </tbody>
        </table>
        <div class="grid-background" aria-hidden="true"
            :class="{ 'starts-even': files.length % 2 === 1 }"
            :style="{ gridTemplateColumns: backgroundColumns }">
            <div v-for="column in columns" :key="column.label"></div>
        </div>
        </div>
        </div>
    </div>
    <Teleport to="body">
        <div v-if="imPreview" class="im-preview" role="tooltip" :style="{ left: `${imPreview.x}px`, top: `${imPreview.y}px` }">
            <span v-if="imPreview.error">{{ imPreview.error }}</span>
            <canvas v-else ref="imCanvas" width="384" height="192" role="img" aria-label="IM社名の先頭ビットマップ"></canvas>
        </div>
    </Teleport>
</template>

<style scoped>
.im-preview {
    position: fixed;
    z-index: 1000;
    pointer-events: none;
    padding: 5px;
    max-width: calc(100vw - 16px);
    background: white;
    color: #334155;
    border: 1px solid #cbd5e1;
    border-radius: 4px;
    box-shadow: 0 3px 12px #0003;
    font-size: 12px;
}
.im-preview canvas { display: block; max-width: 100%; height: auto; }

.option-grid {
    --grid-row-height: 18px;
    width: 100%;
    height: 100%;
    padding: 0 3px 3px 0;
    background: #fff;
    color: #334155;
    font: 11px/16px 'Yu Gothic UI', Meiryo, sans-serif;
}
.grid-scroll {
    width: 100%;
    height: 100%;
    overflow: auto;
    scrollbar-width: thin;
    border: 1px solid #dce2eb;
    border-radius: 4px;
    border-top-left-radius: 0;
}
.grid-content {
    display: flex;
    flex-direction: column;
    min-height: 100%;
}
.grid-background {
    /* 空行は作らず、残りの高さだけを罫線・縞模様で埋める。 */
    display: grid;
    flex: 1;
    --first-row-color: #fff;
    --second-row-color: #f5f9fe;
    background: repeating-linear-gradient(to bottom,
        var(--first-row-color) 0 calc(var(--grid-row-height) - 1px),
        #e8edf4 calc(var(--grid-row-height) - 1px) var(--grid-row-height),
        var(--second-row-color) var(--grid-row-height) calc(2 * var(--grid-row-height) - 1px),
        #e8edf4 calc(2 * var(--grid-row-height) - 1px) calc(2 * var(--grid-row-height)));
}
.grid-background.starts-even {
    --first-row-color: #f5f9fe;
    --second-row-color: #fff;
}
.grid-background > div:not(:last-child) { border-right: 1px solid #edf1f6; }
table {
    flex-shrink: 0;
    width: 100%;
    table-layout: fixed;
    border-collapse: separate;
    border-spacing: 0;
}
th, td {
    height: var(--grid-row-height);
    padding: 0 3px;
    border-right: 1px solid #edf1f6;
    border-bottom: 1px solid #e8edf4;
    text-align: center;
    vertical-align: middle;
    white-space: nowrap;
    overflow: hidden;
}
th {
    position: sticky;
    top: 0;
    z-index: 1;
    height: var(--option-grid-header-height, 24px);
    background: #edf4fd;
    color: #255b9f;
    border-bottom-color: #d7e5f5;
    font-weight: 600;
}
.header-content { overflow: hidden; }
.column-resizer {
    position: absolute;
    top: 0;
    right: 0;
    bottom: 0;
    width: 7px;
    cursor: col-resize;
    touch-action: none;
    user-select: none;
}
.column-resizer:hover { background: #93c5fd; }
.option-grid.resizing, .option-grid.resizing * {
    cursor: col-resize;
    user-select: none;
}
th:last-child, td:last-child { border-right: 0; }
tbody tr:nth-child(odd) { background: #fff; }
tbody tr:nth-child(even) { background: #f5f9fe; }
tbody tr:hover { background: #eaf3ff; }
tbody td:nth-child(2), tbody td:nth-child(3) { text-align: left; }
tbody td { text-overflow: ellipsis; }
.filename-cell { user-select: none; }
tbody td.selected-cell {
    background: #dbeafe;
    box-shadow: inset 0 0 0 1px #60a5fa;
    color: #1e40af;
}
</style>
