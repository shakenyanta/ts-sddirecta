<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import SdMenu from '../components/SdMenu.vue';
import OptionGrid from '../components/OptionGrid.vue';
import FolderCombo from '../components/FolderCombo.vue';
import { features, initializeEdition } from '../editions';
import { folderPathAt, splitFolderPath } from '../utils/executablePath';
import { folderTreeStorageKey, parseFolderTreeState, type FolderTreeState } from '../utils/folderTreeState';
import { dropHistoryStorageKey, parseDropHistoryState, type DropHistoryEntry, type DropHistoryState } from '../utils/dropHistoryState';
import { defaultHistoryPathRules, fixedHistoryPathRules, editableHistoryPathRules, historyPathRulesStorageKey, parseHistoryPathRules, validateHistoryPathRules, historyDisplayPath, historyPathColors, historyPathColorStyle, historyPathsColorStyle, type HistoryPathRule } from '../utils/historyPath';
import { createLogScroller } from '../utils/logScroll';
import { formatFileTimestamp } from '../utils/fileTimestamp';
import { FileIconCache, type IconFile } from '../utils/fileIconCache';
import { parseSdDirectCsv, type AddressRange, type SdArea } from '../utils/sdDirectCsv';
import { writePercent, writeStageLabel, sizeComment, formatByteCount, formatCardCapacity, explainSdFailure, type WritePlan, type WriteProgress, type OperationStatus } from '../utils/sdWrite';

const menuHeight = ref(48);
const minimumMenuHeight = ref(48);
const menuContent = ref<HTMLElement | null>(null);
let menuObserver: ResizeObserver | undefined;
let textObserver: ResizeObserver | undefined;
let textScrollMaximum = 0;
const sdHeightRatio = ref(4 / 7);
const baseWidthRatio = ref(1 / 3);
const optionTabs = [
    { id: 'files', label: 'TSファイル情報' },
    { id: 'file-info', label: 'ファイル情報' },
    { id: 'history', label: '履歴' },
    { id: 'output', label: 'output' },
] as const;
type OptionTabId = typeof optionTabs[number]['id'];
const activeOptionTab = ref<OptionTabId>('files');
const optionTabStorageKey = 'sddirecta-option-tab-v1';

function setOptionTab(id: OptionTabId) {
    activeOptionTab.value = id;
    if (!features.value.optionPanel) return;
    try {
        localStorage.setItem(optionTabStorageKey, id);
    } catch (error) {
        void appendMessage({ text: `???????????????: ${String(error)}`, warning: true });
    }
}

function restoreOptionTab() {
    try {
        const saved = localStorage.getItem(optionTabStorageKey);
        const tab = optionTabs.find(tab => tab.id === saved);
        if (tab) activeOptionTab.value = tab.id;
    } catch (error) {
        void appendMessage({ text: `???????????????: ${String(error)}`, warning: true });
    }
}

const dropHistory = ref<DropHistoryEntry[]>([]);
const customHistoryPathRules = ref<HistoryPathRule[]>(defaultHistoryPathRules.map(rule => ({ ...rule })));
const historyPathRules = computed(() => [...fixedHistoryPathRules, ...customHistoryPathRules.value]);
const historyPathDraft = ref<HistoryPathRule[]>([]);
const historyPathEditorRows = computed(() => [...fixedHistoryPathRules.map(rule => ({ ...rule })), ...historyPathDraft.value]);
const historyPathDialog = ref<HTMLDialogElement | null>(null);
const historyPathError = ref('');
const historySettingsMenu = ref<HTMLDetailsElement | null>(null);
const historyHelpDialog = ref<HTMLDialogElement | null>(null);

function closeHistorySettingsMenu() {
	if (historySettingsMenu.value) historySettingsMenu.value.open = false;
}

function displayHistoryPath(path: string) {
	return historyDisplayPath(path, historyPathRules.value);
}

async function selectRegisteredFolder(folder: string) {
	if (disposed || historyBusy.value) return;
	const rule = historyPathRules.value.find(rule => rule.folder === folder);
	setOptionTab(rule && rule.filter !== 'ts' ? 'file-info' : 'files');
	await initializeSource([folder], undefined, false, 'registered');
}

function editHistoryPathRules() {
	if (disposed || closing.value || !historyPathDialog.value || historyPathDialog.value.open) return;
	historyPathDraft.value = customHistoryPathRules.value.map(rule => ({ ...rule }));
	historyPathError.value = '';
	historyPathDialog.value.showModal();
}

function cancelHistoryPathRules() {
	historyPathError.value = '';
	historyPathDialog.value?.close();
}

function saveHistoryPathRules() {
	if (disposed || closing.value || !historyPathDialog.value?.open) return;
	try {
		const draft = validateHistoryPathRules(historyPathDraft.value.filter(rule => rule.label.trim() || rule.folder.trim()), fixedHistoryPathRules.length);
		const rules = validateHistoryPathRules([...fixedHistoryPathRules, ...editableHistoryPathRules(draft)]).slice(fixedHistoryPathRules.length);
		localStorage.setItem(historyPathRulesStorageKey, JSON.stringify({ version: 1, rules }));
		customHistoryPathRules.value = rules;
		cancelHistoryPathRules();
	} catch (error) {
		historyPathError.value = `省略設定を保存できませんでした: ${String(error)}`;
	}
}

function restoreHistoryPathRules() {
	try {
		const raw = localStorage.getItem(historyPathRulesStorageKey);
		if (raw !== null) customHistoryPathRules.value = editableHistoryPathRules(parseHistoryPathRules(raw));
	} catch (error) {
		historyPathError.value = `省略設定を復元できないため初期設定を使用します: ${String(error)}`;
		void appendMessage({ text: historyPathError.value, warning: true });
	}
}
const historyRows = computed(() => [...dropHistory.value].reverse()
	.sort((left, right) => right.droppedAt - left.droppedAt));
const historyGroups = computed(() => [
	{ id: 'favorites', label: 'お気に入り', entries: historyRows.value.filter(entry => entry.favorite) },
	{ id: 'recent', label: '履歴', entries: historyRows.value.filter(entry => !entry.favorite) },
]);
const selectedHistoryId = ref<string | null>(null);
const historyBusy = computed(() => initializing.value || writing.value || closing.value);
const historyStorageError = ref('');
let historyStorageBlocked = false;
const historyClearDialog = ref<HTMLDialogElement | null>(null);
let historyClearIds = new Set<string>();

function requestHistoryClear() {
	const dialog = historyClearDialog.value;
	if (disposed || closing.value || !dialog || dialog.open) return;
	const ids = historyGroups.value[1].entries.map(entry => entry.id);
	if (!ids.length) return;
	historyClearIds = new Set(ids);
	dialog.showModal();
}

function cancelHistoryClear() {
	historyClearIds.clear();
	historyClearDialog.value?.close();
}

function confirmHistoryClear() {
	if (!historyClearDialog.value?.open) return;
	if (disposed || closing.value) { cancelHistoryClear(); return; }
	// Protect new drops and entries made favorite while confirmation is open.
	const remaining = dropHistory.value.filter(entry => entry.favorite || !historyClearIds.has(entry.id));
	cancelHistoryClear();
	if (remaining.length === dropHistory.value.length) return;
	dropHistory.value = remaining;
	if (!remaining.some(entry => entry.id === selectedHistoryId.value)) selectedHistoryId.value = null;
	saveDropHistory();
}

function restoreDropHistory() {
	try {
		const raw = localStorage.getItem(dropHistoryStorageKey);
		if (raw !== null) dropHistory.value = parseDropHistoryState(raw).entries;
	} catch (error) {
		// Preserve unreadable data for recovery; new entries stay in memory this session.
		historyStorageBlocked = true;
		historyStorageError.value = `履歴を復元できませんでした。元データを保護するため、今回の履歴は保存しません: ${String(error)}`;
		void appendMessage({ text: historyStorageError.value, warning: true });
	}
}

function saveDropHistory() {
	if (historyStorageBlocked) return;
	try {
		localStorage.setItem(dropHistoryStorageKey, JSON.stringify({ version: 1, entries: dropHistory.value } satisfies DropHistoryState));
		historyStorageError.value = '';
	} catch (error) {
		const message = `履歴を保存できませんでした。今回の変更は再起動後に残らない可能性があります: ${String(error)}`;
		if (historyStorageError.value !== message) void appendMessage({ text: message, warning: true });
		historyStorageError.value = message;
	}
}

function toggleHistoryFavorite(id: string) {
	if (disposed || closing.value) return;
	const entry = dropHistory.value.find(entry => entry.id === id);
	if (!entry) return;
	entry.favorite = !entry.favorite;
	saveDropHistory();
}

function deleteHistory(id: string) {
	if (disposed || closing.value) return;
	const index = dropHistory.value.findIndex(entry => entry.id === id);
	if (index < 0 || dropHistory.value[index].favorite) return;
	dropHistory.value.splice(index, 1);
	if (selectedHistoryId.value === id) selectedHistoryId.value = null;
	saveDropHistory();
}

function selectHistory(id: string) {
	if (disposed || historyBusy.value) return;
	selectedHistoryId.value = id;
}

async function reopenHistory(id: string) {
	if (disposed || historyBusy.value) return;
	const entry = dropHistory.value.find(entry => entry.id === id);
	if (!entry) return;
	selectedHistoryId.value = id;
	setOptionTab('files');
	await initializeSource([...entry.paths], undefined, false, 'history');
}

async function copyHistoryPath(id: string, index: number) {
	if (disposed) return;
	const path = dropHistory.value.find(entry => entry.id === id)?.paths[index];
	if (path === undefined) return;
	try {
		await navigator.clipboard.writeText(path);
		if (!disposed) showNotice('フルパスをコピーしました');
	} catch (error) {
		if (disposed) return;
		showNotice('フルパスをコピーできませんでした');
		await appendMessage({ text: `フルパスをコピーできませんでした: ${String(error)}`, warning: true });
	}
}

