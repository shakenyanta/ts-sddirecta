/** Display filesystem timestamps in the user's local time, to the second. */
export function formatFileTimestamp(timestamp: number | null | undefined): string {
    if (timestamp == null || !Number.isFinite(timestamp)) return '';
    const date = new Date(timestamp);
    if (Number.isNaN(date.getTime())) return '';
    const pad = (value: number) => String(value).padStart(2, '0');
    return `${date.getFullYear()}/${pad(date.getMonth() + 1)}/${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`;
}
