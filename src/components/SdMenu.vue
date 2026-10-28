<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { getName, getVersion } from '@tauri-apps/api/app';

const props = defineProps<{ canWrite: boolean; canRead?: boolean; canInitialize?: boolean; busy?: boolean; writeLabel?: string }>();
const emit = defineEmits<{ write: [drive: string]; read: [drive: string]; initialize: [drive: string]; format: [drive: string]; action: [label: string]; error: [message: string] }>();
const actionsOpen = ref(false);
const actionsContainer = ref<HTMLElement | null>(null);
const actionsButton = ref<HTMLButtonElement | null>(null);
const infoDialog = ref<HTMLDialogElement | null>(null);
const infoTitle = ref('');
const appName = ref('');
const appVersion = ref('');
const infoMessage = ref('');
interface UpdateCheck { status: 'available' | 'latest' | 'unpublished'; current_version: string; version: string | null; notes: string }
const availableUpdate = ref<UpdateCheck | null>(null);
const installingUpdate = ref(false);
let infoRequest = 0;

async function showAppInfo(title: 'アップデート確認' | 'バージョン情報') {
    if (installingUpdate.value || (title === 'アップデート確認' && props.busy)) return;
    recordAction(title);
    actionsOpen.value = false;
    infoTitle.value = title;
    appName.value = '';
    appVersion.value = '';
    availableUpdate.value = null;
    infoMessage.value = 'バージョン情報を取得しています…';
    const request = ++infoRequest;
    await nextTick();
    infoDialog.value?.showModal();
    try {
        const [name, version] = await Promise.all([getName(), getVersion()]);
        if (disposed || request !== infoRequest) return;
        appName.value = name;
        appVersion.value = version;
        infoMessage.value = '';
        if (title === 'アップデート確認') {
            infoMessage.value = 'アップデートを確認しています…';
            const result = await invoke<UpdateCheck>('check_portable_update');
            if (disposed || request !== infoRequest) return;
            if (result.status === 'available') {
                availableUpdate.value = result;
                infoMessage.value = `バージョン ${result.version} のアップデートがあります。実行しますか？`;
            } else {
                infoMessage.value = result.status === 'latest' ? '最新バージョンです。' : '更新情報はまだ公開されていません。';
            }
        }
    } catch (error) {
        if (!disposed && request === infoRequest) infoMessage.value = `${title}に失敗しました: ${String(error)}`;
    }
}

async function installUpdate() {
    const version = availableUpdate.value?.version;
    if (!version || installingUpdate.value || props.busy) return;
    recordAction('アップデート実行');
    installingUpdate.value = true;
    infoMessage.value = '更新ファイルをダウンロード・検証しています。完了後、SD処理用プロセスを終了し、アプリを更新して再起動します。';
    try {
        await invoke('install_portable_update', { version });
        infoMessage.value = '更新の準備が完了しました。アプリを再起動しています…';
    } catch (error) {
        availableUpdate.value = null;
        infoMessage.value = `更新を実行できませんでした: ${String(error)}`;
        installingUpdate.value = false;
    }
}

function closeAppInfo() {
    if (installingUpdate.value) return;
    ++infoRequest;
    infoDialog.value?.close();
    actionsButton.value?.focus();
}

function closeActions(event: Event) {
	if (!actionsOpen.value) return;
	if (!actionsContainer.value?.contains(event.target as Node)) actionsOpen.value = false;
}

function onActionsFocusout(event: FocusEvent) {
	if (!actionsContainer.value?.contains(event.relatedTarget as Node | null)) actionsOpen.value = false;
}

interface DriveInfo { drive: string; type: string; size: number; label?: string }
const drives = ref<DriveInfo[]>([]);
const selectedDrive = ref('');
const selectedDriveHasMedia = computed(() => drives.value.some(drive => drive.drive === selectedDrive.value && drive.size > 0));
const selectedDriveHasLetter = computed(() => /^[A-Z]:$/.test(selectedDrive.value));
const canRequestWrite = computed(() => props.canWrite && selectedDriveHasMedia.value && selectedDriveHasLetter.value && !props.busy);
const canRequestRead = computed(() => props.canRead && selectedDriveHasMedia.value && selectedDriveHasLetter.value && !props.busy);
const canRequestInitialize = computed(() => props.canInitialize && selectedDriveHasMedia.value && selectedDriveHasLetter.value && !props.busy);
const initializeTitle = computed(() => props.busy ? 'SDカードの処理中です'
    : selectedDriveHasMedia.value && !selectedDriveHasLetter.value ? '未割り当てのSDカードは「SDフォーマット」から使用可能な状態にできます'
    : !props.canInitialize ? 'SD MAPに初期化対象の領域がありません'
    : !selectedDriveHasMedia.value ? 'SDカードが挿入されたドライブを選択してください'
    : 'SD MAPの指定領域を0xFFで初期化します。フォーマット情報も失われます。');
