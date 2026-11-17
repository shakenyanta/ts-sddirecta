<script setup lang="ts">
import { computed } from 'vue';
import ArrowButton from './ArrowButton.vue';
import { useGridColumnResize } from '../utils/gridColumnResize';

defineProps<{
    rows: { name: string; size: number; content: string; detail: string }[];
    stopIndex: number;
    stopCount: number;
    disabled?: boolean;
}>();
const emit = defineEmits<{ changeStop: [index: number] }>();
const { resizedWidths, resizing, startResize, resizeColumn, stopResize } = useGridColumnResize();
const columns = computed(() => [
    { label: '項番', width: 40 },
    { label: '桁数', width: 40 },
    { label: '名称', width: 256 },
    { label: '内容', width: 200 },
    { label: '詳細', width: 200 },
].map((column, index) => ({ ...column, width: resizedWidths.value?.[index] ?? column.width })));
const tableWidth = computed(() => columns.value.reduce((sum, column) => sum + column.width, 0));
</script>

<template>
    <div class="option-grid report-grid" :class="{ resizing }" :style="{ maxWidth: `${tableWidth + 2}px` }">
        <div class="grid-scroll">
                <div class="grid-content" :style="{ width: `${tableWidth}px` }">
                    <table aria-label="日報解析">
                        <colgroup>
                            <col v-for="column in columns" :key="column.label" :style="{ width: `${column.width}px` }">
                        </colgroup>
                        <thead>
                            <tr>
                                <th v-for="(column, index) in columns" :key="column.label" scope="col">
                                    <div class="header-content">{{ column.label }}</div>
                                    <span class="column-resizer" aria-hidden="true" :title="`${column.label}の列幅を調整`"
                                        @pointerdown.stop="startResize($event, index)" @pointermove="resizeColumn"
                                        @pointerup="stopResize" @pointercancel="stopResize" @lostpointercapture="stopResize"></span>
                                </th>
                            </tr>
                        </thead>
                        <tbody>
                            <tr v-for="(row, index) in rows" :key="index" :class="{ 'stop-row': index >= 4 && index <= 6 }">
                                <td>{{ index + 1 }}</td>
                                <td>{{ row.size }}</td>
                                <td :title="row.name">
                                    <div class="name-cell">
                                        <span class="field-name">{{ row.name }}</span>
                                        <div v-if="index === 4 && stopCount > 0" class="stop-controls" role="group" aria-label="立寄切替">
                                            <span class="stop-label">立寄り</span>
                                            <ArrowButton direction="up" :label="stopIndex === 1 ? '降車を表示' : '前の立寄を表示'"
                                                :disabled="disabled || stopIndex <= 0" @click="emit('changeStop', stopIndex - 1)" />
                                            <ArrowButton direction="down" label="次の立寄を表示" :disabled="disabled || stopIndex >= stopCount"
                                                @click="emit('changeStop', stopIndex + 1)" />
                                            <span class="stop-count">{{ stopIndex }} / {{ stopCount }}</span>
                                        </div>
                                    </div>
                                </td>
                                <td :title="row.content">{{ row.content }}</td>
                                <td :title="row.detail || undefined">{{ row.detail }}</td>
                            </tr>
                        </tbody>
                    </table>
                </div>
        </div>
    </div>
</template>

<style scoped src="../styles/fileGrid.css"></style>
<style scoped>
.report-grid { padding: 0; }
.grid-scroll { height: auto; max-height: 100%; }
.grid-content { flex-shrink: 0; min-height: 0; }
tbody td:nth-child(2) { text-align: center; }
tbody td:nth-child(3), tbody td:nth-child(4), tbody td:nth-child(5) { text-align: left; }
tbody td:nth-child(4) { font-family: Consolas, monospace; }
.name-cell { display: flex; align-items: center; gap: 6px; }
.field-name { min-width: 0; overflow: hidden; text-overflow: ellipsis; }
.stop-controls { display: flex; align-items: center; flex-shrink: 0; gap: 4px; margin-left: auto; white-space: nowrap; color: #52647c; }
.stop-label { padding: 0 4px; border-radius: 3px; background: #dbeafe; color: #1e40af; font-weight: 700; }
.stop-controls :deep(.arrow-button) { width: 24px; height: 16px; }
.stop-count { font-variant-numeric: tabular-nums; }
.stop-row td:nth-child(3) { color: #255b9f; }
</style>
