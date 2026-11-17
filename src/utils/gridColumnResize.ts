import { ref } from 'vue';

export function useGridColumnResize() {
    const resizedWidths = ref<number[] | null>(null);
    const resizing = ref(false);
    let state: { index: number; pointerId: number; startX: number; startWidth: number } | null = null;

    function startResize(event: PointerEvent, index: number) {
        if (event.button !== 0 || state) return;
        const handle = event.currentTarget as HTMLElement;
        const header = handle.closest('tr');
        if (!header) return;
        // 自動配分列も実幅で固定し、操作していない列の幅を保つ。
        const widths = Array.from(header.cells, cell => cell.getBoundingClientRect().width);
        resizedWidths.value = widths;
        state = { index, pointerId: event.pointerId, startX: event.clientX, startWidth: widths[index] };
        resizing.value = true;
        handle.setPointerCapture(event.pointerId);
        event.preventDefault();
    }

    function resizeColumn(event: PointerEvent) {
        if (!state || event.pointerId !== state.pointerId || !resizedWidths.value) return;
        resizedWidths.value[state.index] = Math.max(24, state.startWidth + event.clientX - state.startX);
    }

    function stopResize(event: PointerEvent) {
        if (!state || event.pointerId !== state.pointerId) return;
        state = null;
        resizing.value = false;
        const handle = event.currentTarget as HTMLElement;
        if (handle.hasPointerCapture(event.pointerId)) handle.releasePointerCapture(event.pointerId);
    }

    return { resizedWidths, resizing, startResize, resizeColumn, stopResize };
}
