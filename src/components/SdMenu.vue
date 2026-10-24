<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';

const props = defineProps<{ canWrite: boolean; busy?: boolean; writeLabel?: string }>();
const emit = defineEmits<{ write: [drive: string]; error: [message: string] }>();
const actionsOpen = ref(false);
const actionsContainer = ref<HTMLElement | null>(null);
const actionsButton = ref<HTMLButtonElement | null>(null);

function closeActions(event: Event) {
	if (!actionsOpen.value) return;
	if (!actionsContainer.value?.contains(event.target as Node)) actionsOpen.value = false;
}

function onActionsFocusout(event: FocusEvent) {
	if (!actionsContainer.value?.contains(event.relatedTarget as Node | null)) actionsOpen.value = false;
}

interface DriveInfo { drive: string; type: string; size: number }
const drives = ref<DriveInfo[]>([]);
const selectedDrive = ref('');
const selectedDriveHasMedia = computed(() => drives.value.some(drive => drive.drive === selectedDrive.value && drive.size > 0));
const canRequestWrite = computed(() => props.canWrite && selectedDriveHasMedia.value && !props.busy);
const writeTitle = computed(() => {
	if (props.busy) return 'SDカードの処理中です';
	if (!props.canWrite) return '全エリアと一致するファイルがありません';
	if (!selectedDrive.value) return 'SDドライブを選択してください';
	if (!selectedDriveHasMedia.value) return '選択したドライブにSDカードが挿入されていません';
	return '全エリア（E以外）と一致するファイルをSDカードへ書込します';
});
const backendStatus = ref<'checking' | 'running' | 'stopped'>('checking');
const adminStatus = ref<'idle' | 'checking' | 'ready' | 'error'>('idle');
const adminTitle = computed(() => ({ idle: '管理者権限：未取得', checking: '管理者権限：確認中', ready: '管理者権限：取得済み（SD操作用プロセス）', error: '管理者権限：取得・通信失敗' })[adminStatus.value]);
let adminTimer: ReturnType<typeof setInterval> | undefined;
let checkingAdmin = false;
let timer: ReturnType<typeof setInterval> | undefined;
let checking = false;
let disposed = false;

function formatCapacity(size: number): string {
	let exponent = 20;
	for (let i = 21; i <= 42; i++) {
		if (Math.abs(size - 2 ** i) < Math.abs(size - 2 ** exponent)) exponent = i;
	}
	const unitExponent = exponent >= 40 ? 40 : exponent >= 30 ? 30 : 20;
	return `${2 ** (exponent - unitExponent)}${unitExponent === 40 ? 'TB' : unitExponent === 30 ? 'GB' : 'MB'}`;
}

function selectDrive() {
	try { localStorage.setItem('sddirecta-selected-drive', selectedDrive.value); } catch { /* Selection still works without storage. */ }
}

async function refreshDrives() {
	if (checking || props.busy) return;
	checking = true;
	try {
		const result = await invoke<DriveInfo[]>('get_drives');
		if (disposed || props.busy) return;
		drives.value = result.filter(drive => drive.type === 'Removable');
		backendStatus.value = 'running';
		if (!drives.value.some(drive => drive.drive === selectedDrive.value)) {
			let savedDrive = '';
			try { savedDrive = localStorage.getItem('sddirecta-selected-drive') ?? ''; } catch { /* Optional preference. */ }
			selectedDrive.value = drives.value.find(drive => drive.drive === savedDrive)?.drive ?? drives.value[0]?.drive ?? '';
		}
	} catch (error) {
		if (disposed || props.busy) return;
		drives.value = [];
		selectedDrive.value = '';
		if (backendStatus.value !== 'stopped') emit('error', `ドライブ情報取得失敗: ${String(error)}`);
		backendStatus.value = 'stopped';
	} finally {
		checking = false;
	}
}

async function refreshAdmin() {
	if (checkingAdmin) return;
	checkingAdmin = true;
	try {
		const status = await invoke<'idle' | 'checking' | 'ready' | 'error'>('sd_admin_status');
		if (!disposed) adminStatus.value = status;
	} catch {
		if (!disposed) adminStatus.value = 'error';
	} finally { checkingAdmin = false; }
}

function requestWrite() {
	if (canRequestWrite.value) emit('write', selectedDrive.value);
}

