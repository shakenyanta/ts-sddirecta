<script setup lang="ts">
import { computed, onMounted, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import ArrowButton from '../components/ArrowButton.vue';
import DailyReportGrid from '../components/DailyReportGrid.vue';
import { features, initializeEdition } from '../editions';
import { dailyReportRows, parseDailyReport, type DailyReportResult } from '../utils/dailyReport';

const props = defineProps<{ filename: string }>();
const bytes = ref(new Uint8Array());
const report = ref<DailyReportResult>({ records: [], warnings: [], checksum: null });
const recordIndex = ref(0);
const stopIndex = ref(0);
const loading = ref(true);
const error = ref('');
const record = computed(() => report.value.records[recordIndex.value]);
const rows = computed(() => dailyReportRows(bytes.value, record.value, stopIndex.value));

function changeRecord(index: number) {
    if (loading.value || index < 0 || index >= report.value.records.length) return;
    recordIndex.value = index;
    stopIndex.value = 0;
}
function changeStop(index: number) {
    if (loading.value || !record.value || index < 0 || index > record.value.stopCount) return;
    stopIndex.value = index;
}

onMounted(async () => {
    try {
        await initializeEdition();
        if (!features.value.optionPanel) throw new Error('日報解析はプロ版専用です。');
        if (!props.filename) throw new Error('解析するファイル名が指定されていません。');
        bytes.value = new Uint8Array(await invoke<number[]>('read_daily_report', { filename: props.filename }));
        report.value = parseDailyReport(bytes.value);
        if (!report.value.records.length) error.value = '表示できる営業データがありません。';
        if (report.value.warnings.length) {
            console.warn('日報解析', { filename: props.filename, warnings: report.value.warnings });
        }
    } catch (reason) {
        error.value = String(reason);
    } finally {
        loading.value = false;
    }
});
</script>

<template>
    <main class="daily-report">
        <header class="report-toolbar">
            <h1>CBフォーマット</h1>
            <div class="record-controls" aria-label="営業切替">
                <span>営業</span>
                <ArrowButton direction="up" label="前の営業を表示" :disabled="loading || recordIndex <= 0"
                    @click="changeRecord(recordIndex - 1)" />
                <ArrowButton direction="down" label="次の営業を表示" :disabled="loading || recordIndex >= report.records.length - 1"
                    @click="changeRecord(recordIndex + 1)" />
                <span class="record-position" aria-live="polite">{{ record ? recordIndex + 1 : 0 }} / {{ report.records.length }}</span>
            </div>
        </header>
        <div class="report-messages" :class="{ empty: !loading && !error && !report.warnings.length }">
            <p v-if="loading" role="status">日報ファイルを読み込んでいます…</p>
            <p v-if="error" role="alert">{{ error }}</p>
            <p v-for="warning in report.warnings" :key="warning" role="alert">{{ warning }}</p>
        </div>
        <DailyReportGrid :rows="rows" :stop-index="stopIndex" :stop-count="record?.stopCount ?? 0"
            :disabled="loading || !record" @change-stop="changeStop" />
        <footer>
            <span aria-live="polite">表示：{{ stopIndex ? `立寄${stopIndex}` : '降車' }} ／ 立ち寄り {{ record?.stopCount ?? 0 }}件</span>
            <span v-if="report.checksum">チェックサム候補：{{ report.checksum }}（計算方式未確定）</span>
        </footer>
    </main>
</template>

<style scoped>
.daily-report { display: flex; flex-direction: column; height: 100%; padding: 10px; gap: 8px; background: #f7f9fc; color: #334155; font: 12px/1.5 'Yu Gothic UI', Meiryo, sans-serif; }
.report-toolbar { display: flex; align-items: center; flex-shrink: 0; gap: 12px; min-height: 28px; }
h1 { flex-shrink: 0; color: #246b45; font-size: 14px; font-weight: 600; }
.record-controls { display: flex; align-items: center; gap: 6px; margin-left: auto; flex-shrink: 0; }
.record-position { min-width: 68px; text-align: right; font-variant-numeric: tabular-nums; }
.report-messages { flex-shrink: 0; max-height: 96px; overflow: auto; padding: 6px 8px; border: 1px solid #e7d5aa; border-radius: 4px; background: #fff8e8; color: #795821; }
.report-messages.empty { display: none; }
.daily-report > :deep(.report-grid) { flex: 1; min-height: 0; height: auto; }
footer { display: flex; flex-wrap: wrap; justify-content: space-between; gap: 4px 16px; flex-shrink: 0; color: #52647c; font-size: 11px; }
</style>