function navigateOptionTab(event: KeyboardEvent, index: number) {
    let nextIndex: number;
    switch (event.key) {
        case 'ArrowLeft': nextIndex = (index + optionTabs.length - 1) % optionTabs.length; break;
        case 'ArrowRight': nextIndex = (index + 1) % optionTabs.length; break;
        case 'Home': nextIndex = 0; break;
        case 'End': nextIndex = optionTabs.length - 1; break;
        default: return;
    }
    event.preventDefault();
    void selectOptionTab(optionTabs[nextIndex].id);
    document.getElementById(`option-tab-${activeOptionTab.value}`)?.focus();
}

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
const operationError = ref('');
const failureDetails = computed(() => writeStage.value === 'failed' && operationError.value
    ? explainSdFailure(operationError.value, operationKind.value === 'ff' && progress.value === null) : null);
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
const writeFiles = computed(() => [...sdloadingDictionary.value]
    .filter(([, area]) => area.accessFlag !== 'E')
    .map(([filename, area]) => ({ filename, description: area.description, startAddress: area.startAddress, endAddress: area.endAddress, individualInfo: fileIndividualInfos.value.get(filename)?.value ?? '', modifiedAt: fileMetadata.value.get(filename)?.modified_at ?? null, size: fileMetadata.value.get(filename)?.size ?? null, csvSize: Number.parseInt(area.endAddress, 16) - Number.parseInt(area.startAddress, 16) + 1 })));
const allFileCount = computed(() => writeFiles.value.length);
interface UpdateInfo { update_no: string; update_ver: string; condition: string; car_type: string }
interface FileUpdates { files: Record<string, UpdateInfo[]>; warnings: string[] }
const fileUpdates = ref<FileUpdates>({ files: {}, warnings: [] });
function expandOptionFiles(files: typeof writeFiles.value, infos: Map<string, FileIndividualInfo>, updatesByFile: FileUpdates) {
    return files.flatMap(file => {
        const updates = updatesByFile.files[file.filename];
        if (!updates?.length) return [file];
        return updates.map(update => ({ ...file,
            individualInfo: update.update_no === '02'
                ? infos.get(file.filename)?.bm_parameter2 ?? '' : file.individualInfo,
            description: update.update_no === '01' ? 'パラメータ1'
                : update.update_no === '02' ? 'パラメータ2' : file.description,
            updateNo: update.update_no, updateVer: update.update_ver,
            condition: update.condition, carType: update.car_type,
        }));
    });
}
const tsFileIcons = ref(new Map<string, string>());
const fileIconCache = new FileIconCache();
let tsFileIconsRequest = 0;
const optionFiles = computed(() => expandOptionFiles(writeFiles.value, fileIndividualInfos.value, fileUpdates.value)
    .map(file => ({ ...file, icon: tsFileIcons.value.get(file.filename) })));
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
const requiredSdCapacity = computed(() => sdAddressRange.value ? Number(BigInt(sdAddressRange.value.endAddress) + 1n) : 0);
const mapDetailsOpen = ref(false);
const matchedFiles = ref<string[]>([]);
interface FileMetadata { modified_at: number | null; size: number | null }
const fileMetadata = ref(new Map<string, FileMetadata>());
interface FileIndividualInfo { value: string; bm_parameter2: string }
const fileIndividualInfos = ref(new Map<string, FileIndividualInfo>());
const warningFiles = ref<string[]>([]);

interface MatchedFileResult {
    files: string[];
    file_metadata: Record<string, FileMetadata>;
    individual_infos: Record<string, FileIndividualInfo>;
    file_updates: FileUpdates;
    warning_files: string[];
    from_input: boolean;
    input_exists: boolean;
}
const outputResult = ref<MatchedFileResult | null>(null);
const outputDirectory = ref('');
const outputLoading = ref(false);
const outputError = ref('');
let outputRequest = 0;
interface DirectoryFile { filename: string; metadata: FileMetadata }
const directoryFiles = ref<DirectoryFile[]>([]);
const directoryFileIcons = ref(new Map<string, string>());
const directoryFilesLoading = ref(false);
const directoryFilesError = ref('');
let directoryFilesRequest = 0;
const generalFiles = computed(() => directoryFiles.value.map(file => ({
    filename: file.filename, description: '', individualInfo: '',
    size: file.metadata.size, modifiedAt: file.metadata.modified_at,
    icon: directoryFileIcons.value.get(file.filename),
})));
const outputFiles = computed(() => {
    const result = outputResult.value;
    if (!result) return [];
    const files = result.files.flatMap(filename => {
        const area = sdDirectDictionary.value.get(filename);
        if (!area || area.accessFlag === 'E') return [];
        return [{ filename, description: area.description,
            startAddress: area.startAddress, endAddress: area.endAddress,
            individualInfo: result.individual_infos[filename]?.value ?? '',
            modifiedAt: result.file_metadata[filename]?.modified_at ?? null,
            size: result.file_metadata[filename]?.size ?? null,
            csvSize: Number.parseInt(area.endAddress, 16) - Number.parseInt(area.startAddress, 16) + 1 }];
    });
    return expandOptionFiles(files, new Map(Object.entries(result.individual_infos)), result.file_updates);
});

async function selectOptionTab(id: typeof optionTabs[number]['id']) {
    setOptionTab(id);
    if (id === 'output') await refreshOutputFiles();
    if (id === 'file-info') await refreshDirectoryFiles();
}

async function refreshDirectoryFiles() {
    if (!features.value.optionPanel || disposed || initializing.value || writing.value || closing.value || !sourceDirectory.value) return;
    const request = ++directoryFilesRequest;
    const current = generation;
    const directory = sourceDirectory.value;
    const active = () => !disposed && current === generation && request === directoryFilesRequest;
    directoryFiles.value = [];
    directoryFileIcons.value = new Map();
    directoryFilesError.value = '';
    directoryFilesLoading.value = true;
    try {
        const files = await invoke<DirectoryFile[]>('list_directory_files', { directory });
        if (!active()) return;
        directoryFiles.value = files;
        void loadFileIcons(directory, files.map(file => ({ filename: file.filename, size: file.metadata.size, modifiedAt: file.metadata.modified_at })), active,
            icons => { directoryFileIcons.value = new Map([...directoryFileIcons.value, ...Object.entries(icons)]); });
    } catch (error) {
        if (!active()) return;
        directoryFilesError.value = `ファイル一覧を取得できませんでした: ${directory}: ${String(error)}`;
        await appendMessage({ text: directoryFilesError.value, warning: true });
    } finally {
        if (active()) directoryFilesLoading.value = false;
    }
}

async function loadFileIcons(directory: string, files: IconFile[], active: () => boolean,
    apply: (icons: Record<string, string>) => void) {
    if (!active()) return;
    const cached: Record<string, string> = Object.create(null);
    const pending: IconFile[] = [];
    for (const file of files) {
        const icon = fileIconCache.get(directory, file);
        if (icon) cached[file.filename] = icon;
        else pending.push(file);
    }
    apply(cached);
    try {
        for (let offset = 0; offset < pending.length; offset += 8) {
            if (!active() || writing.value || closing.value) return;
            const batch = pending.slice(offset, offset + 8);
            const icons = await invoke<Record<string, string>>('list_file_icons', {
                directory, filenames: batch.map(file => file.filename),
            });
            if (!active()) return;
            for (const file of batch) {
                if (Object.prototype.hasOwnProperty.call(icons, file.filename) && icons[file.filename]) fileIconCache.set(directory, file, icons[file.filename]);
            }
            apply(icons);
        }
    } catch (error) {
        if (active()) await appendMessage({ text: `ファイルアイコンを取得できませんでした: ${directory}: ${String(error)}`, warning: true });
    }
}

async function refreshOutputFiles() {
    if (!features.value.optionPanel || disposed || initializing.value || writing.value || closing.value) return;
    const request = ++outputRequest;
    const current = generation;
    const active = () => !disposed && request === outputRequest && current === generation;
    outputResult.value = null;
    outputError.value = '';
    outputLoading.value = true;
    try {
        const directory = await invoke<string>('get_executable_directory');
        if (!active()) return;
        outputDirectory.value = `${directory.replace(/[\\/]+$/, '')}\\output`;
        const result = await invoke<MatchedFileResult>('find_output_files', { filenames: [...sdDirectDictionary.value.keys()] });
        if (!active()) return;
        outputResult.value = result;
        // outputでは複数のFILE-BCを保持できるため、複数存在の警告だけを表示しない。
        const updateWarnings = result.file_updates.warnings.filter(text => !text.startsWith('FILE-BCが複数あります:'));
        const warnings = [...result.warning_files.map(name => `拡張子エラー：${name}`), ...updateWarnings];
        if (warnings.length) await appendMessage(...warnings.map(text => ({ text: `output：${text}`, warning: true as const })));
    } catch (error) {
        if (!active()) return;
        outputError.value = `output読込失敗: ${String(error)}`;
        await appendMessage({ text: outputError.value, warning: true });
    } finally {
        if (request === outputRequest) outputLoading.value = false;
    }
}

const canSdWrite = computed(() => !initializing.value && !writing.value && !closing.value && allFileCount.value > 0);
const canSdRead = computed(() => !initializing.value && !writing.value && !closing.value && sdDirectDictionary.value.size > 0);
let matchingGeneration: number | undefined;
let disposed = false;
let lastMatchError = '';
const textPanel = ref<HTMLElement | null>(null);
const pathList = ref<HTMLElement | null>(null);
interface FolderNode {
	path: string;
	name: string;
	children: string[];
	expanded: boolean;
	loaded: boolean;
	// Unlike loaded (this session), this survives restarts and listing failures.
	childrenListed: boolean;
	loading: boolean;
}
const folderNodes = ref(new Map<string, FolderNode>());
// Keep the last successful result independently of tree rebuilds.
const folderMatches = ref(new Map<string, boolean>());
function folderMatchKey(path: string) {
	return folderPathAt(path, splitFolderPath(path).length - 1).toLowerCase();
}
const treeRoot = ref('');
const selectedFolder = ref('');
let restoringFolders = false;
let folderSaveTimer: ReturnType<typeof setTimeout> | undefined;
let folderStorageWarning = false;

