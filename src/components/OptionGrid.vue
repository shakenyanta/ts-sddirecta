<script setup lang="ts">
import { computed } from 'vue';

const props = defineProps<{ files: { filename: string; description: string; individualInfo: string }[] }>();
const rows = computed(() => Array.from(
    { length: Math.max(23, props.files.length) },
    (_, index) => ({
        filename: props.files[index]?.filename ?? '',
        description: props.files[index]?.description ?? '',
        individualInfo: props.files[index]?.individualInfo ?? '',
    }),
));

const columns = [
    { label: 'No', width: 24 },
    { label: 'ファイル名', width: 68 },
    { label: '説明' },
    { label: '個別情報', width: 168 },
    { label: '更新No', width: 48 },
    { label: '更新Ver', width: 52 },
    { label: '条件', width: 30 },
    { label: '車種', width: 40 },
];
</script>

<template>
    <div class="option-grid">
        <div class="grid-scroll">
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
                <tr v-for="(file, index) in rows" :key="index">
                    <td v-for="column in columns" :key="column.label"
                        :title="column.label === '個別情報' ? file.individualInfo || undefined : column.label === '説明' ? file.description || undefined : column.label === 'ファイル名' ? file.filename || undefined : undefined">
                        <template v-if="column.label === 'No'">{{ index + 1 }}</template>
                        <template v-else-if="column.label === 'ファイル名'">{{ file.filename }}</template>
                        <template v-else-if="column.label === '説明'">{{ file.description }}</template>
                        <template v-else-if="column.label === '個別情報'">{{ file.individualInfo }}</template>
                    </td>
                </tr>
            </tbody>
        </table>
        </div>
    </div>
</template>

<style scoped>
.option-grid {
    width: 100%;
    height: 100%;
    padding: 6px;
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
}
table {
    width: 100%;
    min-width: 520px;
    table-layout: fixed;
    border-collapse: separate;
    border-spacing: 0;
}
th, td {
    height: 18px;
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
