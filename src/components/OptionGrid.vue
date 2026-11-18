<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch, watchEffect } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { formatFileTimestamp } from '../utils/fileTimestamp';
import { useGridColumnResize } from '../utils/gridColumnResize';

const props = defineProps<{ files: {
    filename: string; description: string; individualInfo: string; modifiedAt?: number | null; size?: number | null; csvSize?: number;
    startAddress?: string; endAddress?: string;
    updateNo?: string; updateVer?: string; condition?: string; carType?: string;
}[]; directory: string; disabled?: boolean; variant?: 'files' | 'output' }>();
const emit = defineEmits<{ openFile: [filename: string]; analyzeDailyReport: [filename: string] }>();

function formatFileSize(bytes: number): string {
    const units = ['バイト', 'KB', 'MB', 'GB', 'TB', 'PB', 'EB'];
    let value = bytes;
    let unit = 0;
    while (value >= 1024 && unit < units.length - 1) {
        value /= 1024;
        unit++;
    }
    return `${value.toLocaleString('ja-JP', { maximumFractionDigits: 2 })}${units[unit]}`;
}

function descriptionTooltip(file: typeof props.files[number]): string | undefined {
    const size = file.size == null ? '' : formatFileSize(file.size);
    const mismatch = file.size != null && file.csvSize != null && file.size !== file.csvSize;
    const csvSize = mismatch ? formatFileSize(file.csvSize!) : '';
    const showBytes = mismatch && size === csvSize;
    const exactBytes = (bytes: number) => ` (${bytes.toLocaleString('ja-JP')}バイト)`;
    return [
        [file.startAddress, file.endAddress].filter(Boolean).join(','),
        size + (showBytes ? exactBytes(file.size!) : ''),
        mismatch ? `[CSV: ${csvSize}${showBytes ? exactBytes(file.csvSize!) : ''}]` : '',
    ].filter(Boolean).join(' ') || undefined;
}
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
const { resizedWidths, resizing, startResize, resizeColumn, stopResize } = useGridColumnResize();

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
    ...(props.variant === 'output' ? [
        { label: '詳細', width: 186 },
    ] : [
        { label: '更新No', width: 48 },
        { label: '更新Ver', width: 68 },
        { label: '条件', width: 30 },
        { label: '車種', width: 40 },
    ]),
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
                        :title="column.label === '個別情報' ? (isImFile(file.filename) ? undefined : file.individualInfo || undefined) : column.label === '説明' ? descriptionTooltip(file) : column.label === 'ファイル名' ? [file.filename, formatFileTimestamp(file.modifiedAt)].filter(Boolean).join(' ') : undefined">
                        <template v-if="column.label === 'No'">{{ index + 1 }}</template>
                        <template v-else-if="column.label === 'ファイル名'">{{ file.filename }}</template>
                        <template v-else-if="column.label === '説明'">{{ file.description }}</template>
                        <template v-else-if="column.label === '個別情報'">{{ file.individualInfo }}</template>
                        <template v-else-if="column.label === '詳細'">
                            <button v-if="file.description === '自動日報2データ'" type="button"
                                class="daily-report-button" :disabled="disabled"
                                @click.stop="!disabled && emit('analyzeDailyReport', file.filename)">日報解析</button>
                        </template>
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

<style scoped src="../styles/fileGrid.css"></style>

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

.filename-cell { user-select: none; }
.daily-report-button {
    height: 16px;
    padding: 0 6px;
    border: 1px solid #b8cce5;
    border-radius: 3px;
    background: #edf4fd;
    color: #255b9f;
    font: inherit;
    line-height: 14px;
    vertical-align: middle;
    cursor: pointer;
}
.daily-report-button:hover:not(:disabled) { background: #dbeafe; }
.daily-report-button:disabled { opacity: .45; cursor: default; }
tbody td.selected-cell {
    background: #dbeafe;
    box-shadow: inset 0 0 0 1px #60a5fa;
    color: #1e40af;
}
</style>