function saveFolderState() {
	clearTimeout(folderSaveTimer);
	if (restoringFolders || initializing.value || !treeRoot.value || !selectedFolder.value || typeof localStorage === 'undefined') return;
	const nodes: FolderTreeState['nodes'] = [];
	const pending = [treeRoot.value];
	while (pending.length) {
		const node = folderNodes.value.get(pending.pop()!);
		if (!node) continue;
		nodes.push({ path: node.path, children: [...node.children], expanded: node.expanded, childrenListed: node.childrenListed });
		pending.push(...node.children);
	}
	try {
		localStorage.setItem(folderTreeStorageKey, JSON.stringify({ version: 1, root: treeRoot.value,
			selected: selectedFolder.value, scrollTop: pathList.value?.scrollTop ?? 0,
			nodes, matches: [...folderMatches.value] } satisfies FolderTreeState));
	} catch (error) {
		if (!folderStorageWarning) {
			folderStorageWarning = true;
			void appendMessage({ text: `フォルダー表示状態を保存できませんでした: ${String(error)}`, warning: true });
		}
	}
}

function scheduleFolderSave() {
	if (typeof localStorage === 'undefined' || restoringFolders || disposed) return;
	clearTimeout(folderSaveTimer);
	folderSaveTimer = setTimeout(saveFolderState, 250);
}

async function restoreFolderState() {
	if (typeof localStorage === 'undefined') return false;
	let state: FolderTreeState;
	try {
		const raw = localStorage.getItem(folderTreeStorageKey);
		if (!raw) return false;
		state = parseFolderTreeState(raw);
	} catch (error) {
		await appendMessage({ text: `フォルダー表示状態を復元できませんでした。通常表示で起動します: ${String(error)}`, warning: true });
		return false;
	}
	const current = generation;
	const active = () => !disposed && current === generation;
	restoringFolders = true;
	initializing.value = true;
	try {
		let directory: string | null = state.selected;
		try {
			directory = await invoke<string | null>('resolve_saved_folder', { directory: state.selected });
		} catch (error) {
			if (!active()) return true;
			await appendMessage({ text: `保存されたフォルダーを確認できませんでした。前回の階層を保持します: ${state.selected}: ${String(error)}`, warning: true });
		}
		if (!active()) return true;
		folderMatches.value = new Map(state.matches);
		if (!directory) return false;
		folderNodes.value = new Map(state.nodes.map(node => [node.path, { ...node,
			name: splitFolderPath(node.path).slice(-1)[0], loaded: false, loading: false }]));
		treeRoot.value = state.root;
		selectedFolder.value = directory;
		if (!folderNodes.value.has(directory)) resetFolderTree(directory);
		if (directory !== state.selected) {
			const missing = folderPathAt(state.selected, splitFolderPath(directory).length);
			const parent = folderNodes.value.get(directory);
			if (parent) parent.children = parent.children.filter(path => folderMatchKey(path) !== folderMatchKey(missing));
			removeCachedFolderBranch(missing);
			await appendMessage(`保存された選択先がないため親フォルダーへ戻りました: ${directory}`);
		}
		// Path-only ancestors keep their cached child without enumerating siblings.
		const pending = [treeRoot.value];
		while (pending.length && active()) {
			const node = folderNodes.value.get(pending.pop()!);
			if (!node?.expanded) continue;
			if (node.childrenListed) await loadChildFolders(node.path);
			if (!active()) return true;
			pending.push(...node.children.slice().reverse());
		}
		if (!active()) return true;
		await initializeSource(undefined, directory, true);
		await nextTick();
		if (!disposed && generation === current + 1 && pathList.value) pathList.value.scrollTop = state.scrollTop;
		return true;
	} finally {
		restoringFolders = false;
		if (active()) initializing.value = false;
		scheduleFolderSave();
	}
}
const visibleFolders = computed(() => {
	const rows: { node: FolderNode; depth: number; hasMatches: boolean | undefined }[] = [];
	const pending = treeRoot.value ? [{ path: treeRoot.value, depth: 0 }] : [];
	while (pending.length) {
		const { path, depth } = pending.pop()!;
		const node = folderNodes.value.get(path);
		if (!node) continue;
		rows.push({ node, depth, hasMatches: folderMatches.value.get(folderMatchKey(path)) });
		if (node.expanded) {
			for (let i = node.children.length - 1; i >= 0; i--) pending.push({ path: node.children[i], depth: depth + 1 });
		}
	}
	return rows;
});

function resetFolderTree(directory: string) {
	const parts = splitFolderPath(directory);
	const nodes = new Map<string, FolderNode>();
	parts.forEach((name, index) => {
		const path = folderPathAt(directory, index);
		nodes.set(path, { path, name, children: index + 1 < parts.length ? [folderPathAt(directory, index + 1)] : [],
			expanded: index + 1 < parts.length, loaded: false, childrenListed: false, loading: false });
	});
	folderNodes.value = nodes;
	treeRoot.value = parts.length ? folderPathAt(directory, 0) : '';
	selectedFolder.value = parts.length ? folderPathAt(directory, parts.length - 1) : '';
}

function removeCachedFolderBranch(path: string) {
	const key = folderMatchKey(path);
	const prefix = key.replace(/[\\/]+$/, '') + (path.startsWith('/') ? '/' : '\\');
	for (const savedPath of folderNodes.value.keys()) {
		const savedKey = folderMatchKey(savedPath);
		if (savedKey === key || savedKey.startsWith(prefix)) folderNodes.value.delete(savedPath);
	}
	for (const savedKey of folderMatches.value.keys()) {
		if (savedKey === key || savedKey.startsWith(prefix)) folderMatches.value.delete(savedKey);
	}
}

async function loadChildFolders(path: string) {
	const nodes = folderNodes.value;
	const node = nodes.get(path);
	if (!node || node.loaded || node.loading) return;
	node.loading = true;
	try {
		const names = await invoke<string[]>('list_child_folders', { directory: path });
		if (disposed || folderNodes.value !== nodes) return;
		const separator = path.startsWith('/') ? '/' : '\\';
		const children = names.map(name => {
			const child = path.replace(/[\\/]+$/, '') + separator + name;
			if (!nodes.has(child)) nodes.set(child, { path: child, name, children: [], expanded: false, loaded: false, childrenListed: false, loading: false });
			return child;
		});
		// Only successful enumeration proves a cached child has disappeared.
		for (const oldChild of node.children) {
			if (children.includes(oldChild)) continue;
			removeCachedFolderBranch(oldChild);
		}
		node.children = children;
		node.loaded = true;
		node.childrenListed = true;
	} catch (error) {
		if (!disposed && folderNodes.value === nodes) {
			await appendMessage({ text: `子フォルダー一覧を取得できませんでした: ${path}: ${String(error)}`, warning: true });
		}
	} finally {
		node.loading = false;
		scheduleFolderSave();
	}
}

async function toggleFolder(path: string) {
	const node = folderNodes.value.get(path);
	if (!node || closing.value || restoringFolders) return;
	node.expanded = !node.expanded;
	if (node.expanded) await loadChildFolders(path);
	scheduleFolderSave();
}

async function selectFolder(path: string) {
	if (initializing.value || writing.value || closing.value || disposed) return;
	const node = folderNodes.value.get(path);
	if (!node) return;
	if (activeOptionTab.value !== 'file-info') setOptionTab('files');
	node.expanded = true;
	const children = loadChildFolders(path);
	if (path === selectedFolder.value) { await children; return; }
	selectedFolder.value = path;
	await Promise.all([initializeSource(undefined, path, true), children]);
	scheduleFolderSave();
}

onMounted(async () => {
	await initializeEdition();
	if (disposed) return;
	if (features.value.optionPanel) {
        restoreHistoryPathRules();
        restoreOptionTab();
    }
	// Synchronous restore completes before registering drops, avoiding startup races.
	restoreDropHistory();
	window.addEventListener('keydown', onRefreshKey, true);
	window.addEventListener('pagehide', saveFolderState);
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
			if (disposed) return;
			if (writing.value || closing.value) {
				dragging.value = false;
				if (event.payload.type !== 'leave') showNotice(closing.value ? '終了処理中のため変更できません' : 'SD処理中のため変更できません');
				return;
			}
			dragging.value = event.payload.type === 'enter' || event.payload.type === 'over';
			if (event.payload.type === 'drop') {
				// Record accepted operations before loading; failures remain in history.
				dropHistory.value.push({
					id: crypto.randomUUID(),
					droppedAt: Date.now(),
					paths: [...event.payload.paths],
				});
				setOptionTab('files');
				void initializeSource(event.payload.paths);
				saveDropHistory();
			}
		});
		if (disposed) { unlisten(); return; }
		unlistenDrop = unlisten;
	} catch (error) {
		await appendMessage(`ドロップ受付の開始に失敗しました: ${String(error)}`);
	}
	if (disposed) return;
	// A drop received during startup takes priority over the exe directory.
	if (generation === 0 && !await restoreFolderState() && !disposed && generation === 0) await initializeSource();
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

