<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import SdMenu from '../components/SdMenu.vue';
import { splitFolderPath } from '../utils/executablePath';
import { createLogScroller } from '../utils/logScroll';
import { parseSdDirectCsv, type AddressRange, type SdArea } from '../utils/sdDirectCsv';
import { writePercent, writeStageLabel, sizeComment, formatByteCount, type WritePlan, type WriteProgress, type OperationStatus } from '../utils/sdWrite';

const menuHeight = ref(48);
const minimumMenuHeight = ref(48);
const menuContent = ref<HTMLElement | null>(null);
let menuObserver: ResizeObserver | undefined;
const sdHeight = ref(120);
const sourceDirectory = ref('');
const dragging = ref(false);
const initializing = ref(false);
const writing = ref(false);
const closing = ref(false);
const writeStage = ref('');
const currentRun = ref('');
const progress = ref<WriteProgress | null>(null);
const pendingPlan = ref<WritePlan | null>(null);
const writeDialog = ref<HTMLDialogElement | null>(null);
const writeDialogOpen = ref(false);
const writeLog = ref<PanelMessage[]>([]);
const writeLogPanel = ref<HTMLElement | null>(null);
const writeLogScroller = createLogScroller(() => writeLogPanel.value);
const writeTotal = ref(0);
const copyStatus = ref('');
const popupTitle = computed(() => writing.value ? `${statusLabel.value}…`
    : writeStage.value === 'done' ? '書込完了' : writeStage.value === 'failed' ? '書込失敗' : '書込対象なし');
let progressUnlisten: UnlistenFn | undefined;
let operationUnlisten: UnlistenFn | undefined;
const operationStatus = ref<OperationStatus | null>(null);
let operationAnchor = 0;
const operationDetail = computed(() => {
    if (!operationStatus.value) return '';
    // The elapsed timer keeps this reactive even while a device call is blocked.
    void elapsedMs.value;
    const ms = Math.max(0, performance.now() - operationAnchor);
    return `${operationStatus.value.detail} / ${ms >= 5000 ? 'デバイス応答待ち・' : ''}${Math.floor(ms / 1000)}秒経過`;
});
let closeUnlisten: UnlistenFn | undefined;
const shutdownUnlisteners: UnlistenFn[] = [];
let elapsedTimer: ReturnType<typeof setInterval> | undefined;
let elapsedAnchor = 0;
const elapsedMs = ref(0);
const transientNotice = ref('');
let noticeTimer: ReturnType<typeof setTimeout> | undefined;
const percent = computed(() => writePercent(progress.value));
const statusLabel = computed(() => writeStageLabel(writeStage.value));
const writeFileCount = computed(() => [...sdloadingDictionary.value.values()].filter(area => area.accessFlag === 'A' || area.accessFlag === 'W').length);
const allFileCount = computed(() => [...sdloadingDictionary.value.values()].filter(area => area.accessFlag !== 'E').length);
let unlistenDrop: UnlistenFn | undefined;
let generation = 0;
let ready = false;
const useInputDirectory = ref(false);
const displayedDirectory = computed(() => useInputDirectory.value
	? `${sourceDirectory.value.replace(/[\\/]+$/, '')}\\input`
	: sourceDirectory.value);
const pathError = ref('');
type PanelMessage = string | { text: string; warning: true };
const messages = ref<PanelMessage[]>([]);
const sdDirectDictionary = ref(new Map<string, SdArea>());
const sdloadingDictionary = ref(new Map<string, SdArea>());
const sdAddressRange = ref<AddressRange | null>(null);
const matchedFiles = ref<string[]>([]);
const warningFiles = ref<string[]>([]);
const canSdWrite = computed(() => !initializing.value && !writing.value && !closing.value && allFileCount.value > 0);
let matchingGeneration: number | undefined;
let disposed = false;
let lastMatchError = '';
const textPanel = ref<HTMLElement | null>(null);
const pathList = ref<HTMLElement | null>(null);
const folders = computed(() => splitFolderPath(displayedDirectory.value));

