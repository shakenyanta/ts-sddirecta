<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import SdMenu from '../components/SdMenu.vue';
import { folderPathAt, splitFolderPath } from '../utils/executablePath';
import { createLogScroller } from '../utils/logScroll';
import { parseSdDirectCsv, type AddressRange, type SdArea } from '../utils/sdDirectCsv';
import { writePercent, writeStageLabel, sizeComment, formatByteCount, formatCardCapacity, type WritePlan, type WriteProgress, type OperationStatus } from '../utils/sdWrite';

const menuHeight = ref(48);
const minimumMenuHeight = ref(48);
const menuContent = ref<HTMLElement | null>(null);
let menuObserver: ResizeObserver | undefined;
let textObserver: ResizeObserver | undefined;
let textScrollMaximum = 0;
const sdHeight = ref(120);
const sourceDirectory = ref('');
const dragging = ref(false);
const initializing = ref(false);
const writing = ref(false);
const sdMenu = ref<InstanceType<typeof SdMenu> | null>(null);
interface FormatPlan { run_id: string; drive: string; file_system: string; target: { capacity: number } }
interface FormatResult { drive: string; file_system: string; capacity: number; label: string }
const formatPlan = ref<FormatPlan | null>(null);
const formatResult = ref<FormatResult | null>(null);
const formatLabel = ref('');
const formatSimpleTest = ref(false);
const formatStep = ref(0);
const formatTotal = computed(() => formatSimpleTest.value ? 6 : 5);
const formatLoggedSteps = new Set<number>();
let formatPreparationLogIndex: number | null = null;
const formatLabelValid = computed(() => /^[A-Za-z0-9 _-]{0,11}$/.test(formatLabel.value));
let formattedDrive = '';
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
const operationKind = ref<'read' | 'write' | 'ff' | 'format'>('write');
const operationName = computed(() => operationKind.value === 'format' ? 'フォーマット' : operationKind.value === 'ff' ? 'SD FF初期化' : operationKind.value === 'read' ? 'SD読込' : 'SD書込');
const popupTitle = computed(() => writing.value ? `${statusLabel.value}…`
    : operationKind.value === 'format' ? (writeStage.value === 'confirm' || writeStage.value === 'done' ? 'SDフォーマット' : 'フォーマット失敗')
    : operationKind.value === 'ff' ? (writeStage.value === 'done' ? 'FF初期化完了' : 'FF初期化失敗')
    : operationKind.value === 'read' ? (writeStage.value === 'done' ? '読込完了' : '読込失敗')
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
const statusLabel = computed(() => operationKind.value === 'format' && writeStage.value === 'confirm'
    ? 'よろしければ「開始」を押してください。'
    : operationKind.value === 'ff' && writeStage.value === 'write'
    ? 'FF初期化中' : writeStageLabel(writeStage.value));
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
type TimedPanelMessage = { text: string; warning?: true; time: string };
const messages = ref<TimedPanelMessage[]>([]);
const sdDirectDictionary = ref(new Map<string, SdArea>());
const sdloadingDictionary = ref(new Map<string, SdArea>());
const sdAddressRange = ref<AddressRange | null>(null);
const matchedFiles = ref<string[]>([]);
const warningFiles = ref<string[]>([]);
const canSdWrite = computed(() => !initializing.value && !writing.value && !closing.value && allFileCount.value > 0);
const canSdRead = computed(() => !initializing.value && !writing.value && !closing.value && sdDirectDictionary.value.size > 0);
let matchingGeneration: number | undefined;
let disposed = false;
let lastMatchError = '';
const textPanel = ref<HTMLElement | null>(null);
const pathList = ref<HTMLElement | null>(null);
const folders = computed(() => splitFolderPath(displayedDirectory.value));
const collapsedFolderIndex = ref<number | null>(null);
const visibleFolders = computed(() => collapsedFolderIndex.value === null
	? folders.value : folders.value.slice(0, collapsedFolderIndex.value + 1));

function toggleFolder(index: number) {
	collapsedFolderIndex.value = collapsedFolderIndex.value === index ? null : index;
}

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
    if (typeof ResizeObserver !== 'undefined' && textPanel.value) {
        textObserver = new ResizeObserver(() => {
            const panel = textPanel.value;
            if (!panel) return;
            // Menu wrapping/resizing changes the viewport after nextTick. Keep
            // the bottom only when it was already visible; preserve manual scroll.
            const wasAtBottom = panel.scrollTop >= textScrollMaximum - 1;
            textScrollMaximum = Math.max(0, panel.scrollHeight - panel.clientHeight);
            if (wasAtBottom) panel.scrollTop = textScrollMaximum;
        });
        textObserver.observe(textPanel.value);
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

async function initializeSource(paths?: string[], refreshDirectory?: string) {
	if (closing.value) return;
	if (writing.value) { showNotice('SD処理中のため変更できません'); return; }
	if (paths) messages.value = [];
	const current = ++generation;
	const active = () => !disposed && current === generation;
	initializing.value = true;
	collapsedFolderIndex.value = null;
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
		if (refreshDirectory) await appendMessage('F5：表示を更新');
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
    textObserver?.disconnect();
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
				text: `警告：拡張子エラー：${filename.split(/[\\/]/).pop()}`,
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
    const now = new Date();
    const time = `${String(now.getHours()).padStart(2, '0')}:${String(now.getMinutes()).padStart(2, '0')}`;
    messages.value.push(...newMessages.map(message => ({
        ...(typeof message === 'string' ? { text: message } : message), time,
    })));
    await nextTick();
    if (textPanel.value) {
        textPanel.value.scrollTop = textPanel.value.scrollHeight;
        textScrollMaximum = Math.max(0, textPanel.value.scrollHeight - textPanel.value.clientHeight);
    }
}

function appendWriteMessage(...newMessages: PanelMessage[]) {
    writeLog.value.push(...newMessages);
    writeLogScroller.append();
}

function logButtonPress(label: string) {
    void appendMessage(`「${label}」ボタン押下`);
}

async function onSdWrite(drive: string) {
	if (writeDialogOpen.value || writing.value || closing.value || initializing.value || !drive || !sourceDirectory.value) return;
    void appendMessage('「SD書込」ボタン押下');
	if (!progressUnlisten) { await appendMessage('進捗通知を受信できません。アプリを再起動してください'); return; }
    operationKind.value = 'write';
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
        appendWriteMessage('SD書込の準備を開始します');
        // prepare_sd_write reads the latest CSV/files; run_sd_write revalidates
        // them before writing. Do not replay startup logs after the click log.
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

async function onSdRead(drive: string) {
    if (writeDialogOpen.value || writing.value || closing.value || initializing.value || !drive || !sourceDirectory.value) return;
    if (!progressUnlisten) { await appendMessage('進捗通知を受信できません。アプリを再起動してください'); return; }
    operationKind.value = 'read';
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
    try {
        await nextTick();
        writeDialog.value?.showModal();
        const executableDirectory = await invoke<string>('get_executable_directory');
        appendWriteMessage(`SD読込元: ${drive}`, `保存先: ${executableDirectory.replace(/[\\/]+$/, '')}\\output`,
            'CSV・SDの検査後、output直下のファイルをすべて削除します。サブフォルダーは残します。');
        writeStage.value = 'waiting';
        const done = await invoke<WriteProgress>('run_sd_read', { directory: sourceDirectory.value, drive, runId: currentRun.value });
        acceptProgress(done);
        if (latestProgress()?.stage !== 'done') throw new Error('正常完了通知を確認できませんでした');
    } catch (error) {
        writeStage.value = 'failed';
        const last = latestProgress();
        if (last) progress.value = { ...last, stage: 'failed' };
        appendWriteMessage({ text: `SD読込を停止しました: ${String(error)}`, warning: true });
        if (last) {
            appendWriteMessage(`保存完了 ${last.completed}/${last.total}`);
            if (last.index > last.completed) appendWriteMessage({ text: `${last.filename}: 不完全なファイルが残っている可能性があります`, warning: true });
        }
    } finally {
        clearInterval(elapsedTimer);
        elapsedTimer = undefined;
        writing.value = false;
        operationStatus.value = null;
        transientNotice.value = '';
    }
}

async function onSdFfInitialize(drive: string) {
    if (writeDialogOpen.value || writing.value || closing.value || initializing.value || !drive || !sourceDirectory.value) return;
    if (!progressUnlisten) { await appendMessage('進捗通知を受信できません。アプリを再起動してください'); return; }
    operationKind.value = 'ff';
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
    try {
        await nextTick();
        writeDialog.value?.showModal();
        appendWriteMessage(`FF初期化先: ${drive}`, 'CSVの全領域を0xFFで初期化します。CSVに含まれる先頭32KiBも対象です。',
            '各領域の初期化・照合後に、全領域を最終照合します。');
        writeStage.value = 'waiting';
        const done = await invoke<WriteProgress>('run_sd_ff_initialize', { directory: sourceDirectory.value, drive, runId: currentRun.value });
        acceptProgress(done);
        if (latestProgress()?.stage !== 'done') throw new Error('正常完了通知を確認できませんでした');
    } catch (error) {
        writeStage.value = 'failed';
        const last = latestProgress();
        if (last) progress.value = { ...last, stage: 'failed' };
        appendWriteMessage({ text: `SD FF初期化を停止しました: ${String(error)}`, warning: true });
        if (last) {
            appendWriteMessage(`最終照合成功 ${last.completed}/${last.total}`);
            if (last.filename) appendWriteMessage({ text: `${last.filename}: 初期化または照合が完了していません。書換え済みの内容は元に戻しません。`, warning: true });
            const finalPass = ['final_verify', 'final_file_done', 'final_failed'].includes(last.stage);
            appendWriteMessage(finalPass ? `最終照合未完了: ${last.total - last.completed}領域`
                : `後続の未実行領域: ${Math.max(0, last.total - last.index)}件`);
        }
    } finally {
        clearInterval(elapsedTimer);
        elapsedTimer = undefined;
        writing.value = false;
        operationStatus.value = null;
        transientNotice.value = '';
    }
}

function latestProgress(): WriteProgress | null { return progress.value; }

async function onSdFormat(drive: string) {
    if (writeDialogOpen.value || writing.value || closing.value || initializing.value || !drive) return;
    if (!progressUnlisten) { await appendMessage('進捗通知を受信できません。アプリを再起動してください'); return; }
    operationKind.value = 'format';
    writing.value = true;
    dragging.value = false;
    currentRun.value = crypto.randomUUID();
    writeStage.value = 'prepare';
    progress.value = null;
    operationStatus.value = null;
    formatPlan.value = null;
    formatLabel.value = '';
    formatSimpleTest.value = false;
    formatStep.value = 0;
    formatLoggedSteps.clear();
    formatPreparationLogIndex = null;
    formatResult.value = null;
    formattedDrive = '';
    copyStatus.value = '';
    writeLog.value = [];
    writeLogScroller.reset();
    writeDialogOpen.value = true;
    try {
        await nextTick();
        writeDialog.value?.showModal();
        appendWriteMessage(`フォーマット先: ${driveDisplay(drive)} / SDカードの情報を確認しています。`);
        formatPlan.value = await invoke<FormatPlan>('prepare_sd_format', { drive, runId: currentRun.value });
        writeStage.value = 'confirm';
        appendWriteMessage(`対象: ${driveDisplay(formatPlan.value.drive)} / ${formatCardCapacity(formatPlan.value.target.capacity)} / ${formatPlan.value.file_system}`);
    } catch (error) {
        writeStage.value = 'failed';
        appendWriteMessage({ text: `フォーマットを開始できません: ${String(error)}`, warning: true });
    } finally { writing.value = false; operationStatus.value = null; }
}

function driveDisplay(drive: string) { return drive.replace(/^PhysicalDrive\d+$/, '?:'); }

function updateFormatPreparation(finished = false) {
    if (formatPreparationLogIndex === null) return;
    const ms = Math.max(0, Math.floor(performance.now() - elapsedAnchor));
    writeLog.value[formatPreparationLogIndex] = `1/${formatTotal.value} フォーマット準備中 ${ms}ms`;
    if (finished) formatPreparationLogIndex = null;
}

async function startSdFormat() {
    if (writing.value || closing.value || !formatPlan.value || writeStage.value !== 'confirm' || !formatLabelValid.value) return;
    writing.value = true;
    writeStage.value = 'format_prepare';
    formatStep.value = 1;
    elapsedMs.value = 0;
    elapsedAnchor = performance.now();
    formatPreparationLogIndex = writeLog.value.length;
    appendWriteMessage(`1/${formatTotal.value} フォーマット準備中 0ms`);
    elapsedTimer = setInterval(() => {
        elapsedMs.value = Math.max(0, performance.now() - elapsedAnchor);
        updateFormatPreparation();
    }, 100);
    const simpleTest = formatSimpleTest.value;
    try {
        const done = await invoke<FormatResult>('run_sd_format', { runId: currentRun.value, label: formatLabel.value, simpleTest });
        elapsedMs.value = Math.max(elapsedMs.value, performance.now() - elapsedAnchor);
        writeStage.value = 'done';
        progress.value = null;
        formattedDrive = done.drive;
        formatResult.value = done;
        const total = simpleTest ? 6 : 5;
        formatStep.value = total;
        appendWriteMessage(`フォーマット完了 ${total}/${total} ${(elapsedMs.value / 1000).toFixed(2)}秒`);
    } catch (error) {
        writeStage.value = 'failed';
        appendWriteMessage({ text: `フォーマットを停止しました: ${String(error)}。書換え済みの内容は元に戻しません。`, warning: true });
    } finally {
        updateFormatPreparation(true);
        clearInterval(elapsedTimer);
        elapsedTimer = undefined;
        writing.value = false;
        operationStatus.value = null;
        formatPlan.value = null;
    }
}

function closeWriteDialog() {
    if (writing.value) return;
    if (operationKind.value === 'format') {
        void invoke('discard_sd_format', { runId: currentRun.value }).catch(() => {});
        formatPlan.value = null;
        void nextTick().then(() => sdMenu.value?.refreshDrives(formattedDrive || undefined));
    }
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

async function openOutput() {
    if (writing.value) return;
    logButtonPress('outputを開く');
    try {
        await invoke('open_sd_output');
    } catch (error) {
        appendWriteMessage({ text: `outputを開けませんでした: ${String(error)}`, warning: true });
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
	if (operationKind.value === 'format') {
        progress.value = event;
        const stage = event.stage.replace(/_done$/, '');
        writeStage.value = stage;
        const step = ({ format_partition: 2, format_volume: 3, format_verify: 4, format_finalize: 5, format_test: 6 } as Record<string, number>)[stage];
        if (step && step <= formatTotal.value) {
            updateFormatPreparation(true);
            formatStep.value = Math.max(formatStep.value, Math.min(step, formatTotal.value - 1));
            if (event.stage.endsWith('_done') && !formatLoggedSteps.has(step)) {
                formatLoggedSteps.add(step);
                appendWriteMessage(`${step}/${formatTotal.value} ${event.filename} ${event.file_ms}ms`);
            }
        }
        return;
    }
	progress.value = event;
    if (operationKind.value !== 'write') writeTotal.value = event.total;
    if (operationKind.value === 'ff' && event.stage === 'prepare') {
        appendWriteMessage(`初期化対象: ${event.total}領域 / ${formatByteCount(event.total_bytes / 3)}バイト`,
            `初期化・直後照合・最終照合の総処理量: ${formatByteCount(event.total_bytes)}バイト`);
    }
	writeStage.value = event.stage;
    const now = performance.now();
	elapsedMs.value = Math.max(elapsedMs.value, now - elapsedAnchor, event.elapsed_ms);
	elapsedAnchor = now - elapsedMs.value;
	if (!elapsedTimer && event.stage !== 'done' && event.stage !== 'failed') {
		elapsedTimer = setInterval(() => { elapsedMs.value = Math.max(0, performance.now() - elapsedAnchor); }, 100);
	}
	if (event.stage === 'file_done') void appendWriteMessage(`${event.index}/${event.total} ${event.filename} ${event.file_ms}ms ✓`);
    if (event.stage === 'clean') appendWriteMessage(`読込対象: ${event.total}件 / ${formatByteCount(event.total_bytes)}バイト。output直下のファイルを削除します。`);
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

function onFolderDoubleClick(index: number, event: MouseEvent) {
	window.getSelection()?.selectAllChildren(event.currentTarget as Node);
	void openFolder(index);
}

async function openFolder(index: number) {
	const directory = folderPathAt(displayedDirectory.value, index);
	if (!directory || closing.value) return;
	try {
		await invoke('open_folder', { directory });
	} catch (error) {
		await appendMessage(`フォルダーを開けませんでした: ${directory}: ${String(error)}`);
	}
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
						<SdMenu ref="sdMenu" :can-write="canSdWrite" :can-read="canSdRead" :can-initialize="canSdRead" :busy="initializing || writing || closing || writeDialogOpen" :write-label="closing ? '終了待ち…' : writing ? `${statusLabel}…` : 'SD書込'"
							@write="onSdWrite" @read="onSdRead" @initialize="onSdFfInitialize" @format="onSdFormat" @action="logButtonPress" @error="appendMessage" />
					</div>
				</div>
				<div id="resizer1" class="resizer" @pointerdown="resize($event, 1)"></div>
				<section id="panel_sd" class="panel_sd" aria-label="表示中のフォルダー">
					<div class="path-header sd-path-header">
						<div class="path-heading"><h2>書き込み対象</h2><span class="file-count" role="status" aria-label="SD書込ファイル数">{{ allFileCount }}ファイル</span></div>
						<button type="button" class="copy-path" :disabled="!sourceDirectory" @click="copyDirectory" :title="displayedDirectory">
							<svg viewBox="0 0 16 16" aria-hidden="true"><rect x="5" y="2" width="8" height="10" rx="1.5"/><path d="M10 14H4a1 1 0 0 1-1-1V5"/></svg>
							パスをコピー
						</button>
					</div>
					<p v-if="pathError" class="path-message path-error" role="alert">{{ pathError }}</p>
					<ol v-else-if="folders.length" ref="pathList" class="path-list" aria-label="対象フォルダーの階層">
						<li v-for="(folder, index) in visibleFolders" :key="index"
							class="path-row" :class="{ 'path-current': index === folders.length - 1 }"
							:aria-current="index === folders.length - 1 ? 'location' : undefined">
							<span class="path-guides" :style="{ width: `${index * 14}px` }" aria-hidden="true"></span>
							<button v-if="index < folders.length - 1" type="button" class="folder-toggle"
								:aria-expanded="collapsedFolderIndex !== index" :aria-label="`${folder}の子階層を${collapsedFolderIndex === index ? '展開' : '折りたたむ'}`"
								@click="toggleFolder(index)">
								<svg viewBox="0 0 12 12" aria-hidden="true" :class="{ 'folder-expanded': collapsedFolderIndex !== index }"><path d="m4 2 4 4-4 4"/></svg>
							</button>
							<span v-else class="folder-toggle-spacer" aria-hidden="true"></span>
							<svg class="folder-icon" viewBox="0 0 16 16" aria-hidden="true"><path d="M1.5 4a1 1 0 0 1 1-1H6l1.5 2h6a1 1 0 0 1 1 1v6a1 1 0 0 1-1 1h-11a1 1 0 0 1-1-1Z"/></svg>
							<span class="path-name" :title="`${folderPathAt(displayedDirectory, index)}\nダブルクリックでエクスプローラーで開く`"
								@dblclick.prevent="onFolderDoubleClick(index, $event)">{{ folder }}</span>
						</li>
					</ol>
					<p v-else class="path-message" role="status">保存先を取得中…</p>
				</section>
				<div id="resizer2" class="resizer" @pointerdown="resize($event, 2)"></div>
				<div id="panel_text" class="panel_text">
					<div class="path-header text-header">
						<span role="status">SD MAP：{{ sdAddressRange ? `${sdAddressRange.startAddress} - ${sdAddressRange.endAddress}（${sdDirectDictionary.size}件）` : '未取得' }}</span>
						<button type="button" class="clear-messages" :disabled="messages.length === 0" @click="messages = []" aria-label="操作メッセージをクリア">
							<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M2.5 4.5h11M6 4.5V2.8a.8.8 0 0 1 .8-.8h2.4a.8.8 0 0 1 .8.8v1.7M4 4.5l.7 8.5a1 1 0 0 0 1 1h4.6a1 1 0 0 0 1-1l.7-8.5M6.5 7v4.5M9.5 7v4.5"/></svg>
							クリア
						</button>
					</div>
					<div ref="textPanel" class="text-messages" role="log" aria-label="操作メッセージ">
						<p v-for="(message, index) in messages" :key="index" class="path-message"
							:class="{ 'path-warning': message.warning }"><time class="message-time">{{ message.time }}</time><span>{{ message.text }}</span></p>
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
                <p v-if="operationKind !== 'format'" class="write-count" role="status">{{ progress?.completed ?? 0 }}/{{ progress?.total ?? writeTotal }} ({{ percent }}%)</p>
                <p v-else-if="formatStep > 0" class="write-count" role="status">{{ formatStep }}/{{ formatTotal }}{{ writeStage === 'done' ? ' (100%)' : '' }}</p>
            </div>
            <progress :value="operationKind === 'format' ? formatStep / formatTotal * 100 : percent" max="100" :aria-label="`${operationName}の全体進捗`"></progress>
            <p v-if="operationKind === 'format' && writeStage === 'done' && formatResult" class="format-result">
                {{ formatResult.drive }} / <strong>{{ formatResult.file_system }}</strong> / {{ formatCardCapacity(formatResult.capacity) }} / ラベル: {{ formatResult.label || 'なし' }}
            </p>
            <div v-if="operationKind === 'format' && writeStage === 'confirm'" class="format-settings">
                <p class="format-warning">このSDカードのデータはすべて失われます。</p>
                <p :title="formatPlan ? `実容量: ${formatByteCount(formatPlan.target.capacity)}バイト` : ''">対象: {{ driveDisplay(formatPlan?.drive || '') }} / {{ formatCardCapacity(formatPlan?.target.capacity || 0) }} / {{ formatPlan?.file_system }}（容量から自動選択）</p>
                <label class="format-volume-label">
                    <span>ボリュームラベル<small id="format-label-help">（11文字以内）</small></span>
                    <input v-model="formatLabel" maxlength="11" autocomplete="off" aria-describedby="format-label-help">
                </label>
                <label class="format-simple-test"><input v-model="formatSimpleTest" type="checkbox">FAT簡易テスト(書込・読出・削除)</label>
                <p v-if="!formatLabelValid" role="alert">ラベルに使用できない文字があります。</p>
            </div>
            <p class="write-detail" :class="{ 'format-guidance': operationKind === 'format' && writeStage === 'confirm' }">
                <span class="write-current" :title="progress?.filename ? `${statusLabel} ${progress.filename}` : statusLabel">{{ statusLabel }}<template v-if="progress?.filename"> {{ progress.filename }}</template></span>
            </p>
            <p class="write-operation" :title="operationDetail" role="status">{{ writing ? operationDetail : '' }}</p>
            <div ref="writeLogPanel" class="write-log" :class="{ 'format-log': operationKind === 'format' }" role="log" :aria-label="`${operationName}ログ`" tabindex="0" @scroll="writeLogScroller.onScroll">
                <p v-for="(message, index) in writeLog" :key="index" :class="{ 'log-warning': typeof message !== 'string' && message.warning, 'log-done': typeof message === 'string' && (message.startsWith('全件完了') || message.startsWith('フォーマット完了 ')) }">{{ typeof message === 'string' ? message : message.text }}</p>
            </div>
            <p v-if="transientNotice" class="write-notice" role="status">{{ transientNotice }}</p>
            <div class="write-actions">
                <button v-if="operationKind === 'format' && writeStage === 'confirm'" type="button" class="close-write" :disabled="writing || !formatLabelValid" @click="startSdFormat">開始</button>
                <button v-if="operationKind === 'read'" type="button" class="copy-log open-output" :disabled="writing" @click="openOutput">outputを開く</button>
                <button type="button" class="copy-log" aria-live="polite" @click="copyWriteLog">{{ copyStatus || 'ログをコピー' }}</button>
                <button type="button" class="close-write" :disabled="writing" @click="closeWriteDialog">{{ operationKind === 'format' && writeStage === 'confirm' ? 'キャンセル' : '閉じる' }}</button>
            </div>
        </div>
    </dialog>
</template>

<style scoped>
.format-settings { flex-shrink: 0; padding: 8px; border: 1px solid #c5cad3; border-radius: 6px; }
.format-settings p { margin: 3px 0; }
.format-settings .format-warning { color: #a3271d; font-weight: bold; }
.format-log .log-warning { width: auto; min-width: 0; white-space: pre-wrap; overflow-wrap: anywhere; }
.format-volume-label { display: flex; flex-wrap: wrap; align-items: center; gap: 4px 6px; }
.format-volume-label > span { flex-shrink: 0; }
.format-volume-label small { color: #536176; font-size: 10px; }
.format-volume-label input { flex: 1 1 80px; min-width: 80px; max-width: 140px; width: 100%; font: inherit; }
.format-simple-test { display: flex; align-items: flex-start; gap: 5px; margin-top: 6px; }
.format-simple-test input { flex-shrink: 0; width: 13px; height: 13px; margin-top: 2px; accent-color: #1a73df; }
.format-result { flex-shrink: 0; margin: 0; color: #536176; }
.format-result strong { color: #155bb5; }
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
.format-guidance, .format-guidance .write-current { white-space: normal; overflow: visible; }
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
.open-output { margin-right: auto; }
.open-output:disabled { opacity: .45; cursor: default; }
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
	overflow: hidden;
	background: #fff;
	border: 1px solid #e2e7ee;
	border-radius: 7px;
}
.panel_sd .sd-path-header { padding: 5px 7px; background: #fff; border-bottom: 1px solid #edf0f5; }
.path-heading { display: flex; align-items: center; gap: 7px; min-width: 0; }
.path-heading h2 { margin: 0; font-size: 12px; font-weight: 700; color: #273449; white-space: nowrap; }
.file-count { padding: 1px 6px; border-radius: 9px; background: #eef1f5; color: #64748b; font-size: 10px; white-space: nowrap; }
.sd-path-header .copy-path { display: inline-flex; align-items: center; gap: 3px; flex-shrink: 0; padding: 2px; border: 0; border-radius: 3px; background: transparent; color: #3267aa; white-space: nowrap; }
.copy-path svg { width: 14px; height: 14px; fill: none; stroke: currentColor; stroke-width: 1.2; }
.copy-path:hover:enabled { background: #edf4fe; }
.sd-path-header .copy-path:disabled { color: #94a3b8; }
.copy-path:focus-visible, .folder-toggle:focus-visible { outline: 2px solid #4285d4; outline-offset: -1px; }
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
	padding: 4px;
	list-style: none;
	overflow: auto;
}
.path-row {
	display: flex;
	align-items: center;
	min-height: 18px;
	line-height: 18px;
	border-radius: 3px;
}
.path-guides { align-self: stretch; flex-shrink: 0; background: repeating-linear-gradient(to right, transparent 0 6px, #edf0f5 6px 7px, transparent 7px 14px); }
.folder-toggle, .folder-toggle-spacer { flex: 0 0 14px; width: 14px; height: 18px; }
.folder-toggle { display: inline-flex; align-items: center; justify-content: center; padding: 0; border: 0; border-radius: 2px; background: transparent; color: #7a899c; cursor: pointer; }
.folder-toggle:hover { background: #dce9fa; }
.folder-toggle svg { width: 9px; height: 9px; fill: none; stroke: currentColor; stroke-width: 1.5; }
.folder-toggle .folder-expanded { transform: rotate(90deg); }
.folder-icon { flex: 0 0 13px; width: 13px; height: 13px; margin-right: 4px; fill: #f4c45e; stroke: #e4ac3f; stroke-width: .6; }
.path-name {
	min-width: 0;
	overflow: hidden;
	text-overflow: ellipsis;
	white-space: nowrap;
}
.path-current {
	font-weight: 700;
	color: #174a75;
	background: #dfedff;
	border-radius: 2px;
}
.path-message {
	margin: 2px 4px;
	overflow-wrap: anywhere;
}
.panel_text {
	display: grid;
	grid-template-rows: auto minmax(0, 1fr);
	overflow: hidden;
	line-height: 13px;
}
.text-header {
	justify-content: space-between;
}
.text-header .clear-messages {
	display: inline-flex;
	align-items: center;
	gap: 3px;
	flex-shrink: 0;
	padding: 2px;
	border: 0;
	border-radius: 3px;
	background: transparent;
	color: #3267aa;
	white-space: nowrap;
}
.clear-messages svg {
	width: 14px;
	height: 14px;
	fill: none;
	stroke: currentColor;
	stroke-width: 1.2;
	stroke-linecap: round;
	stroke-linejoin: round;
}
.clear-messages:hover:enabled { background: #edf4fe; }
.clear-messages:active:enabled { background: #dce9fa; }
.clear-messages:focus-visible { outline: 2px solid #4285d4; outline-offset: -1px; }
.text-header .clear-messages:disabled { color: #94a3b8; }
.text-messages {
	min-height: 0;
	overflow: auto;
	overflow-anchor: none;
	overscroll-behavior: contain;
	scrollbar-gutter: stable;
	padding-bottom: 2px;
}
.panel_text .path-message {
	display: flex;
	align-items: baseline;
	gap: 6px;
	margin: 0 4px;
}
.message-time {
	flex-shrink: 0;
	font-variant-numeric: tabular-nums;
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