async function initializeSource(paths?: string[], refreshDirectory?: string, preserveTree = false, pathSource: 'drop' | 'history' | 'registered' = 'drop') {
	if (closing.value) return;
	if (writing.value) { showNotice('SD処理中のため変更できません'); return; }
	if (paths) messages.value = [];
	const current = ++generation;
	const active = () => !disposed && current === generation;
	initializing.value = true;
	directoryFilesRequest++;
	directoryFiles.value = [];
	directoryFileIcons.value = new Map();
	tsFileIconsRequest++;
	tsFileIcons.value = new Map();
	directoryFilesLoading.value = false;
	directoryFilesError.value = '';
	outputRequest++;
	outputLoading.value = false;
	outputResult.value = null;
	outputError.value = '';
	if (!preserveTree) {
		resetFolderTree('');
	}
	ready = false;
	sdDirectDictionary.value = new Map();
	sdloadingDictionary.value = new Map();
	sdAddressRange.value = null;
	matchedFiles.value = [];
	fileMetadata.value = new Map();
	fileIndividualInfos.value = new Map();
	fileUpdates.value = { files: {}, warnings: [] };
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
		if (!preserveTree) resetFolderTree(directory);
		if (preserveTree) await appendMessage(`フォルダー選択：${directory.replace(/[\\/]+$/, '').split(/[\\/]/).pop() || directory}`);
		else if (refreshDirectory) await appendMessage('F5：表示を更新');
		else if (paths) await appendMessage(pathSource === 'history' ? '履歴から再読込' : pathSource === 'registered' ? '登録フォルダーを選択' : 'ドラッグ＆ドロップ');
		await nextTick();
		if (!active()) return;
		if (!preserveTree && pathList.value) pathList.value.scrollTop = pathList.value.scrollHeight;
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
			folderMatches.value.set(folderMatchKey(directory), false);
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
		if (active()) {
			if (!preserveTree && displayedDirectory.value) resetFolderTree(displayedDirectory.value);
			initializing.value = false;
			if (activeOptionTab.value === 'file-info') await refreshDirectoryFiles();
			if (activeOptionTab.value === 'output') await refreshOutputFiles();
			if (!preserveTree) {
				await nextTick();
				if (active() && pathList.value) pathList.value.scrollTop = pathList.value.scrollHeight;
			}
			scheduleFolderSave();
		}
	}
}

