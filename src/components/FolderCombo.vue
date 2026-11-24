<script setup lang="ts">
import { computed, nextTick, ref } from 'vue';
import { historyDisplayPath, historyPathColorStyle, type HistoryPathRule } from '../utils/historyPath';

const props = defineProps<{ rules: readonly HistoryPathRule[]; selectedFolder: string; disabled: boolean }>();
const emit = defineEmits<{ select: [folder: string] }>();
const expanded = ref(false);
const trigger = ref<HTMLButtonElement | null>(null);
const list = ref<HTMLDivElement | null>(null);
const selected = computed(() => props.rules.find(rule => rule.label === historyDisplayPath(props.selectedFolder, props.rules).label));
const position = ref({ left: '0px', top: '0px', width: '0px', maxHeight: '240px' });

function optionLabel(rule: HistoryPathRule) {
	return `${rule.label}（${rule.filter === 'ts' ? 'TS用' : 'なし'}）`;
}

async function open(last = false) {
	if (props.disabled || !props.rules.length) return;
	const bounds = trigger.value!.getBoundingClientRect();
	position.value = { left: `${bounds.left}px`, top: `${bounds.bottom + 3}px`, width: `${bounds.width}px`, maxHeight: `${Math.min(240, window.innerHeight - bounds.bottom - 7)}px` };
	expanded.value = true;
	await nextTick();
	list.value?.showPopover();
	const options = list.value?.querySelectorAll<HTMLButtonElement>('[role="option"]');
	const index = props.rules.findIndex(rule => rule === selected.value);
	options?.[index >= 0 ? index : last ? options.length - 1 : 0]?.focus();
}

function close() {
	list.value?.hidePopover();
	expanded.value = false;
	trigger.value?.focus();
}

function choose(folder: string) {
	close();
	if (!props.disabled) emit('select', folder);
}

function navigate(event: KeyboardEvent) {
	const options = [...(list.value?.querySelectorAll<HTMLButtonElement>('[role="option"]') ?? [])];
	const index = options.indexOf(document.activeElement as HTMLButtonElement);
	let next: number;
	if (event.key === 'ArrowDown') next = (index + 1) % options.length;
	else if (event.key === 'ArrowUp') next = (index - 1 + options.length) % options.length;
	else if (event.key === 'Home') next = 0;
	else if (event.key === 'End') next = options.length - 1;
	else return;
	event.preventDefault();
	options[next]?.focus();
}
</script>

<template>
	<div class="folder-combo" @focusout="event => { if (!(event.currentTarget as HTMLElement).contains(event.relatedTarget as Node)) expanded = false; }" @keydown.esc.prevent.stop="close">
		<button ref="trigger" type="button" class="combo-trigger" aria-haspopup="listbox" :aria-expanded="expanded && !disabled"
			aria-label="登録フォルダーを選択" :disabled="disabled || !rules.length" :style="historyPathColorStyle(selected?.color)"
			@click="expanded ? close() : open()" @keydown.down.prevent="open()" @keydown.up.prevent="open(true)">
			<span>{{ selected ? optionLabel(selected) : 'フォルダーを選択' }}</span><span aria-hidden="true">▾</span>
		</button>
		<div v-if="expanded && !disabled" ref="list" popover="auto" class="combo-options" :style="position" role="listbox" aria-label="登録フォルダー" @keydown="navigate" @toggle="event => { if ((event as ToggleEvent).newState === 'closed') expanded = false; }">
			<button v-for="rule in rules" :key="rule.folder" type="button" role="option" :aria-selected="rule === selected"
				:style="historyPathColorStyle(rule.color)" :title="rule.folder" @click="choose(rule.folder)">{{ optionLabel(rule) }}</button>
		</div>
	</div>
</template>

<style scoped>
.folder-combo { position: relative; flex: 1; min-width: 0; max-width: 240px; }
.folder-combo button { box-sizing: border-box; width: 100%; padding: 4px 7px; border: 1px solid #adc9ed; border-radius: 3px; background: #fff; color: CanvasText; font: inherit; text-align: left; cursor: pointer; }
.combo-trigger { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
.combo-trigger span:first-child { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.combo-trigger:disabled { opacity: .6; cursor: default; }
.combo-options { position: fixed; inset: auto; margin: 0; overflow-y: auto; box-sizing: border-box; padding: 4px; background: white; border: 1px solid #adc9ed; border-radius: 4px; box-shadow: 0 3px 10px #173f7830; }
.combo-options:popover-open { display: grid; gap: 3px; }
.folder-combo button:focus-visible { outline: 2px solid #4285d4; outline-offset: -2px; }
.combo-options button:hover { outline: 1px solid #4285d4; outline-offset: -1px; }
</style>
