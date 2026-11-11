export interface DropHistoryEntry {
	id: string;
	droppedAt: number;
	paths: string[];
	favorite?: boolean;
}

export interface DropHistoryState {
	version: 1;
	entries: DropHistoryEntry[];
}

export const dropHistoryStorageKey = 'sddirecta-drop-history-v1';

// Validate the whole document so damaged history is never silently discarded.
export function parseDropHistoryState(raw: string): DropHistoryState {
	const state = JSON.parse(raw);
	if (state?.version !== 1 || !Array.isArray(state.entries)) {
		throw new Error('履歴の保存形式またはバージョンが不正です');
	}
	const ids = new Set<string>();
	for (const entry of state.entries) {
		if (!entry || typeof entry.id !== 'string'
			|| !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(entry.id)
			|| ids.has(entry.id) || !Number.isSafeInteger(entry.droppedAt)
			|| entry.droppedAt < 0 || entry.droppedAt > 8640000000000000
			|| (entry.favorite !== undefined && typeof entry.favorite !== 'boolean')
			|| !Array.isArray(entry.paths) || !entry.paths.length
			|| !entry.paths.every((path: unknown) => typeof path === 'string' && path.length > 0
				&& path.length <= 32768 && !path.includes('\0'))) {
			throw new Error('履歴のID・日時・パスが不正です');
		}
		ids.add(entry.id);
	}
	return state;
}