onUnmounted(() => {
	cancelHistoryClear();
	saveFolderState();
	clearTimeout(folderSaveTimer);
	window.removeEventListener('pagehide', saveFolderState);
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
		const result = await invoke<MatchedFileResult>('find_dictionary_files', {
			filenames: [...sdDirectDictionary.value.keys()],
			directory: sourceDirectory.value,
		});
		if (disposed || current !== generation) return;
		const warningsChanged = JSON.stringify(warningFiles.value) !== JSON.stringify(result.warning_files);
		const updates = result.file_updates ?? { files: {}, warnings: [] };
		const updateWarningsChanged = JSON.stringify(fileUpdates.value.warnings) !== JSON.stringify(updates.warnings);
		const changed = warningsChanged || updateWarningsChanged || useInputDirectory.value !== result.from_input || JSON.stringify(matchedFiles.value) !== JSON.stringify(result.files);
		const metadataChanged = result.files.some(filename => {
			const previous = fileMetadata.value.get(filename);
			const next = result.file_metadata?.[filename];
			return previous?.modified_at !== next?.modified_at || previous?.size !== next?.size;
		});
		const matchedDictionary = new Map<string, SdArea>();
		for (const filename of result.files) {
			const range = sdDirectDictionary.value.get(filename);
			if (range) matchedDictionary.set(filename, { ...range });
		}
		sdloadingDictionary.value = matchedDictionary;
		folderMatches.value.set(folderMatchKey(sourceDirectory.value), matchedDictionary.size > 0);
		matchedFiles.value = result.files;
		fileMetadata.value = new Map(Object.entries(result.file_metadata ?? {}));
		fileIndividualInfos.value = new Map(Object.entries(result.individual_infos ?? {}));
		fileUpdates.value = updates;
		warningFiles.value = result.warning_files;
		useInputDirectory.value = result.from_input;
		if (features.value.optionPanel && (report || changed || metadataChanged)) {
			const request = ++tsFileIconsRequest;
			tsFileIcons.value = new Map();
			void loadFileIcons(displayedDirectory.value, writeFiles.value,
				() => !disposed && current === generation && request === tsFileIconsRequest,
				icons => { tsFileIcons.value = new Map([...tsFileIcons.value, ...Object.entries(icons)]); });
		}
		if (matchedDictionary.size > 0) folderMatches.value.set(folderMatchKey(displayedDirectory.value), true);
		scheduleFolderSave();
		lastMatchError = '';
		if (!report && !changed) return;
		if (result.from_input && result.files.length) {
			useInputDirectory.value = true;
			await nextTick();
			if (disposed || current !== generation) return;
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
		if ((report || updateWarningsChanged) && updates.warnings.length) {
			await appendMessage(...updates.warnings.map(text => ({ text: `警告：${text}`, warning: true as const })));
			if (disposed || current !== generation) return;
		}
		if (!result.files.length) {
			await appendMessage('書き込み対象ファイルはありません');
		}
	} catch (error) {
		if (disposed || current !== generation) return;
		matchedFiles.value = [];
		fileMetadata.value = new Map();
		fileIndividualInfos.value = new Map();
		fileUpdates.value = { files: {}, warnings: [] };
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

async function openDiagnosticLog() {
    try { await invoke('open_diagnostic_log'); }
    catch (error) { transientNotice.value = `診断ログを開けませんでした: ${String(error)}`; }
}

async function onSdWrite(drive: string) {
	if (writeDialogOpen.value || writing.value || closing.value || initializing.value || !drive || !sourceDirectory.value) return;
    void appendMessage('「SD書込」ボタン押下');
	if (!progressUnlisten) { await appendMessage('進捗通知を受信できません。アプリを再起動してください'); return; }
    operationKind.value = 'write';
    operationError.value = '';
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
		operationError.value = String(error);
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
    operationError.value = '';
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
        operationError.value = String(error);
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
        if (activeOptionTab.value === 'output') await refreshOutputFiles();
    }
}

async function onSdFfInitialize(drive: string) {
    if (writeDialogOpen.value || writing.value || closing.value || initializing.value || !drive || !sourceDirectory.value) return;
    if (!progressUnlisten) { await appendMessage('進捗通知を受信できません。アプリを再起動してください'); return; }
    operationKind.value = 'ff';
    operationError.value = '';
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
        operationError.value = String(error);
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
    operationError.value = '';
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
        operationError.value = String(error);
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
        operationError.value = String(error);
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
        const text = writeLog.value.map(message => typeof message === 'string' ? message : message.text);
        if (failureDetails.value) text.push(`${failureDetails.value.causeLabel}: ${failureDetails.value.cause}`, `対処: ${failureDetails.value.action}`, failureDetails.value.note);
        await navigator.clipboard.writeText(text.join('\n'));
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

function onFolderDoubleClick(path: string, event: MouseEvent) {
	window.getSelection()?.selectAllChildren(event.currentTarget as Node);
	void openFolder(path);
}

const openingFile = ref(false);
const openingDailyReport = ref(false);
const openingReceipt = ref(false);
async function openReceipt(output = false) {
    const directory = output ? outputDirectory.value : displayedDirectory.value;
    if (!directory || openingReceipt.value || initializing.value || writing.value || closing.value || (output && outputLoading.value)) return;
    openingReceipt.value = true;
    try {
        await invoke('open_receipt', { directory });
    } catch (error) {
        await appendMessage({ text: `レシートを開けませんでした: ${String(error)}`, warning: true });
    } finally {
        openingReceipt.value = false;
    }
}
async function openDailyReport(filename: string) {
    if (!features.value.optionPanel || openingDailyReport.value || initializing.value || writing.value
        || closing.value || outputLoading.value) return;
    if (!outputFiles.value.some(file => file.filename === filename && file.description === '自動日報2データ')) return;
    openingDailyReport.value = true;
    logButtonPress('日報解析');
    try {
        await invoke('open_daily_report', { filename });
    } catch (error) {
        await appendMessage({ text: `日報解析を開けませんでした: ${filename}: ${String(error)}`, warning: true });
    } finally {
        openingDailyReport.value = false;
    }
}
async function openGridFile(filename: string, output = false, general = false) {
    const directory = general ? sourceDirectory.value : output ? outputDirectory.value : displayedDirectory.value;
    const files = general ? generalFiles.value : output ? outputFiles.value : optionFiles.value;
    if (openingFile.value || initializing.value || writing.value || closing.value || (output && outputLoading.value) || (general && directoryFilesLoading.value) || !directory) return;
    if (!files.some(file => file.filename === filename)) return;
    openingFile.value = true;
    try {
        await invoke('open_grid_file', { directory, filename });
    } catch (error) {
        await appendMessage(`ファイルを開けませんでした: ${filename}: ${String(error)}`);
    } finally {
        openingFile.value = false;
    }
}

async function openFolder(directory: string) {
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

function resizeOptions(event: PointerEvent) {
	const target = event.currentTarget as HTMLElement;
	const width = target.parentElement!.clientWidth - target.offsetWidth;
	if (width <= 0) return;
	const startX = event.clientX;
	const initialWidth = width * baseWidthRatio.value;
	const minimumBaseWidth = Math.min(280, width / 3);
	const minimumOptionWidth = Math.min(80, width / 3);
	target.setPointerCapture(event.pointerId);
	const move = (moveEvent: PointerEvent) => {
		const baseWidth = Math.max(minimumBaseWidth, Math.min(width - minimumOptionWidth, initialWidth + moveEvent.clientX - startX));
		baseWidthRatio.value = baseWidth / width;
	};
	const stop = () => {
		target.removeEventListener('pointermove', move);
		target.removeEventListener('lostpointercapture', stop);
	};
	target.addEventListener('pointermove', move);
	target.addEventListener('lostpointercapture', stop);
}

function resize(event: PointerEvent, divider: 1 | 2) {
	const target = event.currentTarget as HTMLElement;
	const startY = event.clientY;
	const initialMenuHeight = Math.max(menuHeight.value, minimumMenuHeight.value);
	const initialSdHeight = target.parentElement!.querySelector<HTMLElement>('.panel_sd')!.offsetHeight;
	const textHeight = target.parentElement!.querySelector<HTMLElement>('.panel_text')!.offsetHeight;
	const availableHeight = initialSdHeight + textHeight;
	if (availableHeight <= 0) return;
	target.setPointerCapture(event.pointerId);

	const move = (moveEvent: PointerEvent) => {
		const delta = moveEvent.clientY - startY;
		if (divider === 1) {
			const change = Math.max(minimumMenuHeight.value - initialMenuHeight, Math.min(delta, initialSdHeight));
			menuHeight.value = initialMenuHeight + change;
			sdHeightRatio.value = (initialSdHeight - change) / Math.max(1, availableHeight - change);
		} else {
			sdHeightRatio.value = (initialSdHeight + Math.max(-initialSdHeight, Math.min(delta, Math.max(0, textHeight - 66)))) / availableHeight;
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
		<div class="panel-inner" :class="{ 'has-options': features.optionPanel }"
			:style="features.optionPanel ? { gridTemplateColumns: `minmax(0, ${baseWidthRatio}fr) 1px minmax(0, ${1 - baseWidthRatio}fr)` } : undefined">
			<div class="panel-base" :style="{ gridTemplateRows: `${Math.max(menuHeight, minimumMenuHeight)}px 1px minmax(0, ${sdHeightRatio}fr) 1px minmax(0, ${1 - sdHeightRatio}fr) minmax(44px, auto)` }">
				<div id="panel_menu" class="panel_menu">
					<div ref="menuContent" class="menu-content">
						<SdMenu ref="sdMenu" :required-capacity="requiredSdCapacity" :can-write="canSdWrite" :can-read="canSdRead" :can-initialize="canSdRead" :busy="initializing || writing || closing || writeDialogOpen" :write-label="closing ? '終了待ち…' : writing ? `${statusLabel}…` : 'SD書込'"
							@write="onSdWrite" @read="onSdRead" @initialize="onSdFfInitialize" @format="onSdFormat" @action="logButtonPress" @error="appendMessage" @warning="text => appendMessage({ text, warning: true })" />
					</div>
				</div>
				<div id="resizer1" class="resizer" @pointerdown="resize($event, 1)"></div>
				<section id="panel_sd" class="panel_sd" aria-label="表示中のフォルダー">
					<div class="path-header sd-path-header">
						<FolderCombo v-if="features.folderSelector" :rules="historyPathRules" :selected-folder="sourceDirectory" :disabled="historyBusy" @select="selectRegisteredFolder" />
						<span v-else class="folder-selector-placeholder" aria-hidden="true"></span>
						<button type="button" class="copy-path" :disabled="!sourceDirectory" @click="copyDirectory" :title="displayedDirectory">
							<svg viewBox="0 0 16 16" aria-hidden="true"><rect x="5" y="2" width="8" height="10" rx="1.5"/><path d="M10 14H4a1 1 0 0 1-1-1V5"/></svg>
							パスをコピー
						</button>
					</div>
					<p v-if="pathError" class="path-message path-error" role="alert">{{ pathError }}</p>
					<ol v-else-if="visibleFolders.length" ref="pathList" class="path-list" aria-label="対象フォルダーの階層" @scroll.passive="scheduleFolderSave">
						<li v-for="{ node, depth, hasMatches } in visibleFolders" :key="node.path"
							class="path-row" :class="{ 'path-current': node.path === selectedFolder }"
							:aria-current="node.path === selectedFolder ? 'location' : undefined" :aria-busy="node.loading">
							<span class="path-guides" :style="{ width: `${depth * 14}px` }" aria-hidden="true"></span>
							<button v-if="!node.loaded || node.children.length" type="button" class="folder-toggle"
								:aria-expanded="node.expanded" :aria-label="`${node.name}の子階層を${node.expanded ? '折りたたむ' : '展開'}`"
								@click="toggleFolder(node.path)">
								<svg viewBox="0 0 12 12" aria-hidden="true" :class="{ 'folder-expanded': node.expanded }"><path d="m4 2 4 4-4 4"/></svg>
							</button>
							<span v-else class="folder-toggle-spacer" aria-hidden="true"></span>
							<svg class="folder-icon" viewBox="0 0 16 16" aria-hidden="true"><path d="M1.5 4a1 1 0 0 1 1-1H6l1.5 2h6a1 1 0 0 1 1 1v6a1 1 0 0 1-1 1h-11a1 1 0 0 1-1-1Z"/></svg>
							<button type="button" class="path-name" :class="{ 'folder-has-matches': hasMatches === true }"
								:title="`${node.path}\n${hasMatches === undefined ? '対象ファイルは未確認' : `最後の確認結果：対象ファイル${hasMatches ? 'あり' : 'なし'}`}\nクリックで選択・ダブルクリックでエクスプローラーで開く`"
								@click="selectFolder(node.path)"
								@dblclick.prevent="onFolderDoubleClick(node.path, $event)"><span v-if="hasMatches" class="folder-match-dot" aria-label="対象ファイルあり（最後の確認結果）">● </span>{{ node.name }}</button>
							<span v-if="node.loading" class="folder-loading" role="status">読込中…</span>
						</li>
					</ol>
					<p v-else class="path-message" role="status">保存先を取得中…</p>
				</section>
				<div id="resizer2" class="resizer" @pointerdown="resize($event, 2)"></div>
				<div id="panel_text" class="panel_text">
					<div class="path-header text-header">
						<h2>処理ログ</h2>
						<button type="button" class="clear-messages" :disabled="messages.length === 0" @click="messages = []" aria-label="操作メッセージをクリア">
							<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M2.5 4.5h11M6 4.5V2.8a.8.8 0 0 1 .8-.8h2.4a.8.8 0 0 1 .8.8v1.7M4 4.5l.7 8.5a1 1 0 0 0 1 1h4.6a1 1 0 0 0 1-1l.7-8.5M6.5 7v4.5M9.5 7v4.5"/></svg>
							クリア
						</button>
					</div>
					<div class="map-summary">
						<div class="map-summary-row">
							<span class="map-heading" role="status">SD MAP <span class="map-count">{{ sdAddressRange ? `${sdDirectDictionary.size}件` : '未取得' }}</span></span>
							<button v-if="features.mapDetails" type="button" class="map-details-toggle" :aria-expanded="mapDetailsOpen" aria-controls="map-address-range" @click="mapDetailsOpen = !mapDetailsOpen">
								詳細<svg viewBox="0 0 12 12" aria-hidden="true" :class="{ 'details-open': mapDetailsOpen }"><path d="m3 4.5 3 3 3-3"/></svg>
							</button>
						</div>
						<p v-if="features.mapDetails" v-show="mapDetailsOpen" id="map-address-range" class="map-address">{{ sdAddressRange ? `${sdAddressRange.startAddress} - ${sdAddressRange.endAddress}` : 'アドレス範囲は未取得です' }}</p>
					</div>
					<div ref="textPanel" class="text-messages" role="log" aria-label="操作メッセージ">
						<p v-for="(message, index) in messages" :key="index" class="path-message"
							:class="{ 'path-warning': message.warning }"><time class="message-time">{{ message.time }}</time><span class="message-body"><svg v-if="message.warning" class="message-warning-icon" viewBox="0 0 16 16" role="img" aria-label="警告"><path fill="currentColor" d="M8 1 0 15h16Z"/><path stroke="#fff" stroke-width="1.5" d="M8 5v5m0 1v1.5"/></svg>{{ message.text }}</span></p>
					</div>
				</div>
				<div id="panel_footer" class="panel_footer" :class="{ 'drop-unavailable': initializing || writing || closing }" role="status">
					<div class="drop-hint"><svg v-if="!initializing && !writing && !closing" viewBox="0 0 24 24" aria-hidden="true"><path d="M14 21H5a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h9l5 5v5M14 3v6h5M18 15v8m-4-4h8"/></svg><span>{{ closing ? '終了処理中：SD操作プロセスの終了を確認しています' : writing ? 'SD処理中：ドライブ・対象フォルダーの変更と再読込はできません' : dragging ? 'ドロップして対象フォルダーを変更' : initializing ? '対象フォルダーを読込中…' : 'ファイル・フォルダーをドロップ' }}</span></div>
					<div v-if="transientNotice" class="path-error">{{ transientNotice }}</div>
						<div v-if="historyStorageError" class="path-error" role="alert">{{ historyStorageError }}</div>
				</div>
			</div>
			<div v-if="features.optionPanel" id="resizer3" class="resizer resizer-horizontal" @pointerdown="resizeOptions"></div>
			<section v-if="features.optionPanel" id="panel_option" class="panel_option" aria-label="オプション"
				:style="{ '--option-menu-height': `${Math.max(menuHeight, minimumMenuHeight)}px` }">
				<div class="option-header">
				<div class="option-tabs" role="tablist" aria-label="表示内容">
					<button v-for="(tab, index) in optionTabs" :id="`option-tab-${tab.id}`" :key="tab.id"
						type="button" role="tab" :aria-selected="activeOptionTab === tab.id"
						:aria-controls="`option-panel-${tab.id}`" :tabindex="activeOptionTab === tab.id ? 0 : -1"
						@click="selectOptionTab(tab.id)" @keydown="navigateOptionTab($event, index)">{{ tab.label }}</button>
				</div>
				<details v-if="activeOptionTab === 'history'" ref="historySettingsMenu" class="history-settings"
					@keydown.esc.stop.prevent="closeHistorySettingsMenu(); ($event.currentTarget as HTMLElement).querySelector('summary')?.focus()"
					@focusout="!($event.currentTarget as HTMLElement).contains($event.relatedTarget as Node | null) && closeHistorySettingsMenu()">
					<summary title="履歴の設定" aria-label="履歴の設定"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="m9 3-.6 2.4-2 .9-2.2-.7-2 3.4 1.7 1.7v2.6L2.2 15l2 3.4 2.2-.7 2 .9L9 21h4l.6-2.4 2-.9 2.2.7 2-3.4-1.7-1.7v-2.6L19.8 9l-2-3.4-2.2.7-2-.9L13 3Z"/><circle cx="11" cy="12" r="3"/></svg></summary>
					<div class="history-settings-menu">
						<button type="button" :disabled="closing" @click="closeHistorySettingsMenu(); editHistoryPathRules()">パスの省略設定…</button>
						<button type="button" @click="closeHistorySettingsMenu(); historyHelpDialog?.showModal()">操作方法</button>
						<button type="button" class="history-settings-clear" :disabled="closing || !historyGroups[1].entries.length" @click="closeHistorySettingsMenu(); requestHistoryClear()">履歴をクリア…</button>
					</div>
				</details>
				</div>
				<div v-show="activeOptionTab === 'files'" id="option-panel-files" class="option-tab-panel"
					role="tabpanel" aria-labelledby="option-tab-files" tabindex="0">
					<OptionGrid :directory="displayedDirectory" :files="optionFiles" :disabled="initializing || writing || closing" @open-receipt="openReceipt()" @open-file="openGridFile" />
				</div>
				<div v-show="activeOptionTab === 'file-info'" id="option-panel-file-info" class="option-tab-panel"
					role="tabpanel" aria-labelledby="option-tab-file-info" tabindex="0" :aria-busy="directoryFilesLoading">
					<p v-if="directoryFilesLoading" class="output-status" role="status">ファイルを読み込んでいます…</p>
					<p v-else-if="directoryFilesError" class="output-status" role="alert">{{ directoryFilesError }}</p>
					<p v-else-if="!generalFiles.length" class="output-status" role="status">ファイルがありません。</p>
					<OptionGrid variant="general" :directory="sourceDirectory" :files="generalFiles" :disabled="historyBusy || directoryFilesLoading" @open-file="openGridFile($event, false, true)" />
				</div>
                <div v-show="activeOptionTab === 'output'" id="option-panel-output" class="option-tab-panel output-panel"
                    role="tabpanel" aria-labelledby="option-tab-output" tabindex="0" :aria-busy="outputLoading">
                    <p v-if="outputLoading" class="output-status" role="status">outputを読み込んでいます…</p>
                    <p v-else-if="outputError" class="output-status" role="alert">{{ outputError }}</p>
                    <p v-else-if="!outputFiles.length" class="output-status" role="status">outputにCSVと一致するファイルがありません。全エリアSD読込後に表示されます。</p>
                    <OptionGrid variant="output" :directory="outputDirectory" :files="outputFiles" :disabled="initializing || writing || closing || outputLoading || openingDailyReport" @open-receipt="openReceipt(true)" @open-file="openGridFile($event, true)" @analyze-daily-report="openDailyReport" />
                </div>
				<div v-show="activeOptionTab === 'history'" id="option-panel-history" class="option-tab-panel history-panel"
					role="tabpanel" aria-labelledby="option-tab-history" tabindex="0">
					<p v-if="historyPathError && !historyPathDialog?.open" class="history-settings-error" role="alert">{{ historyPathError }}</p>
					<section v-for="group in historyGroups" :key="group.id" class="history-section" :class="`history-${group.id}`"
						:aria-label="group.label">
					<div class="history-list">
						<table class="history-table" :aria-label="group.label">
							<colgroup><col class="history-favorite-column"><col class="history-date-column"><col><col class="history-delete-column"></colgroup>
							<thead><tr><th scope="col" class="history-favorite-cell" aria-label="お気に入り"></th><th scope="col">日時</th><th scope="col">保存先パス</th><th scope="col">削除</th></tr></thead>
							<tbody>
								<tr v-for="entry in group.entries" :key="entry.id" :class="{ 'history-selected': selectedHistoryId === entry.id }"
									:style="historyPathsColorStyle(entry.paths, historyPathRules)"
									@click="selectHistory(entry.id)">
									<td class="history-favorite-cell">
										<button type="button" class="history-favorite" :class="{ 'is-favorite': entry.favorite }"
											:aria-pressed="!!entry.favorite" :aria-label="entry.favorite ? 'お気に入りを解除' : 'お気に入りに登録'"
											:title="entry.favorite ? 'お気に入りを解除' : 'お気に入りに登録'" :disabled="closing"
											@click.stop="toggleHistoryFavorite(entry.id)" @dblclick.stop.prevent @keydown.enter.stop>{{ entry.favorite ? '★' : '☆' }}</button>
									</td>
									<td class="history-date"><time :datetime="new Date(entry.droppedAt).toISOString()">{{ formatFileTimestamp(entry.droppedAt) }}</time></td>
									<td>
										<div v-for="(path, index) in entry.paths" :key="index" class="history-path-row" :style="historyPathsColorStyle([path], historyPathRules)">
										<button type="button" class="history-path" :title="`${path}\nダブルクリック、または選択してEnterで再読込`" :aria-label="path"
											:disabled="historyBusy" @focus="selectHistory(entry.id)"
											@dblclick.prevent="reopenHistory(entry.id)" @keydown.enter.prevent="!$event.repeat && reopenHistory(entry.id)">
											<span v-if="displayHistoryPath(path).label" class="history-path-label" aria-hidden="true">[{{ displayHistoryPath(path).label }}] </span>
											<span v-if="displayHistoryPath(path).path.length > 24" class="history-path-prefix" aria-hidden="true">{{ displayHistoryPath(path).path.slice(0, -24) }}</span>
											<span class="history-path-suffix" :class="{ 'has-prefix': displayHistoryPath(path).path.length > 24 }" aria-hidden="true"><span>{{ displayHistoryPath(path).path.slice(-24) }}</span></span>
										</button>
										<button type="button" class="history-copy" title="フルパスをコピー" :aria-label="`${path} をコピー`"
											@click.stop="copyHistoryPath(entry.id, index)" @dblclick.stop.prevent @keydown.enter.stop>
											<svg viewBox="0 0 16 16" aria-hidden="true"><rect x="5" y="2" width="8" height="10" rx="1.5"/><path d="M10 14H4a1 1 0 0 1-1-1V5"/></svg>
										</button>
										</div>
									</td>
									<td class="history-delete-cell">
										<button type="button" class="history-delete" aria-label="この履歴を削除"
											:title="entry.favorite ? 'お気に入りを解除すると削除できます' : 'この履歴を削除（元のファイルは削除しません）'"
											:disabled="!!entry.favorite || closing" @click.stop="deleteHistory(entry.id)" @dblclick.stop.prevent @keydown.enter.stop>
											<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M2 4h12M6 4V2h4v2M4 4l.7 10h6.6L12 4M6.5 6v6M9.5 6v6"/></svg>
										</button>
									</td>
								</tr>
							</tbody>
						</table>
						<p v-if="!group.entries.length" class="history-empty" role="status">{{ group.id === 'favorites' ? 'お気に入りはありません。☆で登録できます。' : '通常の履歴はありません。ファイル・フォルダーをドロップすると記録されます。' }}</p>
					</div>
					</section>
				</div>
			</section>
		</div>
	</div>
	<dialog v-if="features.optionPanel" ref="historyHelpDialog" class="history-clear-dialog" aria-labelledby="history-help-title">
		<h2 id="history-help-title">履歴の操作方法</h2>
		<p>パスをダブルクリック、または選択してEnterで再読込します。</p>
		<p>☆でお気に入りに登録し、★で解除します。パスの横のコピーアイコンでフルパスをコピーできます。</p>
		<div class="history-clear-actions"><button type="button" @click="historyHelpDialog?.close()">閉じる</button></div>
	</dialog>
	<dialog v-if="features.optionPanel" ref="historyPathDialog" class="history-clear-dialog history-path-dialog" aria-labelledby="history-path-title"
		@cancel.prevent="cancelHistoryPathRules">
		<form @submit.prevent="saveHistoryPathRules">
			<h2 id="history-path-title">パスの省略設定</h2>
			<p>登録フォルダーとその配下を「[表示名] 相対パス」で表示します。複数に一致すると、最も深いフォルダーを優先します。</p>
			<div class="history-rule-list">
				<table class="history-rule-table" aria-label="パスの省略設定一覧">
					<colgroup><col class="history-rule-number-column"><col class="history-rule-label-column"><col class="history-rule-filter-column"><col><col class="history-rule-color-column"><col class="history-rule-delete-column"></colgroup>
					<thead><tr><th scope="col">No</th><th scope="col">表示名</th><th scope="col">フィルタ</th><th scope="col">省略するフォルダー</th><th scope="col">色</th><th scope="col">削除</th></tr></thead>
					<tbody>
				<tr v-for="(rule, index) in historyPathEditorRows" :key="index" class="history-rule-editor">
					<td class="history-rule-number">{{ index + 1 }}</td>
					<td><input v-model="rule.label" type="text" :style="historyPathColorStyle(rule.color)" :disabled="closing || index < fixedHistoryPathRules.length" :aria-label="`${index + 1}行目の表示名`" placeholder="共有"></td>
					<td><select :value="rule.filter ?? 'none'" :disabled="closing || index < fixedHistoryPathRules.length" :aria-label="`${index + 1}行目のフィルタ`" @change="rule.filter = ($event.target as HTMLSelectElement).value as HistoryPathRule['filter']">
						<option value="ts">TS用</option><option value="none">なし</option>
					</select></td>
					<td><input v-model="rule.folder" type="text" :disabled="closing || index < fixedHistoryPathRules.length" :aria-label="`${index + 1}行目の省略するフォルダー`" placeholder="C:\作業"></td>
					<td><select :value="rule.color ?? 'blue'" :style="historyPathColorStyle(rule.color)" :disabled="closing || index < fixedHistoryPathRules.length" :aria-label="`${index + 1}行目の色`" @change="rule.color = ($event.target as HTMLSelectElement).value as HistoryPathRule['color']">
						<option value="none" style="color: CanvasText; background-color: Canvas">なし</option>
						<option v-for="(preset, colorIndex) in historyPathColors" :key="preset.id" :value="preset.id" :style="historyPathColorStyle(preset.id)">{{ colorIndex + 1 }}：{{ preset.label }}</option>
					</select></td>
					<td><span v-if="index < fixedHistoryPathRules.length">固定</span><button v-else type="button" :disabled="closing" :aria-label="`${index + 1}行目を削除`" @click="historyPathDraft.splice(index - fixedHistoryPathRules.length, 1)">削除</button></td>
				</tr>
					</tbody>
				</table>
			</div>
			<p>色は「なし」と8パターンから選べます。表示名と履歴の行に反映し、複数の保存先がある行はパスごとに色分けします。</p>
			<p v-if="historyPathError" class="history-settings-error" role="alert">{{ historyPathError }}</p>
			<div class="history-clear-actions">
				<button type="button" class="history-rule-add" :disabled="closing" @click="historyPathDraft.push({ label: '', folder: '', color: 'none' })">追加</button>
				<button type="button" autofocus @click="cancelHistoryPathRules">キャンセル</button>
				<button type="submit" :disabled="closing">保存</button>
			</div>
		</form>
	</dialog>
	<dialog v-if="features.optionPanel" ref="historyClearDialog" class="history-clear-dialog" aria-labelledby="history-clear-title"
		aria-describedby="history-clear-description" @cancel.prevent="cancelHistoryClear">
		<h2 id="history-clear-title">履歴をクリア</h2>
		<p>保護されていない履歴をすべて削除しますか？</p>
		<p id="history-clear-description">お気に入りは残ります。元のファイルやフォルダーは削除されません。</p>
		<div class="history-clear-actions">
			<button type="button" autofocus @click="cancelHistoryClear">キャンセル</button>
			<button type="button" class="history-clear-confirm" :disabled="closing" @click="confirmHistoryClear">削除する</button>
		</div>
	</dialog>
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
            <section v-if="failureDetails" class="failure-summary" role="alert" aria-label="失敗の確認結果と対処">
                <strong>確認できた内容</strong>
                <p class="failure-facts">{{ failureDetails.message }}</p>
                <p><strong>{{ failureDetails.causeLabel }}：</strong>{{ failureDetails.cause }}</p>
                <p><strong>対処：</strong>{{ failureDetails.action }}</p>
                <p class="failure-note">{{ failureDetails.note }}</p>
            </section>
            <div ref="writeLogPanel" class="write-log" :class="{ 'format-log': operationKind === 'format' }" role="log" :aria-label="`${operationName}ログ`" tabindex="0" @scroll="writeLogScroller.onScroll">
                <p v-for="(message, index) in writeLog" :key="index" :class="{ 'log-warning': typeof message !== 'string' && message.warning, 'log-done': typeof message === 'string' && (message.startsWith('全件完了') || message.startsWith('フォーマット完了 ')) }">{{ typeof message === 'string' ? message : message.text }}</p>
            </div>
            <p v-if="transientNotice" class="write-notice" role="status">{{ transientNotice }}</p>
            <div class="write-actions">
                <button v-if="operationKind === 'format' && writeStage === 'confirm'" type="button" class="close-write" :disabled="writing || !formatLabelValid" @click="startSdFormat">開始</button>
                <button v-if="operationKind === 'read'" type="button" class="copy-log open-output" :disabled="writing" @click="openOutput">outputを開く</button>
                <button v-if="failureDetails" type="button" class="copy-log" :disabled="writing" @click="openDiagnosticLog">診断ログを開く</button>
                <button type="button" class="copy-log" aria-live="polite" @click="copyWriteLog">{{ copyStatus || 'ログをコピー' }}</button>
                <button type="button" class="close-write" :disabled="writing" @click="closeWriteDialog">{{ operationKind === 'format' && writeStage === 'confirm' ? 'キャンセル' : '閉じる' }}</button>
            </div>
        </div>
    </dialog>
</template>

<style scoped>
.failure-summary { flex: 0 1 auto; min-height: 0; max-height: 50%; overflow: auto; padding: 7px 9px; border: 1px solid #e3bcb5; border-radius: 6px; background: #fff5f2; color: #6c251c; line-height: 1.4; overflow-wrap: anywhere; }
.failure-summary p { margin: 4px 0 0; }
.failure-facts { white-space: pre-wrap; }
.failure-note { font-size: 11px; }
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
.write-actions { display: flex; flex-wrap: wrap; flex-shrink: 0; justify-content: flex-end; align-items: center; gap: 6px; }
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
	border: 1px solid #dce2eb;
	padding: 1px;
	display: flex;
	justify-content: center;
	align-items: center;
	/* 外枠をビューポート高さに合わせ、中央配置に固定 */
	height: 100vh;
	box-sizing: border-box;
}
.panel-inner {
	border: 0;
	width: 100%;
	/* 高さは外枠に合わせ、はみ出す場合はスクロールさせる */
	height: 100%;
	max-height: 100%;
	box-sizing: border-box;
	padding: 1px;
	overflow: auto;
}
.panel-inner.has-options {
	display: grid;
	grid-template-rows: minmax(0, 1fr);
}
.panel_option {
	--option-grid-header-height: 24px;
	display: flex;
	flex-direction: column;
	min-width: 0;
	min-height: 0;
	overflow: hidden;
	background: #fff;
}
.option-header { display: flex; position: relative; flex-shrink: 0; background: #f5f9fe; border-bottom: 1px solid #d7e5f5; }
.option-header .option-tabs { flex: 1; min-width: 0; border-bottom: 0; }
.history-settings { align-self: center; margin: 0 6px; color: #52647c; }
.history-settings summary { display: flex; padding: 4px; border-radius: 3px; cursor: pointer; list-style: none; }
.history-settings summary::-webkit-details-marker { display: none; }
.history-settings summary:hover, .history-settings[open] summary { background: #eaf3ff; }
.history-settings summary:focus-visible, .history-settings-menu button:focus-visible { outline: 2px solid #2875d9; outline-offset: -2px; }
.history-settings summary svg { width: 18px; height: 18px; fill: none; stroke: currentColor; stroke-width: 1.5; }
.history-settings-menu { position: absolute; top: 100%; right: 6px; z-index: 5; min-width: 160px; padding: 4px; border: 1px solid #d7e5f5; border-radius: 5px; background: #fff; box-shadow: 0 4px 12px #0002; }
.history-settings-menu button { display: block; width: 100%; padding: 7px 10px; border: 0; border-radius: 3px; background: transparent; color: #26364d; font: 12px 'Yu Gothic UI', Meiryo, sans-serif; text-align: left; cursor: pointer; }
.history-settings-menu button:hover:not(:disabled) { background: #eaf3ff; }
.history-settings-menu button:disabled { opacity: .5; cursor: default; }
.history-settings-menu .history-settings-clear { margin-top: 4px; border-top: 1px solid #d7e5f5; border-radius: 0; }
.option-tabs {
	display: flex;
	flex-shrink: 0;
	/* グリッドの上枠1pxとヘッダー下線を、resizer1の1pxに揃える。 */
	height: calc(var(--option-menu-height) - var(--option-grid-header-height));
	border-bottom: 1px solid #d7e5f5;
	background: #f5f9fe;
}
.option-tabs button {
	padding: 0 12px;
	border: 0;
	border-bottom: 2px solid transparent;
	background: transparent;
	color: #52647c;
	font: 12px/18px 'Yu Gothic UI', Meiryo, sans-serif;
	cursor: pointer;
}
.option-tabs button:hover { background: #eaf3ff; }
.option-tabs button[aria-selected='true'] {
	border-bottom-color: #2875d9;
	background: #fff;
	color: #255b9f;
	font-weight: 600;
}
.option-tabs button:focus-visible { outline: 2px solid #2875d9; outline-offset: -3px; }
.option-tab-panel {
	flex: 1;
	min-width: 0;
	min-height: 0;
	overflow: hidden;
}
.output-panel { display: flex; flex-direction: column; }
.output-panel :deep(.option-grid) { flex: 1; min-height: 0; height: auto; }
.output-status { flex-shrink: 0; margin: 4px 6px; color: #52647c; font-size: 12px; }
.history-panel { display: flex; flex-direction: column; }
.history-settings-error { margin: 4px 6px; color: #a74444; font-size: 11px; overflow-wrap: anywhere; }
.history-clear-dialog.history-path-dialog { width: calc(100vw - 32px); height: calc(100vh - 32px); max-width: none; box-sizing: border-box; max-height: calc(100vh - 32px); overflow: auto; }
.history-path-dialog > form { display: flex; flex-direction: column; height: 100%; min-height: 0; }
.history-path-dialog > form > :not(.history-rule-list) { flex-shrink: 0; }
.history-rule-list { flex: 1; min-height: 0; overflow: auto; }
.history-rule-table { width: 100%; min-width: 560px; table-layout: fixed; border-collapse: separate; border-spacing: 0; }
.history-rule-number-column { width: 32px; }
.history-rule-label-column { width: 21%; }
.history-rule-filter-column { width: 80px; }
.history-rule-color-column { width: 96px; }
.history-rule-delete-column { width: 54px; }
.history-rule-table th, .history-rule-table td { padding: 5px; border-right: 1px solid #d7e5f5; border-bottom: 1px solid #d7e5f5; text-align: left; vertical-align: middle; }
.history-rule-table td.history-rule-number { text-align: center; }
.history-rule-table th:first-child, .history-rule-table td:first-child { border-left: 1px solid #d7e5f5; }
.history-rule-table th { position: sticky; top: 0; z-index: 1; background: #edf4fd; border-top: 1px solid #d7e5f5; font-weight: normal; }
.history-rule-table tbody tr:nth-child(even) { background: #f5f9fe; }
.history-rule-editor input, .history-rule-editor select { display: block; width: 100%; box-sizing: border-box; padding: 4px; border: 1px solid #b8c9df; border-radius: 3px; font: inherit; }
.history-rule-editor button, .history-rule-add { padding: 4px 8px; border: 1px solid #d7e5f5; border-radius: 3px; background: #fff; font: inherit; cursor: pointer; }
.history-path-label { flex-shrink: 0; max-width: 30%; overflow: hidden; text-overflow: ellipsis; color: inherit; }
.history-clear-dialog { max-width: min(440px, calc(100vw - 32px)); padding: 16px; border: 1px solid #d7e5f5; border-radius: 7px; color: #26364d; font: 12px/1.6 'Yu Gothic UI', Meiryo, sans-serif; }
.history-clear-dialog::backdrop { background: #0005; }
.history-clear-dialog h2 { margin: 0 0 10px; font-size: 15px; }
.history-clear-actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 16px; }
.history-clear-actions button { padding: 4px 12px; border: 1px solid #d7e5f5; border-radius: 4px; background: #fff; font: inherit; cursor: pointer; }
.history-clear-actions .history-clear-confirm { color: #a74444; border-color: #e5c4c4; }
.history-clear:focus-visible, .history-clear-actions button:focus-visible { outline: 2px solid #2875d9; outline-offset: 1px; }
.history-section { display: flex; flex-direction: column; min-height: 0; min-width: 0; overflow: hidden; }
.history-favorites { flex: 0 1 auto; max-height: min(160px, 40%); border-bottom: 1px solid #d7e5f5; }
.history-recent { flex: 1; }
.history-section > .history-list { height: auto; }
.history-list {
	height: 100%;
	flex: 1;
	min-height: 0;
	overflow: auto;
	color: #26364d;
	font: 11px/18px 'Yu Gothic UI', Meiryo, sans-serif;
}
.history-table {
	width: 100%;
	min-width: 400px;
	table-layout: fixed;
	border-collapse: separate;
	border-spacing: 0;
}
.history-date-column { width: 136px; }
.history-favorite-column { width: 24px; }
.history-delete-column { width: 36px; }
.history-table .history-delete-cell { padding: 0; text-align: center; }
.history-delete { width: 22px; height: 18px; padding: 2px 4px; border: 0; border-radius: 2px; background: transparent; color: #a74444; cursor: pointer; }
.history-delete:hover:not(:disabled) { background: #fde5e5; }
.history-delete:focus-visible { outline: 2px solid #2875d9; outline-offset: -2px; }
.history-delete:disabled { opacity: .4; cursor: default; }
.history-delete svg { display: block; width: 14px; height: 14px; fill: none; stroke: currentColor; stroke-width: 1.2; }
.history-table .history-favorite-cell { padding: 0; text-align: center; }
.history-favorite { width: 24px; height: 18px; padding: 0; border: 0; border-radius: 2px; background: transparent; color: #52647c; font: 15px/18px sans-serif; cursor: pointer; }
.history-favorite.is-favorite { color: #986600; }
.history-favorite:hover { background: #cce0fa; }
.history-favorite:focus-visible { outline: 2px solid #2875d9; outline-offset: -2px; }
.history-favorite:disabled { opacity: .5; cursor: default; }
.history-table th, .history-table td {
	padding: 0 6px;
	border-right: 1px solid #edf1f6;
	border-bottom: 1px solid #e8edf4;
	text-align: left;
	vertical-align: top;
}
.history-table th {
	position: sticky;
	top: 0;
	z-index: 1;
	height: var(--option-grid-header-height, 24px);
	vertical-align: middle;
	background: #edf4fd;
	border-bottom-color: #d7e5f5;
}
.history-table th:last-child, .history-table td:last-child { border-right: 0; }
.history-table tbody tr:nth-child(even) { background: #f5f9fe; }
.history-table tbody tr:hover { background: #eaf3ff; }
.history-table tbody tr.history-selected { background: #dbeafe; }
.history-table tbody tr:hover td { box-shadow: inset 0 1px #2875d966, inset 0 -1px #2875d966; }
.history-table tbody tr.history-selected td { position: relative; z-index: 0; }
.history-table tbody tr.history-selected td::after { content: ''; position: absolute; inset: 0 -1px -1px 0; z-index: 1; border-top: 2px solid #2875d9; border-bottom: 2px solid #2875d9; box-sizing: border-box; pointer-events: none; }
.history-table tbody tr.history-selected td:first-child::after { border-left: 2px solid #2875d9; }
.history-table tbody tr.history-selected td:last-child::after { right: 0; border-right: 2px solid #2875d9; }
.history-date { white-space: nowrap; }
.history-path-row { display: flex; gap: 4px; min-width: 0; }
.history-path {
	display: flex; flex: 1; width: 100%; min-width: 0; padding: 0; border: 0;
	background: transparent; color: inherit; font: inherit; text-align: left; white-space: pre;
	cursor: pointer;
}
.history-path:disabled { cursor: default; }
.history-path:focus-visible { outline: 2px solid #2875d9; outline-offset: -2px; }
.history-copy { flex: 0 0 18px; width: 18px; height: 18px; padding: 2px; border: 0; border-radius: 2px; background: transparent; color: inherit; cursor: pointer; }
.history-copy:hover { background: #cce0fa; }
.history-copy:focus-visible { outline: 2px solid #2875d9; outline-offset: -2px; }
.history-copy svg { display: block; width: 14px; height: 14px; fill: none; stroke: currentColor; stroke-width: 1.2; }
.history-path-prefix { min-width: 0; overflow: hidden; text-overflow: ellipsis; }
.history-path-suffix { display: flex; justify-content: flex-end; min-width: 0; max-width: 100%; overflow: hidden; }
.history-path-suffix.has-prefix { flex-shrink: 0; max-width: 70%; }
.history-path-suffix > span { flex-shrink: 0; }
.history-empty { margin: 12px; color: #52647c; }
.panel-base {
	border: 0;
	min-width: 0;
	min-height: 0;
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
	margin: 5px 6px;
	padding: 6px;
	border: 1px dashed #83afe9;
	border-radius: 5px;
	background: #f0f6ff;
	color: #255b9f;
	font: 11px/16px 'Yu Gothic UI', Meiryo, sans-serif;
	overflow-wrap: anywhere;
}
.drop-hint { display: flex; align-items: center; justify-content: center; gap: 7px; min-height: 20px; }
.drop-hint svg { flex: 0 0 20px; width: 20px; height: 20px; fill: none; stroke: currentColor; stroke-width: 1.5; stroke-linecap: round; stroke-linejoin: round; }
.panel_footer.drop-unavailable { border-style: solid; border-color: #dce2eb; background: #f3f5f7; color: #64748b; }
.drop-active .panel_footer:not(.drop-unavailable) { border-color: #2875d9; background: #dcecff; box-shadow: inset 0 0 0 1px #2875d9; }
.drop-active {
	outline: 3px solid #4285f4;
	outline-offset: -3px;
	background: #e7f1fb;
}
.panel_menu {
	/* The menu's measured content sets its minimum row height. */
	overflow: visible;
	container: sd-menu-panel / inline-size;
	background: #fff;
	border-radius: 7px;
	/* Draw the frame inside without changing the measured menu height. */
	box-shadow: inset 0 0 0 1px #e2e7ee;
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
.folder-selector-placeholder { flex: 1; min-width: 0; max-width: 240px; height: 24px; }
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
	scrollbar-width: thin;
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
	border: 0;
	padding: 0;
	background: transparent;
	color: inherit;
	font: inherit;
	text-align: left;
	cursor: pointer;
	min-width: 0;
	overflow: hidden;
	text-overflow: ellipsis;
	white-space: nowrap;
}
.folder-loading { margin-left: 6px; color: #64748b; font-size: 10px; white-space: nowrap; }
.path-name.folder-has-matches { font-weight: 700; }
.folder-match-dot { color: #2879ce; font-size: 10px; }
.path-name:focus-visible { outline: 2px solid #4285d4; }
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
	grid-template-rows: auto auto minmax(0, 1fr);
	overflow: hidden;
	line-height: 18px;
	border: 1px solid #e2e7ee;
	border-radius: 5px;
	background: #fff;
	color: #273449;
}
.text-header {
	justify-content: space-between;
	padding: 3px 7px;
	background: #edf3fb;
	border-bottom-color: #dce5f0;
}
.text-header h2 { font-size: 12px; font-weight: 700; }
.map-summary { padding: 2px 7px; border-bottom: 1px solid #edf0f5; }
.map-summary-row { display: flex; align-items: center; justify-content: space-between; gap: 4px; }
.map-heading { display: inline-flex; align-items: center; gap: 7px; }
.map-count { padding: 0 6px; border-radius: 9px; background: #e5efff; color: #255b9f; font-size: 10px; }
.map-details-toggle { display: inline-flex; align-items: center; gap: 3px; padding: 1px 2px; border: 0; border-radius: 3px; background: transparent; color: #3267aa; font: inherit; cursor: pointer; }
.map-details-toggle svg { width: 12px; height: 12px; fill: none; stroke: currentColor; stroke-width: 1.5; }
.map-details-toggle .details-open { transform: rotate(180deg); }
.map-details-toggle:hover { background: #edf4fe; }
.map-details-toggle:focus-visible { outline: 2px solid #4285d4; outline-offset: -1px; }
.map-address { color: #64748b; font-variant-numeric: tabular-nums; overflow-wrap: anywhere; }
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
	scrollbar-width: thin;
}
.panel_text .path-message {
	display: flex;
	align-items: baseline;
	gap: 6px;
	margin: 0;
	padding: 1px 7px;
	border-bottom: 1px solid #f0f3f7;
}
.message-time {
	flex: 0 0 32px;
	color: #788496;
	font-variant-numeric: tabular-nums;
}
.message-body { min-width: 0; }
.message-warning-icon { width: 12px; height: 12px; margin-right: 4px; vertical-align: -1px; color: #c2840a; }
.panel_text .path-warning { background: #fff5df; color: #965b0a; }
.panel_text .path-warning .message-time { color: #965b0a; }
.path-error {
	color: #a12323;
}
.path-warning {
	color: #c00;
}
.resizer {
	background: #dce2eb;
	cursor: row-resize;
	touch-action: none;
	user-select: none;
}
#resizer2,
.resizer-horizontal {
	position: relative;
	z-index: 1;
}
#resizer2::after {
	content: '';
	position: absolute;
	inset: -3px 0;
}
#resizer2:hover,
#resizer2:active,
.resizer-horizontal:hover,
.resizer-horizontal:active {
	background: #4285d4;
}
.resizer-horizontal {
	cursor: col-resize;
}
.resizer-horizontal::after {
	content: '';
	position: absolute;
	inset: 0 -3px;
}
</style>
