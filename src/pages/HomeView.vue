<script setup lang="ts">

</script>

<template>
	<div class="panel-outer">
		<div class="panel-inner">
			<div class="panel-base"></div>
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
	padding: 12px;
	box-sizing: border-box;
}
</style>