onMounted(async () => {
	window.addEventListener('keydown', onRefreshKey, true);
	try {
		const unlisten = await listen<WriteProgress>('sd-progress', event => acceptProgress(event.payload));
		if (disposed) { unlisten(); return; }
		progressUnlisten = unlisten;
        const stopOperation = await listen<OperationStatus>('sd-operation', event => acceptOperation(event.payload));
        if (disposed) { stopOperation(); return; }
        operationUnlisten = stopOperation;
		const closeListener = await listen('sd-close-blocked', () => showNotice('SD処理中のため終了できません'));
		if (disposed) { closeListener(); return; }
		closeUnlisten = closeListener;
		for (const name of ['sd-shutdown-started', 'sd-shutdown-error']) {
			const stop = await listen<string>(name, event => {
				closing.value = true;
				if (name === 'sd-shutdown-error') void appendMessage({ text: `終了処理に失敗しました: ${event.payload}。もう一度終了操作を行ってください。`, warning: true });
			});
			if (disposed) { stop(); return; }
			shutdownUnlisteners.push(stop);
		}
	} catch (error) {
		await appendMessage(`SD進捗通知の開始に失敗しました: ${String(error)}`);
	}
	if (typeof ResizeObserver !== 'undefined' && menuContent.value) {
		menuObserver = new ResizeObserver(([entry]) => {
			minimumMenuHeight.value = Math.max(48, Math.ceil(entry.contentRect.height));
		});
		menuObserver.observe(menuContent.value);
	}
	try {
		const unlisten = await getCurrentWebview().onDragDropEvent(event => {
			if (writing.value || closing.value) {
				dragging.value = false;
				if (event.payload.type !== 'leave') showNotice(closing.value ? '終了処理中のため変更できません' : 'SD処理中のため変更できません');
				return;
			}
			dragging.value = event.payload.type === 'enter' || event.payload.type === 'over';
			if (event.payload.type === 'drop') void initializeSource(event.payload.paths);
		});
		if (disposed) { unlisten(); return; }
		unlistenDrop = unlisten;
	} catch (error) {
		await appendMessage(`ドロップ受付の開始に失敗しました: ${String(error)}`);
	}
	if (disposed) return;
	// A drop received during startup takes priority over the exe directory.
	if (generation === 0) await initializeSource();
});

function onRefreshKey(event: KeyboardEvent) {
	if (event.key !== 'F5') return;
	event.preventDefault();
	if (writing.value || closing.value) {
		event.stopImmediatePropagation();
		showNotice(closing.value ? '終了処理中のため再読込できません' : 'SD処理中のため再読込できません');
		return;
	}
	if (event.repeat || initializing.value || disposed) return;
	void initializeSource(undefined, sourceDirectory.value || undefined);
}

async function initializeSource(paths?: string[], refreshDirectory?: string, forWrite = false) {
	if (closing.value) return;
	if (writing.value && !forWrite) { showNotice('SD処理中のため変更できません'); return; }
	if (paths) messages.value = [];
	const current = ++generation;
	const active = () => !disposed && current === generation;
	initializing.value = true;
	ready = false;
	sdDirectDictionary.value = new Map();
	sdloadingDictionary.value = new Map();
	sdAddressRange.value = null;
	matchedFiles.value = [];
	warningFiles.value = [];
	useInputDirectory.value = false;
	sourceDirectory.value = '';
	pathError.value = '';
	lastMatchError = '';
	try {
		const directory = refreshDirectory ?? (paths
			? await invoke<string>('resolve_drop_directory', { paths })
			: await invoke<string>('get_executable_directory'));
		if (!active()) return;
		sourceDirectory.value = directory;
		if (refreshDirectory) await appendMessage(forWrite ? '書込前：対象情報を更新' : 'F5：表示を更新');
		else if (paths) await appendMessage('ドラッグ＆ドロップ');
		await nextTick();
		if (!active()) return;
		if (pathList.value) pathList.value.scrollTop = pathList.value.scrollHeight;
		let exists: boolean;
		try {
			exists = await invoke<boolean>('check_startup_csv', { directory });
		} catch (error) {
			if (!active()) return;
			await appendMessage(`sdDirectaxs.csv確認失敗: ${String(error)}`);
			return;
		}
		if (!active()) return;
		if (!exists) {
			await appendMessage('sdDirectaxs.csv無し');
			return;
		}
		try {
			const csv = await invoke<{ contents: string; source_folder: string }>('read_startup_csv', { directory });
			if (!active()) return;
			const parsed = parseSdDirectCsv(csv.contents);
			sdDirectDictionary.value = parsed.dictionary;
			sdAddressRange.value = parsed.addressRange;
			await appendMessage(`sdDirectaxs.csv読込完了 (${csv.source_folder})`, `dictionary作成：${sdDirectDictionary.value.size}件`);
		} catch (error) {
			if (!active()) return;
			await appendMessage(`sdDirectaxs.csv読込失敗: ${String(error)}`);
			return;
		}
		if (!active()) return;
		ready = true;
		await refreshMatchedFiles(true);
	} catch (error) {
		if (!active()) return;
		pathError.value = `対象フォルダーを取得できませんでした: ${String(error)}`;
		await appendMessage(pathError.value);
	} finally {
		if (active()) initializing.value = false;
	}
}