const readTitle = computed(() => props.busy ? 'SDカードの処理中です'
    : selectedDriveHasMedia.value && !selectedDriveHasLetter.value ? '未割り当てのSDカードは「SDフォーマット」から使用可能な状態にできます'
    : !props.canRead ? 'SD MAPに読込対象の領域がありません'
    : !selectedDriveHasMedia.value ? 'SDカードが挿入されたドライブを選択してください'
    : 'SD MAPの指定領域を読み込み、outputフォルダーに保存します。');
const writeTitle = computed(() => {
	if (props.busy) return 'SDカードの処理中です';
	if (selectedDriveHasMedia.value && !selectedDriveHasLetter.value) return '未割り当てのSDカードは「SDフォーマット」から使用可能な状態にできます';
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

async function refreshDrives(preferredDrive?: string) {
	if (checking || props.busy) return;
	checking = true;
	try {
		const result = await invoke<DriveInfo[]>('get_drives');
		if (disposed || props.busy) return;
		drives.value = result.filter(drive => drive.type === 'Removable');
		if (preferredDrive && drives.value.some(drive => drive.drive === preferredDrive)) selectedDrive.value = preferredDrive;
		backendStatus.value = 'running';
		if (!drives.value.some(drive => drive.drive === selectedDrive.value)) {
			let savedDrive = '';
			try { savedDrive = localStorage.getItem('sddirecta-selected-drive') ?? ''; } catch { /* Optional preference. */ }
			selectedDrive.value = drives.value.find(drive => drive.drive === savedDrive)?.drive ?? drives.value.find(drive => drive.size > 0)?.drive ?? drives.value[0]?.drive ?? '';
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
	if (!canRequestWrite.value) return;
    emit('write', selectedDrive.value);
}

function recordAction(label: string) {
    emit('action', label);
}

function requestRead() {
    if (!canRequestRead.value) return;
    recordAction('全エリアSD読込');
    actionsOpen.value = false;
    emit('read', selectedDrive.value);
}

function requestInitialize() {
    if (!canRequestInitialize.value) return;
    recordAction('全エリアSD FF初期化');
    actionsOpen.value = false;
    emit('initialize', selectedDrive.value);
}

function requestFormat() {
    if (!selectedDriveHasMedia.value || props.busy || installingUpdate.value) return;
    recordAction('SDフォーマット');
    actionsOpen.value = false;
    emit('format', selectedDrive.value);
}

defineExpose({ refreshDrives });

async function openDiagnosticLog() {
    recordAction('ログ表示');
	actionsOpen.value = false;
	try {
		await invoke('open_diagnostic_log');
	} catch (error) {
		emit('error', `ログを開けませんでした: ${String(error)}`);
	}
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
		</div>
		<div class="drive-select-inline" role="radiogroup" aria-label="書込先ドライブ">
			<div v-if="!drives.length" class="no-drive-warning-inline" role="status">
				<svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true"><path fill="currentColor" d="M1 21h22L12 2 1 21zm12-3h-2v-2h2v2zm0-4h-2v-4h2v4z" /></svg>
				<span>{{ backendStatus === 'checking' ? '確認中…' : backendStatus === 'stopped' ? '取得失敗' : 'SDドライブなし' }}</span>
			</div>
			<label v-for="drive in drives" :key="drive.drive" class="drive-item-inline" :class="{ 'drive-disabled': busy }">
				<span class="radio-label-inline">
					<input v-model="selectedDrive" type="radio" :value="drive.drive" name="drive-select" :disabled="busy" @change="selectDrive" :aria-label="`ドライブ ${drive.label || drive.drive}`" />
					<span class="drive-letter">{{ drive.drive.startsWith('PhysicalDrive') ? '?:' : drive.label || drive.drive }}</span>
				</span>
				<span class="sd-icon-wrapper-inline" role="img" :aria-label="`${drive.drive} ${busy ? '処理中' : drive.size > 0 ? formatCapacity(drive.size) : 'メディア未挿入'}`">
					<span class="sd-card-icon-inline" :class="busy ? 'status-busy' : drive.size > 0 ? 'status-inserted' : 'status-empty'" :title="busy ? '処理中' : drive.size > 0 ? formatCapacity(drive.size) : 'メディア未挿入'">
						<svg viewBox="0 0 24 24" width="32" height="32" aria-hidden="true"><path fill="currentColor" d="M18 2h-8L4 8v12c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V4c0-1.1-.9-2-2-2zm-6 6h-2V4h2v4zm3 0h-2V4h2v4zm3 0h-2V4h2v4z" /></svg>
					</span>
					<span v-if="drive.size > 0" class="sd-capacity-display-inline"><span>{{ formatCapacity(drive.size) }}</span></span>
				</span>
			</label>
		</div>
		<div ref="actionsContainer" class="menu-actions" @focusout="onActionsFocusout">
			<button ref="actionsButton" type="button" class="menu-toggle" title="メニュー" aria-label="メニュー"
				:aria-expanded="actionsOpen" aria-controls="sd-actions" @click="actionsOpen = !actionsOpen">
				<svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" aria-hidden="true"><path d="M4 6h16M4 12h16M4 18h16" /></svg>
			</button>
			<div v-if="actionsOpen" id="sd-actions" class="menu-dropdown" role="group" aria-label="メニュー操作">
				<div class="menu-operation-buttons" role="group" aria-label="SD操作">
					<button type="button" :disabled="!canRequestRead" :title="readTitle" @click="requestRead">全エリアSD読込</button>
					<button type="button" :disabled="!canRequestInitialize" :title="initializeTitle" @click="requestInitialize">全エリアSD FF初期化</button>
					<button type="button" class="menu-format-button" :disabled="!selectedDriveHasMedia || busy || installingUpdate" title="選択したSDカードをフォーマットします" @click="requestFormat">SDフォーマット</button>
				</div>
				<div class="menu-log-section">
					<button type="button" @click="openDiagnosticLog">ログ表示</button>
				</div>
				<div class="menu-info-section">
					<button type="button" :disabled="busy || installingUpdate" @click="showAppInfo('アップデート確認')">アップデート確認</button>
					<button type="button" @click="showAppInfo('バージョン情報')">バージョン情報</button>
				</div>
			</div>
		</div>
		<dialog ref="infoDialog" class="app-info-dialog" aria-labelledby="app-info-title" @cancel.prevent="closeAppInfo">
			<div class="app-info-content">
				<header class="app-info-header">
					<h2 id="app-info-title">{{ infoTitle }}</h2>
					<span v-if="availableUpdate" class="update-badge">更新あり</span>
				</header>
				<div class="app-info-body">
					<p v-if="appName" class="app-info-name">{{ appName }}</p>
					<div v-if="appVersion" class="app-info-versions" aria-label="バージョン">
						<span><small>現在</small><strong>{{ appVersion }}</strong></span>
						<template v-if="availableUpdate?.version">
							<span class="version-arrow" aria-hidden="true">→</span>
							<span class="new-version"><small>更新後</small><strong>{{ availableUpdate.version }}</strong></span>
						</template>
					</div>
					<p v-if="infoMessage" class="app-info-message" role="status">{{ infoMessage }}</p>
					<section v-if="availableUpdate?.notes" class="update-notes" aria-labelledby="update-notes-title">
						<h3 id="update-notes-title">更新内容</h3>
						<p>{{ availableUpdate.notes }}</p>
					</section>
				</div>
				<form method="dialog" @submit.prevent="closeAppInfo">
					<button type="submit" :disabled="installingUpdate" :class="{ 'app-info-primary': !availableUpdate }" autofocus>{{ availableUpdate ? 'あとで' : '閉じる' }}</button>
					<button v-if="availableUpdate" type="button" class="app-info-primary" :disabled="busy || installingUpdate" @click="installUpdate">{{ installingUpdate ? '更新中…' : '更新する' }}</button>
				</form>
			</div>
		</dialog>
	</div>
</template>

<style scoped>
.app-info-dialog { position: fixed; inset: 0; margin: auto; width: min(380px, calc(100vw - 8px)); max-width: none; max-height: calc(100dvh - 8px); padding: 0; border: 0; border-radius: 18px; background: #f8f9fe; color: #121212; box-shadow: 0 12px 32px #0003; font: 12px/1.5 'Yu Gothic UI', Meiryo, sans-serif; overflow: hidden; overflow-wrap: anywhere; }
.app-info-dialog::backdrop { background: #0005; }
.app-info-content { display: flex; flex-direction: column; max-height: calc(100dvh - 8px); padding: 12px; gap: 10px; }
.app-info-header { display: flex; flex-shrink: 0; align-items: center; justify-content: space-between; gap: 8px; padding-bottom: 8px; border-bottom: 3px solid #1a73df; }
.app-info-dialog h2 { margin: 0; font-size: 16px; line-height: 1.3; font-weight: 700; }
.update-badge { flex-shrink: 0; padding: 2px 7px; border-radius: 20px; background: #e5efff; color: #155bb5; font-size: 10px; font-weight: 700; }
.app-info-body { min-height: 0; overflow: auto; scrollbar-width: thin; }
.app-info-name { margin: 0 0 6px; color: #536176; font-size: 11px; }
.app-info-versions { display: flex; flex-wrap: wrap; align-items: center; gap: 10px; margin-bottom: 10px; }
.app-info-versions > span:not(.version-arrow) { display: flex; align-items: baseline; gap: 6px; }
.app-info-versions small { color: #536176; font-size: 10px; }
.app-info-versions strong { font-size: 18px; font-weight: 600; font-variant-numeric: tabular-nums; }
.version-arrow { color: #8391a3; }
.new-version strong { color: #155bb5; }
.app-info-message { margin: 0; }
.update-notes { margin-top: 10px; padding: 8px 10px; border: 1px solid #dce5f5; border-radius: 8px; background: #eef2f8; }
.update-notes h3 { margin: 0 0 4px; color: #536176; font-size: 10px; font-weight: 600; }
.update-notes p { margin: 0; white-space: pre-wrap; }
.app-info-dialog form { display: flex; flex-shrink: 0; justify-content: flex-end; gap: 6px; }
.app-info-dialog button { padding: 5px 12px; border: 1px solid #c5cad3; border-radius: 20px; background: #e9ecf2; color: #242424; cursor: pointer; font: inherit; line-height: 1.2; }
.app-info-dialog button.app-info-primary { border-color: #1a73df; background: #1a73df; color: #fff; font-weight: 700; }
.app-info-dialog button:hover:not(:disabled) { background: #dce3ef; }
.app-info-dialog button.app-info-primary:hover:not(:disabled) { border-color: #155bb5; background: #155bb5; }
.app-info-dialog button:focus-visible { outline: 2px solid #1a73df; outline-offset: 2px; }
.app-info-dialog button:disabled { opacity: .45; cursor: default; }
@media (max-width: 400px), (max-height: 400px) {
    .app-info-dialog { border-radius: 14px; }
    .app-info-content { padding: 8px; gap: 8px; }
    .app-info-dialog h2 { font-size: 14px; }
    .app-info-dialog button { padding: 4px 10px; font-size: 11px; }
}
.sd-menu { display: grid; grid-template-columns: auto minmax(0, 1fr) auto; align-items: center; gap: 6px; width: 100%; min-height: 48px; padding: 5px 6px; font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', 'Helvetica Neue', Arial, 'Hiragino Sans', 'Hiragino Kaku Gothic ProN', Meiryo, sans-serif; font-size: 14px; line-height: 1.5; color: #333; }
.menu-actions { position: relative; }
.menu-toggle { display: flex; align-items: center; justify-content: center; width: 32px; height: 34px; padding: 0; border: 1px solid transparent; border-radius: 6px; background: transparent; color: #526171; cursor: pointer; }
.menu-toggle:hover, .menu-toggle[aria-expanded="true"] { background: #e8f0fe; border-color: #c6d8f4; color: #245da6; }
.menu-toggle:focus-visible, .menu-dropdown button:focus-visible { outline: 2px solid #174a75; outline-offset: 1px; }
.menu-dropdown { position: absolute; top: calc(100% + 5px); right: 0; z-index: 10; width: 180px; max-width: calc(100vw - 20px); max-height: calc(100dvh - 80px); overflow-y: auto; padding: 6px; border: 1px solid #dce2eb; border-radius: 10px; background: #f8f9fe; color: #293445; box-shadow: 0 6px 20px #2233441f, 0 1px 3px #2233440d; font: 12px/16px 'Yu Gothic UI', Meiryo, sans-serif; scrollbar-width: thin; }
.menu-dropdown button { display: flex; align-items: center; width: 100%; min-height: 28px; padding: 6px 12px; border: 0; border-radius: 5px; background: transparent; color: inherit; font: inherit; font-weight: 400; text-align: left; cursor: pointer; transition: background-color .12s, color .12s; }
.menu-dropdown button:hover:not(:disabled), .menu-dropdown button:focus-visible { background: #e8f0fe; color: #155bb5; }
.menu-dropdown button:focus-visible { outline: 2px solid #1a73df; outline-offset: -2px; }
.menu-dropdown button:active:not(:disabled) { background: #dae7fc; color: #174a75; }
.menu-dropdown button:disabled { color: #9aa5b4; cursor: default; }
.menu-operation-buttons { display: grid; gap: 5px; }
.menu-operation-buttons button { border: 2px solid transparent; border-radius: 8px; background: linear-gradient(135deg, #4285f4 0%, #357ae8 100%); color: #fff; font-weight: 600; box-shadow: 0 3px 8px rgba(66, 133, 244, 0.3), 0 1px 3px rgba(66, 133, 244, 0.2); }
.menu-operation-buttons button:hover:not(:disabled), .menu-operation-buttons button:focus-visible { background: linear-gradient(135deg, #357ae8 0%, #2a67c8 100%); color: #fff; }
.menu-operation-buttons button:active:not(:disabled) { background: #2a67c8; color: #fff; }
.menu-operation-buttons .menu-format-button { background: linear-gradient(135deg, #c45c12 0%, #a94b0b 100%); box-shadow: 0 3px 8px rgba(196, 92, 18, 0.3), 0 1px 3px rgba(196, 92, 18, 0.2); }
.menu-operation-buttons .menu-format-button:hover:not(:disabled), .menu-operation-buttons .menu-format-button:focus-visible { background: linear-gradient(135deg, #a94b0b 0%, #8e3e08 100%); }
.menu-operation-buttons .menu-format-button:active:not(:disabled) { background: #8e3e08; }
.menu-operation-buttons button:disabled { background: #e0e0e0; color: #9e9e9e; opacity: 0.6; box-shadow: none; }
.menu-log-section, .menu-info-section { margin-top: 5px; padding-top: 5px; border-top: 1px solid #e0e5ed; }
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
/* Match ts-sddirect's inner capacity span, whose weight is reset by base.css. */
.sd-capacity-display-inline > span { font-weight: normal; }
.sd-write-wrapper { position: relative; display: inline-flex; flex-direction: column; flex-shrink: 0; }
.drive-disabled { opacity: 0.5; cursor: default; }
.sd-write-button { padding: 0.45rem 1.1rem; background: linear-gradient(135deg, #4285f4 0%, #357ae8 100%); color: white; border: 2px solid transparent; border-radius: 8px; font-family: inherit; font-size: 0.95rem; font-weight: 600; cursor: pointer; transition: all 0.3s cubic-bezier(0.4, 0, 0.2, 1); outline: none; white-space: nowrap; box-shadow: 0 3px 8px rgba(66, 133, 244, 0.3), 0 1px 3px rgba(66, 133, 244, 0.2); position: relative; overflow: hidden; }
.sd-write-button { height: 42px; padding-block: 0; line-height: 1.5; }
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
	.sd-write-button { height: 38px; padding-inline: 0.65rem; }
	.no-drive-warning-inline { padding: 5px; gap: 3px; font-size: 12px; white-space: normal; }
}
@container sd-menu-panel (max-width: 200px) {
	.drive-select-inline { grid-column: 1 / -1; grid-row: 2; }
	.menu-actions { grid-column: 3; grid-row: 1; }
	.no-drive-warning-inline { padding-inline: 0.4rem; white-space: normal; }
}
</style>