function onKeydown(event: KeyboardEvent) {
	if (event.key === 'Escape' && actionsOpen.value) {
		actionsOpen.value = false;
		actionsButton.value?.focus();
	}
	if (event.key === 'F5') {
		event.preventDefault();
		void refreshDrives();
	}
}

onMounted(() => {
	void refreshDrives();
	void refreshAdmin();
	adminTimer = setInterval(() => void refreshAdmin(), 500);
	timer = setInterval(() => void refreshDrives(), 2000);
	window.addEventListener('keydown', onKeydown);
	window.addEventListener('pointerdown', closeActions);
});
onUnmounted(() => {
	disposed = true;
	clearInterval(timer);
	clearInterval(adminTimer);
	window.removeEventListener('keydown', onKeydown);
	window.removeEventListener('pointerdown', closeActions);
});
</script>

<template>
	<div class="sd-menu" aria-label="SDカード操作">
		<div class="sd-write-wrapper">
			<button type="button" class="sd-write-button" :disabled="!canRequestWrite" @click="requestWrite"
				:title="writeTitle">{{ writeLabel || 'SD書込' }}</button>
			<span class="backend-indicator" :class="`admin-indicator--${adminStatus}`" :title="adminTitle" role="img" :aria-label="adminTitle"></span>
			<span class="admin-label" :title="adminTitle">管理者権限</span>
		</div>
		<div class="drive-select-inline" role="radiogroup" aria-label="書込先ドライブ">
			<div v-if="!drives.length" class="no-drive-warning-inline" role="status">
				<svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true"><path fill="currentColor" d="M1 21h22L12 2 1 21zm12-3h-2v-2h2v2zm0-4h-2v-4h2v4z" /></svg>
				<span>{{ backendStatus === 'checking' ? '確認中…' : backendStatus === 'stopped' ? '取得失敗' : 'SDドライブなし' }}</span>
			</div>
			<label v-for="drive in drives" :key="drive.drive" class="drive-item-inline" :class="{ 'drive-disabled': busy }">
				<span class="radio-label-inline">
					<input v-model="selectedDrive" type="radio" :value="drive.drive" name="drive-select" :disabled="busy" @change="selectDrive" :aria-label="`ドライブ ${drive.drive}`" />
					<span class="drive-letter">{{ drive.drive }}</span>
				</span>
				<span class="sd-icon-wrapper-inline" role="img" :aria-label="`${drive.drive} ${busy ? '処理中' : drive.size > 0 ? formatCapacity(drive.size) : 'メディア未挿入'}`">
					<span class="sd-card-icon-inline" :class="busy ? 'status-busy' : drive.size > 0 ? 'status-inserted' : 'status-empty'" :title="busy ? '処理中' : drive.size > 0 ? formatCapacity(drive.size) : 'メディア未挿入'">
						<svg viewBox="0 0 24 24" width="32" height="32" aria-hidden="true"><path fill="currentColor" d="M18 2h-8L4 8v12c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V4c0-1.1-.9-2-2-2zm-6 6h-2V4h2v4zm3 0h-2V4h2v4zm3 0h-2V4h2v4z" /></svg>
					</span>
					<span v-if="drive.size > 0" class="sd-capacity-display-inline">{{ formatCapacity(drive.size) }}</span>
				</span>
			</label>
		</div>
		<div ref="actionsContainer" class="menu-actions" @focusout="onActionsFocusout">
			<button ref="actionsButton" type="button" class="menu-toggle" title="メニュー" aria-label="メニュー" :disabled="busy"
				:aria-expanded="actionsOpen" aria-controls="sd-actions" @click="actionsOpen = !actionsOpen">
				<svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" aria-hidden="true"><path d="M4 6h16M4 12h16M4 18h16" /></svg>
			</button>
			<div v-if="actionsOpen" id="sd-actions" class="menu-dropdown" role="group" aria-label="メニュー操作">
				<button type="button">全エリアSD読み込み</button>
				<button type="button">選択エリアSD読み込み</button>
				<button type="button">全エリアSD書き込み</button>
				<button type="button">選択エリアSD書き込み</button>
				<button type="button">全エリアSD FF初期化</button>
				<button type="button">SD選択エリア FF初期化</button>
			</div>
		</div>
	</div>
</template>

