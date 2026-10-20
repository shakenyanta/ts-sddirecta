<script setup lang="ts">
import { computed, nextTick, onMounted, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { splitFolderPath } from '../utils/executablePath';
import { parseSdDirectCsv, type AddressRange } from '../utils/sdDirectCsv';

const menuHeight = ref(48);
const sdHeight = ref(120);
const executableDirectory = ref('');
const pathError = ref('');
const messages = ref<string[]>([]);
const sdDirectDictionary = ref(new Map<string, AddressRange>());
const textPanel = ref<HTMLElement | null>(null);
const pathList = ref<HTMLElement | null>(null);
const folders = computed(() => splitFolderPath(executableDirectory.value));

onMounted(async () => {
	try {
		executableDirectory.value = await invoke<string>('get_executable_directory');
		await nextTick();
		if (pathList.value) pathList.value.scrollTop = pathList.value.scrollHeight;
	} catch (error) {
		pathError.value = `保存先を取得できませんでした: ${String(error)}`;
	}
	let exists: boolean;
	try {
		exists = await invoke<boolean>('check_startup_csv');
	} catch (error) {
		await appendMessage(`sdDirectaxs.csv確認失敗: ${String(error)}`);
		return;
	}
	if (!exists) {
		await appendMessage('sdDirectaxs.csv無し');
		return;
	}
	await appendMessage('sdDirectaxs.csv読込完了');
	try {
		const csv = await invoke<string>('read_startup_csv');
		sdDirectDictionary.value = parseSdDirectCsv(csv);
		const lines = [...sdDirectDictionary.value].map(([filename, range]) =>
			`${filename}: ${range.startAddress}, ${range.endAddress}`);
		await appendMessage(`dictionary（${sdDirectDictionary.value.size}件）`, ...lines);
	} catch (error) {
		await appendMessage(`sdDirectaxs.csv読込失敗: ${String(error)}`);
	}
});

async function appendMessage(...newMessages: string[]) {
	for (const message of newMessages) messages.value.push(message);
	await nextTick();
	if (textPanel.value) textPanel.value.scrollTop = textPanel.value.scrollHeight;
}

async function copyDirectory() {
	try {
		await navigator.clipboard.writeText(executableDirectory.value);
	} catch {
		await appendMessage('コピーできませんでした');
		return;
	}
	await appendMessage('コピーしました');
}

function resize(event: PointerEvent, divider: 1 | 2) {
	const target = event.currentTarget as HTMLElement;
	const startY = event.clientY;
	const initialMenuHeight = menuHeight.value;
	const initialSdHeight = sdHeight.value;
	const textHeight = target.parentElement!.querySelector<HTMLElement>('.panel_text')!.offsetHeight;
	target.setPointerCapture(event.pointerId);

	const move = (moveEvent: PointerEvent) => {
		const delta = moveEvent.clientY - startY;
		if (divider === 1) {
			const change = Math.max(-initialMenuHeight, Math.min(delta, initialSdHeight));
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
	<div class="panel-outer">
		<div class="panel-inner">
			<div class="panel-base" :style="{ gridTemplateRows: `${menuHeight}px 1px ${sdHeight}px 1px minmax(0, 1fr) 28px` }">
				<div id="panel_menu" class="panel_menu"></div>
				<div id="resizer1" class="resizer" @pointerdown="resize($event, 1)"></div>
				<section id="panel_sd" class="panel_sd" aria-label="実行ファイルの保存先">
					<div class="path-header">
						<span>exeの保存先</span>
						<button type="button" :disabled="!executableDirectory" @click="copyDirectory">フルパスをコピー</button>
					</div>
					<p v-if="pathError" class="path-message path-error" role="alert">{{ pathError }}</p>
					<ol v-else-if="folders.length" ref="pathList" class="path-list" aria-label="保存先のフォルダー階層">
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
						<button type="button" :disabled="messages.length === 0" @click="messages = []" aria-label="操作メッセージをクリア">クリア</button>
					</div>
					<div ref="textPanel" class="text-messages" role="log" aria-label="操作メッセージ">
						<p v-for="(message, index) in messages" :key="index" class="path-message">{{ message }}</p>
					</div>
				</div>
				<div id="panel_footer" class="panel_footer"></div>
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
	justify-content: flex-end;
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
