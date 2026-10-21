<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import type { UnlistenFn } from '@tauri-apps/api/event';
import SdMenu from '../components/SdMenu.vue';
import { splitFolderPath } from '../utils/executablePath';
import { parseSdDirectCsv, type AddressRange } from '../utils/sdDirectCsv';

const menuHeight = ref(48);
const minimumMenuHeight = ref(48);
const menuContent = ref<HTMLElement | null>(null);
let menuObserver: ResizeObserver | undefined;
const sdHeight = ref(120);
const sourceDirectory = ref('');
const dragging = ref(false);
const initializing = ref(false);
let unlistenDrop: UnlistenFn | undefined;
let generation = 0;
let ready = false;
const useInputDirectory = ref(false);
const displayedDirectory = computed(() => useInputDirectory.value
	? `${sourceDirectory.value.replace(/[\\/]+$/, '')}\\input`
	: sourceDirectory.value);
const pathError = ref('');
const messages = ref<string[]>([]);
const sdDirectDictionary = ref(new Map<string, AddressRange>());
const sdAddressRange = ref<AddressRange | null>(null);
const matchedFiles = ref<string[]>([]);
const canSdWrite = computed(() => !initializing.value && matchedFiles.value.some(filename => sdDirectDictionary.value.has(filename)));
let matchingGeneration: number | undefined;
let disposed = false;
let lastMatchError = '';
const textPanel = ref<HTMLElement | null>(null);
const pathList = ref<HTMLElement | null>(null);
const folders = computed(() => splitFolderPath(displayedDirectory.value));

onMounted(async () => {
	window.addEventListener('keydown', onRefreshKey, true);
	if (typeof ResizeObserver !== 'undefined' && menuContent.value) {
		menuObserver = new ResizeObserver(([entry]) => {
			minimumMenuHeight.value = Math.max(48, Math.ceil(entry.contentRect.height));
		});
		menuObserver.observe(menuContent.value);
	}
	try {
		const unlisten = await getCurrentWebview().onDragDropEvent(event => {
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
	if (event.repeat || initializing.value || disposed) return;
	void initializeSource(undefined, sourceDirectory.value || undefined);
}

async function initializeSource(paths?: string[], refreshDirectory?: string) {
	const current = ++generation;
	const active = () => !disposed && current === generation;
	initializing.value = true;
	ready = false;
	sdDirectDictionary.value = new Map();
	sdAddressRange.value = null;
	matchedFiles.value = [];
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
			const csv = await invoke<string>('read_startup_csv', { directory });
			if (!active()) return;
			const parsed = parseSdDirectCsv(csv);
			sdDirectDictionary.value = parsed.dictionary;
			sdAddressRange.value = parsed.addressRange;
			await appendMessage('sdDirectaxs.csv読込完了', `dictionary作成：${sdDirectDictionary.value.size}件`);
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
	disposed = true;
	generation++;
	unlistenDrop?.();
	menuObserver?.disconnect();
	window.removeEventListener('keydown', onRefreshKey, true);
});

async function refreshMatchedFiles(report = false) {
	const current = generation;
	if (matchingGeneration === current || disposed || !ready) return;
	matchingGeneration = current;
	try {
		const result = await invoke<{ files: string[]; from_input: boolean; input_exists: boolean }>('find_dictionary_files', {
			filenames: [...sdDirectDictionary.value.keys()],
			directory: sourceDirectory.value,
		});
		if (disposed || current !== generation) return;
		const changed = useInputDirectory.value !== result.from_input || JSON.stringify(matchedFiles.value) !== JSON.stringify(result.files);
		matchedFiles.value = result.files;
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
			await appendMessage('inputフォルダへ移動', ...result.files);
		} else {
			await appendMessage(...(result.files.length ? result.files : ['一致するファイルはありません']));
		}
	} catch (error) {
		if (disposed || current !== generation) return;
		matchedFiles.value = [];
		const message = `ファイル照合失敗: ${String(error)}`;
		if (message !== lastMatchError) await appendMessage(message);
		lastMatchError = message;
	} finally {
		if (matchingGeneration === current) matchingGeneration = undefined;
	}
}

async function appendMessage(...newMessages: string[]) {
	for (const message of newMessages) messages.value.push(message);
	await nextTick();
	if (textPanel.value) textPanel.value.scrollTop = textPanel.value.scrollHeight;
}

async function onSdWrite(drive: string) {
	await appendMessage(drive ? `SD書込先: ${drive}` : 'SDドライブを選択してください');
	if (drive) await appendMessage('SD書込処理は未接続です');
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
			<div class="panel-base" :style="{ gridTemplateRows: `${Math.max(menuHeight, minimumMenuHeight)}px 1px ${sdHeight}px 1px minmax(0, 1fr) 28px` }">
				<div id="panel_menu" class="panel_menu">
					<div ref="menuContent" class="menu-content">
						<SdMenu :can-write="canSdWrite" :busy="initializing" @write="onSdWrite" @error="appendMessage" />
					</div>
				</div>
				<div id="resizer1" class="resizer" @pointerdown="resize($event, 1)"></div>
				<section id="panel_sd" class="panel_sd" aria-label="表示中のフォルダー">
					<div class="path-header">
						<span role="status">ファイル数：{{ matchedFiles.length }}件</span>
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
						<span role="status">SD：{{ sdAddressRange ? `${sdAddressRange.startAddress} - ${sdAddressRange.endAddress}` : '未取得' }}</span>
						<button type="button" :disabled="messages.length === 0" @click="messages = []" aria-label="操作メッセージをクリア">クリア</button>
					</div>
					<div ref="textPanel" class="text-messages" role="log" aria-label="操作メッセージ">
						<p v-for="(message, index) in messages" :key="index" class="path-message">{{ message }}</p>
					</div>
				</div>
				<div id="panel_footer" class="panel_footer" role="status">{{ dragging ? 'ドロップして対象フォルダーを変更' : initializing ? '対象フォルダーを読込中…' : 'ファイル・フォルダーをドロップできます' }}</div>
			</div>
		</div>
	</div>
</template>

<style scoped>
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
.resizer {
	background: #666;
	cursor: row-resize;
	touch-action: none;
	user-select: none;
}
</style>
