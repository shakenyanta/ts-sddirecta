<script setup lang="ts">
import { ref } from 'vue';

const menuHeight = ref(48);
const sdHeight = ref(120);

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
				<div id="panel_sd" class="panel_sd"></div>
				<div id="resizer2" class="resizer" @pointerdown="resize($event, 2)"></div>
				<div id="panel_text" class="panel_text"></div>
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
.resizer {
	background: #666;
	cursor: row-resize;
	touch-action: none;
	user-select: none;
}
</style>