onUnmounted(() => {
    writeLogScroller.dispose();
	disposed = true;
	generation++;
	unlistenDrop?.();
	progressUnlisten?.();
    operationUnlisten?.();
	closeUnlisten?.();
	for (const stop of shutdownUnlisteners) stop();
	clearInterval(elapsedTimer);
	clearTimeout(noticeTimer);
	menuObserver?.disconnect();
	window.removeEventListener('keydown', onRefreshKey, true);
});

async function refreshMatchedFiles(report = false) {
	const current = generation;
	if (matchingGeneration === current || disposed || !ready) return;
	matchingGeneration = current;
	try {
		const result = await invoke<{ files: string[]; warning_files: string[]; from_input: boolean; input_exists: boolean }>('find_dictionary_files', {
			filenames: [...sdDirectDictionary.value.keys()],
			directory: sourceDirectory.value,
		});
		if (disposed || current !== generation) return;
		const warningsChanged = JSON.stringify(warningFiles.value) !== JSON.stringify(result.warning_files);
		const changed = warningsChanged || useInputDirectory.value !== result.from_input || JSON.stringify(matchedFiles.value) !== JSON.stringify(result.files);
		const matchedDictionary = new Map<string, SdArea>();
		for (const filename of result.files) {
			const range = sdDirectDictionary.value.get(filename);
			if (range) matchedDictionary.set(filename, { ...range });
		}
		sdloadingDictionary.value = matchedDictionary;
		matchedFiles.value = result.files;
		warningFiles.value = result.warning_files;
		useInputDirectory.value = result.from_input;
		lastMatchError = '';
		if (report) await appendMessage(result.input_exists ? 'inputフォルダあり' : 'inputフォルダ無し');
		if (disposed || current !== generation) return;
		if (!report && !changed) return;
		if (result.from_input && result.files.length) {
			useInputDirectory.value = true;
			await nextTick();
			if (disposed || current !== generation) return;
			if (pathList.value) pathList.value.scrollTop = pathList.value.scrollHeight;
			await appendMessage('inputフォルダへ移動');
			if (disposed || current !== generation) return;
		}
		if ((report || warningsChanged) && result.warning_files.length) {
			await appendMessage(...result.warning_files.map(filename => ({
				text: `警告：拡張子が異なるファイルがあります：${filename}`,
				warning: true as const,
			})));
			if (disposed || current !== generation) return;
		}
		if (!result.files.length) {
			await appendMessage('一致するファイルはありません');
		}
	} catch (error) {
		if (disposed || current !== generation) return;
		matchedFiles.value = [];
		warningFiles.value = [];
		sdloadingDictionary.value = new Map();
		const message = `ファイル照合失敗: ${String(error)}`;
		if (message !== lastMatchError) await appendMessage(message);
		lastMatchError = message;
	} finally {
		if (matchingGeneration === current) matchingGeneration = undefined;
	}
}

async function appendMessage(...newMessages: PanelMessage[]) {
    messages.value.push(...newMessages);
    await nextTick();
    if (textPanel.value) textPanel.value.scrollTop = textPanel.value.scrollHeight;
}