<style scoped>
.sd-menu { display: grid; grid-template-columns: auto minmax(0, 1fr) auto; align-items: center; gap: 6px; width: 100%; min-height: 48px; padding: 5px 6px; font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', 'Helvetica Neue', Arial, 'Hiragino Sans', 'Hiragino Kaku Gothic ProN', Meiryo, sans-serif; font-size: 14px; line-height: 1.5; color: #333; }
.menu-actions { position: relative; }
.menu-toggle { display: flex; align-items: center; justify-content: center; width: 32px; height: 34px; padding: 0; border: 1px solid transparent; border-radius: 6px; background: transparent; color: #526171; cursor: pointer; }
.menu-toggle:hover, .menu-toggle[aria-expanded="true"] { background: #e8f0fe; border-color: #c6d8f4; color: #245da6; }
.menu-toggle:focus-visible, .menu-dropdown button:focus-visible { outline: 2px solid #174a75; outline-offset: 1px; }
.menu-dropdown { position: absolute; top: calc(100% + 5px); right: 0; z-index: 10; width: 212px; max-width: calc(100vw - 20px); padding: 4px; border: 1px solid #d9dfe5; border-radius: 8px; background: #fff; box-shadow: 0 5px 18px #22334426; }
.menu-dropdown button { display: flex; align-items: center; justify-content: space-between; gap: 12px; width: 100%; padding: 8px; border: 0; border-radius: 4px; background: transparent; color: #333; font: inherit; font-size: 12px; text-align: left; cursor: pointer; }
.menu-dropdown button:hover:not(:disabled) { background: #e8f0fe; }
.menu-dropdown button:disabled { color: #9ca3af; cursor: default; }
.drive-select-inline { display: flex; flex: 1; min-width: 0; flex-direction: row; flex-wrap: wrap; align-items: center; gap: 0.4rem; }
.no-drive-warning-inline { display: flex; align-items: center; gap: 0.4rem; padding: 0.45rem 1rem; background: linear-gradient(135deg, #fff9e6 0%, #fff3cd 100%); border: 1.5px solid #ffc107; border-radius: 8px; color: #856404; font-size: 0.95rem; font-weight: 500; white-space: nowrap; }
.no-drive-warning-inline svg { flex-shrink: 0; }
.drive-item-inline { display: flex; flex-shrink: 0; align-items: center; gap: 0.3rem; padding: 0.15rem 0.5rem; border: 1.5px solid #e5e5e5; border-radius: 8px; background: white; cursor: pointer; transition: all 0.25s cubic-bezier(0.4, 0, 0.2, 1); }
.drive-item-inline:has(input:checked) { border-color: #4285f4; background: linear-gradient(135deg, #e8f0fe 0%, #f0f6ff 100%); box-shadow: 0 1px 4px rgba(66, 133, 244, 0.15); }
.drive-item-inline:not(:has(input:checked)):hover { border-color: #4285f4; background: #fafbfc; }
.radio-label-inline { display: flex; align-items: center; gap: 0.3rem; cursor: pointer; }
.radio-label-inline input { cursor: pointer; margin: 0; width: 16px; height: 16px; }
.drive-letter { font-size: 0.95rem; font-weight: 600; }
.sd-icon-wrapper-inline { position: relative; display: flex; align-items: center; justify-content: center; }
.sd-card-icon-inline { display: flex; align-items: center; justify-content: center; background: #fff; transition: all 0.3s cubic-bezier(0.4, 0, 0.2, 1); }
.sd-card-icon-inline:hover { transform: scale(1.1); }
.status-inserted { color: #4285f4; animation: sdCardInsert 0.6s cubic-bezier(0.34, 1.56, 0.64, 1); }
@keyframes sdCardInsert {
	0% { transform: translateY(8px) scale(0.9); opacity: 0.5; }
	50% { transform: translateY(-3px) scale(1.05); opacity: 1; }
	70% { transform: translateY(1px) scale(0.98); }
	100% { transform: translateY(0) scale(1); opacity: 1; }
}
.status-empty { color: #d5d5d5; }
.status-busy { background: #000; color: #fff; font-weight: bold; }
.sd-capacity-display-inline { position: absolute; bottom: 2px; left: 50%; transform: translateX(-50%); display: flex; align-items: center; justify-content: center; padding: 0.05rem 0.2rem; background: transparent; font-size: 0.6rem; font-weight: 800; color: #fff; min-width: 20px; text-align: center; letter-spacing: 0.02em; text-shadow: -1px -1px 0 #333, 1px -1px 0 #333, -1px 1px 0 #333, 1px 1px 0 #333, 0 0 3px rgba(0, 0, 0, 0.5); z-index: 1; white-space: nowrap; pointer-events: none; }
.sd-write-wrapper { position: relative; display: inline-flex; flex-direction: column; flex-shrink: 0; }
.admin-label { font-size: 10px; line-height: 12px; text-align: center; color: #526171; }
.drive-disabled { opacity: 0.5; cursor: default; }
.sd-write-button { padding: 0.45rem 1.1rem; background: linear-gradient(135deg, #4285f4 0%, #357ae8 100%); color: white; border: 2px solid transparent; border-radius: 8px; font-family: inherit; font-size: 0.95rem; font-weight: 600; cursor: pointer; transition: all 0.3s cubic-bezier(0.4, 0, 0.2, 1); outline: none; white-space: nowrap; box-shadow: 0 3px 8px rgba(66, 133, 244, 0.3), 0 1px 3px rgba(66, 133, 244, 0.2); position: relative; overflow: hidden; }
.sd-write-button::before { content: ''; position: absolute; top: 0; left: -100%; width: 100%; height: 100%; background: linear-gradient(90deg, transparent, rgba(255, 255, 255, 0.3), transparent); transition: left 0.5s; }
.sd-write-button:hover::before { left: 100%; }
.sd-write-button:hover { background: linear-gradient(135deg, #357ae8 0%, #2a67c8 100%); transform: translateY(-3px); box-shadow: 0 6px 16px rgba(66, 133, 244, 0.4), 0 2px 6px rgba(66, 133, 244, 0.3); }
.sd-write-button:active { transform: translateY(-1px); box-shadow: 0 3px 8px rgba(66, 133, 244, 0.35); }
.sd-write-button:focus-visible { outline: 2px solid #174a75; outline-offset: 2px; }
.sd-write-button:disabled, .sd-write-button:disabled:hover { background: #e0e0e0; border-color: transparent; color: #9e9e9e; cursor: not-allowed; opacity: 0.6; box-shadow: none; transform: none; }
.sd-write-button:disabled::before { display: none; }
.backend-indicator { position: absolute; top: -5px; right: -5px; width: 11px; height: 11px; border-radius: 50%; border: 2px solid #fff; z-index: 1; transition: background-color 0.4s ease, box-shadow 0.4s ease; }
.backend-indicator--checking { background-color: #cbd5e1; animation: backend-pulse 1.4s ease-in-out infinite; }
.backend-indicator--running { background-color: #22c55e; box-shadow: 0 0 0 2px rgba(34, 197, 94, 0.25); animation: backend-ripple 2s ease-out 3; }
.backend-indicator--stopped { background-color: #9ca3af; }
.admin-indicator--idle { background-color: #9ca3af; }
.admin-indicator--checking { background-color: #eab308; animation: backend-pulse 1.4s ease-in-out infinite; }
.admin-indicator--ready { background-color: #22c55e; box-shadow: 0 0 0 2px rgba(34, 197, 94, 0.25); }
.admin-indicator--error { background-color: #dc2626; }
@keyframes backend-pulse { 0%, 100% { opacity: 1; } 50% { opacity: 0.25; } }
@keyframes backend-ripple { 0% { box-shadow: 0 0 0 0 rgba(34, 197, 94, 0.5); } 60% { box-shadow: 0 0 0 5px rgba(34, 197, 94, 0); } 100% { box-shadow: 0 0 0 0 rgba(34, 197, 94, 0); } }
@container sd-menu-panel (max-width: 320px) {
	.sd-menu { gap: 0.3rem; padding-inline: 3px; }
	.drive-select-inline { gap: 0.3rem; }
	.drive-item-inline { gap: 3px; padding-inline: 4px; }
	.radio-label-inline { gap: 3px; }
	.drive-letter { font-size: 0.85rem; }
	.sd-card-icon-inline svg { width: 28px; height: 28px; }
	.sd-write-button { padding-inline: 0.65rem; }
	.no-drive-warning-inline { padding: 5px; gap: 3px; font-size: 12px; white-space: normal; }
}
@container sd-menu-panel (max-width: 200px) {
	.drive-select-inline { grid-column: 1 / -1; grid-row: 2; }
	.menu-actions { grid-column: 3; grid-row: 1; }
	.no-drive-warning-inline { padding-inline: 0.4rem; white-space: normal; }
}
</style>
