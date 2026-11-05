<script setup lang="ts">
import { computed, onMounted, ref, watchEffect } from 'vue';

const props = defineProps<{ files: {
    filename: string; description: string; individualInfo: string;
    updateNo?: string; updateVer?: string; condition?: string; carType?: string;
}[] }>();
const grid = ref<HTMLElement | null>(null);
const filenameWidth = ref(68);

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
]);
// 説明列に最低80pxを残し、収まらない場合は横スクロールする。
const tableMinWidth = computed(() => Math.max(520,
    columns.value.reduce((width, column) => width + (column.width ?? 80), 0),
));
// 表と余白の縦罫線は、同じ列定義から幅を決める。
const backgroundColumns = computed(() => columns.value
    .map(column => column.width ? `${column.width}px` : 'minmax(0, 1fr)').join(' '));
</script>

<template>
    <div ref="grid" class="option-grid">
        <div class="grid-scroll">
        <div class="grid-content" :style="{ minWidth: `${tableMinWidth}px` }">
        <table aria-label="ファイル情報">
            <colgroup>
                <col v-for="column in columns" :key="column.label" :style="{ width: column.width ? `${column.width}px` : undefined }">
            </colgroup>
            <thead>
                <tr>
                    <th v-for="column in columns" :key="column.label" scope="col">
                        <div class="header-content"><span>{{ column.label }}</span></div>
                    </th>
                </tr>
            </thead>
            <tbody>
                <tr v-for="(file, index) in files" :key="index">
                    <td v-for="column in columns" :key="column.label"
                        :title="column.label === '個別情報' ? file.individualInfo || undefined : column.label === '説明' ? file.description || undefined : column.label === 'ファイル名' ? file.filename || undefined : undefined">
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
</template>

<style scoped>
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
    height: 24px;
    background: #edf4fd;
    color: #255b9f;
    border-bottom-color: #d7e5f5;
    font-weight: 600;
}
.header-content { overflow: hidden; }
th:last-child, td:last-child { border-right: 0; }
tbody tr:nth-child(odd) { background: #fff; }
tbody tr:nth-child(even) { background: #f5f9fe; }
tbody tr:hover { background: #eaf3ff; }
tbody td:nth-child(2), tbody td:nth-child(3) { text-align: left; }
tbody td { text-overflow: ellipsis; }
</style>