function appendWriteMessage(...newMessages: PanelMessage[]) {
    writeLog.value.push(...newMessages);
    writeLogScroller.append();
}

async function onSdWrite(drive: string) {
	if (writeDialogOpen.value || writing.value || closing.value || initializing.value || !drive || !sourceDirectory.value) return;
	if (!progressUnlisten) { await appendMessage('進捗通知を受信できません。アプリを再起動してください'); return; }
	writing.value = true;
	dragging.value = false;
	writeStage.value = 'prepare';
	currentRun.value = crypto.randomUUID();
	progress.value = null;
	elapsedMs.value = 0;
    operationStatus.value = null;
    elapsedAnchor = performance.now();
    elapsedTimer = setInterval(() => { elapsedMs.value = Math.max(0, performance.now() - elapsedAnchor); }, 100);
    writeLog.value = [];
    writeLogScroller.reset();
    writeTotal.value = 0;
    copyStatus.value = '';
    writeDialogOpen.value = true;
	let plan: WritePlan | null = null;
	try {
        await nextTick();
        writeDialog.value?.showModal();
        await appendMessage('SD書込の準備を開始します');
		await initializeSource(undefined, sourceDirectory.value, true);
		plan = await invoke<WritePlan>('prepare_sd_write', { directory: sourceDirectory.value, drive, runId: currentRun.value });
		if (!plan.entries.length) { await appendWriteMessage('書込対象なし（全エリアと一致するファイルがありません）'); return; }
		pendingPlan.value = plan;
		writeTotal.value = plan.entries.length;
		const initialLog: PanelMessage[] = [`SD書込先: ${plan.drive} / 対象: ${plan.entries.length}件 / ${formatByteCount(plan.total_bytes)}バイト`];
		for (const entry of plan.entries) {
			const text = sizeComment(entry);
			if (text) initialLog.push(entry.file_size > entry.size ? { text, warning: true } : text);
		}
        appendWriteMessage(...initialLog);
		writeStage.value = 'waiting';
		const done = await invoke<WriteProgress>('run_sd_write', { runId: currentRun.value });
		acceptProgress(done);
		if (latestProgress()?.stage !== 'done') throw new Error('正常完了通知を確認できませんでした');
	} catch (error) {
		writeStage.value = 'failed';
		const last = latestProgress();
		if (last) progress.value = { ...last, stage: 'failed' };
		await appendWriteMessage({ text: `SD書込を停止しました: ${String(error)}`, warning: true });
		if (plan) {
			const completed = last?.completed ?? 0;
			const started = last?.index ?? 0;
            if (last?.stage === 'final_failed' || last?.stage === 'final_verify' || last?.stage === 'final_file_done') {
                await appendWriteMessage(`最終照合成功 ${completed}/${plan.entries.length}`, `最終照合未完了: ${plan.entries.slice(completed).map(entry => entry.filename).join(', ') || 'なし'}`);
            } else {
                if (started > 0) await appendWriteMessage({ text: `${last?.filename}: 一部だけ書き込まれている可能性があります`, warning: true });
                const unexecuted = plan.entries.slice(started).map(entry => entry.filename);
                await appendWriteMessage(`最終照合成功 0/${plan.entries.length} ${(elapsedMs.value / 1000).toFixed(2)}秒`, `未実行: ${unexecuted.join(', ') || 'なし'}`);
            }
		}
	} finally {
		clearInterval(elapsedTimer);
		elapsedTimer = undefined;
		if (plan) await invoke('discard_sd_write', { runId: plan.run_id }).catch(() => {});
		writing.value = false;
        operationStatus.value = null;
		pendingPlan.value = null;
		transientNotice.value = '';
	}
}

function latestProgress(): WriteProgress | null { return progress.value; }

function closeWriteDialog() {
    if (writing.value) return;
    writeLogScroller.dispose();
    writeDialog.value?.close();
    writeDialogOpen.value = false;
}

async function copyWriteLog() {
    try {
        await navigator.clipboard.writeText(writeLog.value.map(message => typeof message === 'string' ? message : message.text).join('\n'));
        copyStatus.value = 'コピーしました';
    } catch {
        copyStatus.value = 'コピーできませんでした';
    }
}

