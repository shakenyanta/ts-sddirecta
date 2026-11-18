export interface HistoryPathRule { label: string; folder: string }

export const historyPathRulesStorageKey = 'sddirecta-history-path-rules-v1';
export const defaultHistoryPathRules: readonly HistoryPathRule[] = [
	{ label: '共有', folder: String.raw`\\130.2.1.173\disk\保存ファイル` },
	{ label: 'ローカル', folder: String.raw`C:\Users\SYSTEM-PC7` },
];

function normalizedFolder(path: string): string {
	return path.replace(/\//g, '\\').replace(/\\+$/, '');
}

/** Validate the entire draft before saving; an empty list disables shortening. */
export function validateHistoryPathRules(value: unknown): HistoryPathRule[] {
	if (!Array.isArray(value)) throw Error('省略設定の形式が不正です。');
	const labels = new Set<string>();
	const folders = new Set<string>();
	return value.map((rule, index) => {
		if (!rule || typeof rule.label !== 'string' || typeof rule.folder !== 'string') throw Error(`${index + 1}行目の形式が不正です。`);
		const label = rule.label.trim();
		const folder = normalizedFolder(rule.folder.trim());
		if (!label || /[\[\]\x00-\x1f]/.test(label)) throw Error(`${index + 1}行目の表示名を入力してください（角括弧・制御文字は使えません）。`);
		const absolute = /^[a-z]:\\/i.test(rule.folder.trim().replace(/\//g, '\\')) || /^\\\\[^\\]+\\[^\\]+/.test(folder);
		if (!absolute || /[<>"|?*\x00-\x1f]/.test(folder) || folder.slice(2).includes(':')
			|| folder.split('\\').some(part => part === '.' || part === '..') || /\\\\/.test(folder.slice(2))) {
			throw Error(`${index + 1}行目にはドライブから始まる絶対パス、または共有フォルダーのUNCパスを入力してください。`);
		}
		if (labels.has(label.toLowerCase()) || folders.has(folder.toLowerCase())) throw Error(`${index + 1}行目の表示名またはフォルダーが重複しています。`);
		labels.add(label.toLowerCase());
		folders.add(folder.toLowerCase());
		return { label, folder: /^[a-z]:$/i.test(folder) ? folder + '\\' : folder };
	});
}

export function parseHistoryPathRules(raw: string): HistoryPathRule[] {
	const state = JSON.parse(raw);
	if (!state || state.version !== 1) throw Error('省略設定のバージョンが不正です。');
	return validateHistoryPathRules(state.rules);
}

/** Match whole directories and prefer the deepest rule, independent of list order. */
export function historyDisplayPath(path: string, rules: readonly HistoryPathRule[] = defaultHistoryPathRules): { label: string; path: string } {
	const compared = normalizedFolder(path).toLowerCase();
	let match: HistoryPathRule | undefined;
	let length = -1;
	for (const rule of rules) {
		const base = normalizedFolder(rule.folder).toLowerCase();
		if ((compared === base || compared.startsWith(base + '\\')) && base.length > length) {
			match = rule;
			length = base.length;
		}
	}
	return match ? { label: match.label, path: compared.length === length ? '.' : path.slice(length + 1) } : { label: '', path };
}
