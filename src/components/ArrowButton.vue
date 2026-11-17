<script setup lang="ts">
defineProps<{ direction: 'up' | 'down'; label: string; disabled?: boolean }>();
</script>

<template>
    <button type="button" class="arrow-button" :disabled="disabled" :title="label" :aria-label="label">
        <svg viewBox="0 0 16 16" aria-hidden="true" :class="{ down: direction === 'down' }">
            <path d="M8 13V3M3.5 7.5 8 3l4.5 4.5" />
        </svg>
    </button>
</template>

<style scoped>
.arrow-button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    width: 28px;
    height: 26px;
    border: 1px solid #b8cce5;
    border-radius: 4px;
    background: #edf4fd;
    color: #255b9f;
    cursor: pointer;
}
.arrow-button:hover:not(:disabled) { background: #dbeafe; }
.arrow-button:focus-visible { outline: 2px solid #2563eb; outline-offset: 1px; }
.arrow-button:disabled { opacity: .35; cursor: default; }
svg { width: 16px; height: 16px; fill: none; stroke: currentColor; stroke-width: 1.6; stroke-linecap: round; stroke-linejoin: round; }
svg.down { transform: rotate(180deg); }
</style>