async function openDiagnosticLog() {
    try {
        await invoke('open_diagnostic_log');
    } catch (error) {
        await appendWriteMessage({ text: `ログを開けませんでした: ${String(error)}`, warning: true });
    }
}

function showNotice(message: string) {
	transientNotice.value = message;
	clearTimeout(noticeTimer);
	noticeTimer = setTimeout(() => { transientNotice.value = ''; }, 2500);
}

function acceptProgress(event: WriteProgress) {
	if (!writing.value || event.run_id !== currentRun.value) return;
	if (progress.value?.stage === 'done' || progress.value?.stage === 'failed' || progress.value?.stage === 'final_failed') return;
    operationStatus.value = null;
	progress.value = event;
	writeStage.value = event.stage;
    const now = performance.now();
	elapsedMs.value = Math.max(elapsedMs.value, now - elapsedAnchor, event.elapsed_ms);
	elapsedAnchor = now - elapsedMs.value;
	if (!elapsedTimer && event.stage !== 'done' && event.stage !== 'failed') {
		elapsedTimer = setInterval(() => { elapsedMs.value = Math.max(0, performance.now() - elapsedAnchor); }, 100);
	}
	if (event.stage === 'file_done') void appendWriteMessage(`${event.index}/${event.total} ${event.filename} ${event.file_ms}ms ✓`);
    if (event.stage === 'final_file_done') void appendWriteMessage(`最終照合 ${event.completed}/${event.total} ${event.filename} ${event.file_ms}ms ✓`);
	if (event.stage === 'failed' || event.stage === 'final_failed') void appendWriteMessage({ text: `${event.stage === 'final_failed' ? '最終照合失敗' : '失敗'} ${event.index}/${event.total} ${event.filename} ${event.file_ms}ms: ${event.error}`, warning: true });
	if (event.stage === 'done' || event.stage === 'failed' || event.stage === 'final_failed') {
		clearInterval(elapsedTimer);
		elapsedTimer = undefined;
	}
	if (event.stage === 'done') void appendWriteMessage(`全件完了 ${event.completed}/${event.total} ${(event.elapsed_ms / 1000).toFixed(2)}秒`);
}

function acceptOperation(event: OperationStatus) {
    if (!writing.value || event.run_id !== currentRun.value) return;
    if (['done', 'failed', 'final_failed'].includes(progress.value?.stage ?? '')) return;
    operationStatus.value = event;
    operationAnchor = performance.now() - event.operation_ms;
}

async function copyDirectory() {
	try {
		await navigator.clipboard.writeText(displayedDirectory.value);
	} catch {
		await appendMessage('コピーできませんでした');
		return;
	}
	await appendMessage('コピーしました');
}

function resize(event: PointerEvent, divider: 1 | 2) {
	const target = event.currentTarget as HTMLElement;
	const startY = event.clientY;
	const initialMenuHeight = Math.max(menuHeight.value, minimumMenuHeight.value);
	const initialSdHeight = sdHeight.value;
	const textHeight = target.parentElement!.querySelector<HTMLElement>('.panel_text')!.offsetHeight;
	target.setPointerCapture(event.pointerId);

	const move = (moveEvent: PointerEvent) => {
		const delta = moveEvent.clientY - startY;
		if (divider === 1) {
			const change = Math.max(minimumMenuHeight.value - initialMenuHeight, Math.min(delta, initialSdHeight));
			menuHeight.value = initialMenuHeight + change;
			sdHeight.value = initialSdHeight - change;
		} else {
			sdHeight.value = initialSdHeight + Math.max(-initialSdHeight, Math.min(delta, textHeight));
		}
	};
	const stop = () => {
		target.removeEventListener('pointermove', move);
		target.removeEventListener('lostpointercapture', stop);
	};
	target.addEventListener('pointermove', move);
	target.addEventListener('lostpointercapture', stop);
}
</script>

<template>
	<div class="panel-outer" :class="{ 'drop-active': dragging }">
		<div class="panel-inner">
			<div class="panel-base" :style="{ gridTemplateRows: `${Math.max(menuHeight, minimumMenuHeight)}px 1px ${sdHeight}px 1px minmax(0, 1fr) minmax(28px, auto)` }">
				<div id="panel_menu" class="panel_menu">
					<div ref="menuContent" class="menu-content">
						<SdMenu :can-write="canSdWrite" :busy="initializing || writing || closing" :write-label="closing ? '終了待ち…' : writing ? `${statusLabel}…` : 'SD書込'"
							@write="onSdWrite" @error="appendMessage" />
					</div>
				</div>
				<div id="resizer1" class="resizer" @pointerdown="resize($event, 1)"></div>
				<section id="panel_sd" class="panel_sd" aria-label="表示中のフォルダー">
					<div class="path-header">
						<span role="status">SD書込ファイル数：選択{{ writeFileCount }}件 全{{ allFileCount }}件</span>
						<button type="button" :disabled="!sourceDirectory" @click="copyDirectory">フルパスをコピー</button>
					</div>
					<p v-if="pathError" class="path-message path-error" role="alert">{{ pathError }}</p>
					<ol v-else-if="folders.length" ref="pathList" class="path-list" aria-label="対象フォルダーの階層">
						<li v-for="(folder, index) in folders" :key="index"
							class="path-row" :class="{ 'path-current': index === folders.length - 1 }"
							:style="{ paddingLeft: `${Math.min(index * 9, 108)}px` }"
							:aria-current="index === folders.length - 1 ? 'location' : undefined">
							<span v-if="index > 0" class="path-branch" aria-hidden="true"></span>
							<span class="path-name" :title="folder">{{ folder }}</span>
						</li>
					</ol>
					<p v-else class="path-message" role="status">保存先を取得中…</p>
				</section>
				<div id="resizer2" class="resizer" @pointerdown="resize($event, 2)"></div>
				<div id="panel_text" class="panel_text">
					<div class="path-header text-header">
						<span role="status">SDエリア：{{ sdAddressRange ? `${sdAddressRange.startAddress} - ${sdAddressRange.endAddress}（${sdDirectDictionary.size}件）` : '未取得' }}</span>
						<button type="button" :disabled="messages.length === 0" @click="messages = []" aria-label="操作メッセージをクリア">クリア</button>
					</div>
					<div ref="textPanel" class="text-messages" role="log" aria-label="操作メッセージ">
						<p v-for="(message, index) in messages" :key="index" class="path-message"
							:class="{ 'path-warning': typeof message !== 'string' && message.warning }">{{ typeof message === 'string' ? message : message.text }}</p>
					</div>
				</div>
				<div id="panel_footer" class="panel_footer" role="status">

					<div>{{ closing ? '終了処理中：SD操作プロセスの終了を確認しています' : writing ? 'SD処理中：ドライブ・対象フォルダーの変更と再読込はできません' : dragging ? 'ドロップして対象フォルダーを変更' : initializing ? '対象フォルダーを読込中…' : 'ファイル・フォルダーをドロップできます' }}</div>
					<div v-if="transientNotice" class="path-error">{{ transientNotice }}</div>
				</div>
			</div>
		</div>
	</div>
	<dialog ref="writeDialog" class="write-dialog" aria-labelledby="write-title" @cancel.prevent="closeWriteDialog">
        <div class="write-dialog-content">
            <div class="write-header">
                <h2 id="write-title" :class="{ 'write-failed': writeStage === 'failed' }">{{ popupTitle }}</h2>
                <p class="write-count" role="status">{{ progress?.completed ?? 0 }}/{{ progress?.total ?? writeTotal }} ({{ percent }}%)</p>
            </div>
            <progress :value="percent" max="100" aria-label="SD書込・照合の全体進捗"></progress>
            <p class="write-detail">
                <span class="write-current" :title="progress?.filename ? `${statusLabel} ${progress.filename}` : statusLabel">{{ statusLabel }}<template v-if="progress?.filename"> {{ progress.filename }}</template></span>
            </p>
            <p class="write-operation" :title="operationDetail" role="status">{{ writing ? operationDetail : '' }}</p>
            <div ref="writeLogPanel" class="write-log" role="log" aria-label="SD書込ログ" tabindex="0" @scroll="writeLogScroller.onScroll">
                <p v-for="(message, index) in writeLog" :key="index" :class="{ 'log-warning': typeof message !== 'string' && message.warning, 'log-done': typeof message === 'string' && message.startsWith('全件完了') }">{{ typeof message === 'string' ? message : message.text }}</p>
            </div>
            <p v-if="transientNotice" class="write-notice" role="status">{{ transientNotice }}</p>
            <div class="write-actions">
                <span class="write-time">{{ (elapsedMs / 1000).toFixed(2) }}秒</span>
                <button type="button" class="copy-log" aria-live="polite" @click="copyWriteLog">{{ copyStatus || 'ログをコピー' }}</button>
                <button type="button" @click="openDiagnosticLog">ログ表示</button>
                <button type="button" class="close-write" :disabled="writing" @click="closeWriteDialog">閉じる</button>
            </div>
        </div>
    </dialog>
</template>

<style scoped>
.write-operation { margin: 0; flex-shrink: 0; min-height: 1.2em; line-height: 1.2; overflow: hidden; white-space: nowrap; text-overflow: ellipsis; font-size: 11px; color: #526171; }
.write-dialog { position: fixed; inset: 0; margin: auto; width: min(650px, calc(100vw - 8px)); height: min(480px, calc(100dvh - 8px)); max-width: none; max-height: calc(100dvh - 8px); padding: 0; border: 0; border-radius: 18px; background: #f8f9fe; color: #121212; box-shadow: 0 12px 32px #0003; font: 12px 'Yu Gothic UI', Meiryo, sans-serif; overflow: hidden; }
.write-dialog::backdrop { background: #0005; }
.write-dialog-content { display: flex; flex-direction: column; height: 100%; min-height: 0; gap: 6px; padding: 12px; }
.write-header { display: flex; flex-shrink: 0; align-items: center; justify-content: space-between; gap: 8px; }
.write-dialog h2 { min-width: 0; margin: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 16px; line-height: 1.3; font-weight: 700; }
.write-dialog h2.write-failed { color: #b3261e; }
.write-dialog progress { appearance: none; display: block; flex-shrink: 0; width: 100%; height: 3px; border: 0; border-radius: 8px; overflow: hidden; background: #dce5f5; accent-color: #1a73df; }
.write-dialog progress::-webkit-progress-bar { background: #dce5f5; }
.write-dialog progress::-webkit-progress-value { background: #1a73df; border-radius: 8px; transition: width .15s; }
.write-dialog progress::-moz-progress-bar { background: #1a73df; }
.write-count { flex-shrink: 0; margin: 0; color: #155bb5; font-size: 11px; white-space: nowrap; }
.write-detail { display: flex; flex-shrink: 0; margin: 0; color: #536176; font-size: 11px; line-height: 1.2; white-space: nowrap; }
.write-current { min-width: 0; overflow: hidden; text-overflow: ellipsis; }
.write-time { flex: 1; min-width: 0; color: #536176; font-size: 11px; white-space: nowrap; }
.write-log { flex: 1; min-height: 0; overflow: auto; overflow-anchor: none; scrollbar-gutter: stable; padding: 4px; border-radius: 3px; background: #1e1e1e; color: #36dbc3; font: 11px/1.25 Consolas, 'Yu Gothic UI', monospace; scrollbar-color: #888 #eee; scrollbar-width: thin; }
.write-log::-webkit-scrollbar { width: 8px; height: 8px; }
.write-log::-webkit-scrollbar-thumb { border-radius: 4px; background: #888; }
.write-log::-webkit-scrollbar-track, .write-log::-webkit-scrollbar-corner { background: #eee; }
.write-log p { width: max-content; min-width: 100%; margin: 0; white-space: nowrap; overflow-wrap: normal; word-break: normal; }
.write-log .log-warning { color: #ffb4ab; }
.write-log .log-done { margin-top: 2px; padding: 2px 4px; border-radius: 3px; background: #183e46; color: #00d8ff; font-weight: bold; }
.write-actions { display: flex; flex-shrink: 0; justify-content: flex-end; align-items: center; gap: 6px; }
.write-actions button { flex-shrink: 0; border-radius: 20px; padding: 5px 12px; font: inherit; line-height: 1.2; cursor: pointer; }
.copy-log { border: 1px solid #c5cad3; background: #e9ecf2; color: #242424; }
.close-write { border: 1px solid #1a73df; background: #1a73df; color: white; font-weight: bold !important; }
.close-write:disabled { opacity: .45; cursor: default; }
.write-actions button:focus-visible, .write-log:focus-visible { outline: 2px solid #1a73df; outline-offset: 2px; }
.write-notice { flex-shrink: 0; margin: 0; font-size: 11px; line-height: 1.2; color: #536176; }
@media (max-width: 400px), (max-height: 400px) {
    .write-dialog { border-radius: 14px; }
    .write-dialog-content { padding: 8px; gap: 5px; }
    .write-dialog h2 { font-size: 14px; }
    .write-log { font-size: 11px; line-height: 1.2; }
    .write-actions button { padding: 4px 10px; font-size: 11px; }
}
.panel-outer {
	border: 1px solid #000;
	padding: 1px;
	display: flex;
	justify-content: center;
	align-items: center;
	/* 外枠をビューポート高さに合わせ、中央配置に固定 */
	height: 100vh;
	box-sizing: border-box;
}
.panel-inner {
	border: 1px solid #666;
	width: 360px;
	/* 高さは外枠に合わせ、はみ出す場合はスクロールさせる */
	height: 100%;
	max-height: 100%;
	box-sizing: border-box;
	padding: 1px;
	overflow: auto;
}
.panel-base {
	border: 1px dashed #999;
	width: 100%;
	height: 100%;
	display: grid;
	padding: 0;
	box-sizing: border-box;
}
.panel_menu,
.panel_sd,
.panel_text,
.panel_footer {
	min-width: 0;
	min-height: 0;
	overflow: auto;
}
.panel_footer {
	border-top: 1px solid #666;
	padding: 3px 4px;
	font: 11px 'Yu Gothic UI', Meiryo, sans-serif;
}
.drop-active {
	outline: 3px solid #4285f4;
	outline-offset: -3px;
	background: #e7f1fb;
}
.panel_menu {
	/* The menu's measured content sets its minimum row height. */
	overflow: visible;
	container: sd-menu-panel / inline-size;
}
.menu-content {
	min-width: 0;
}
.panel_sd,
.panel_text {
	font-family: 'Yu Gothic UI', Meiryo, sans-serif;
	font-size: 11px;
}
.panel_sd {
	display: flex;
	flex-direction: column;
}
.path-header {
	display: flex;
	align-items: center;
	justify-content: space-between;
	flex-shrink: 0;
	gap: 4px;
	padding: 2px 4px;
	background: #f3f5f7;
	border-bottom: 1px solid #d9dfe5;
}
.path-header button {
	font: inherit;
	padding: 1px 4px;
	cursor: pointer;
}
.path-header button:disabled {
	cursor: default;
}
.path-list {
	flex: 1;
	min-height: 0;
	margin: 0;
	padding: 3px 4px;
	list-style: none;
	overflow: auto;
}
.path-row {
	display: flex;
	align-items: center;
	min-height: 15px;
	line-height: 15px;
}
.path-branch {
	align-self: flex-start;
	flex: 0 0 7px;
	height: 8px;
	margin-right: 2px;
	border-left: 1px solid #a8b3bf;
	border-bottom: 1px solid #a8b3bf;
}
.path-name {
	min-width: 0;
	overflow: hidden;
	text-overflow: ellipsis;
	white-space: nowrap;
}
.path-current {
	font-weight: 700;
	color: #174a75;
	background: #e7f1fb;
	border-radius: 2px;
}
.path-message {
	margin: 2px 4px;
	overflow-wrap: anywhere;
}
.panel_text {
	display: flex;
	flex-direction: column;
	line-height: 13px;
}
.text-header {
	justify-content: space-between;
}
.text-messages {
	flex: 1;
	min-height: 0;
	overflow: auto;
}
.panel_text .path-message {
	margin: 0 4px;
}
.path-error {
	color: #a12323;
}
.path-warning {
	color: #c00;
}
.resizer {
	background: #666;
	cursor: row-resize;
	touch-action: none;
	user-select: none;
}
</style>
