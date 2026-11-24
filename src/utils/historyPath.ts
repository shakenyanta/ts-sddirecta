export const historyPathColors = [
	{ id: 'blue', label: '青', color: '#173f78', backgroundColor: '#e8f1ff' },
	{ id: 'green', label: '緑', color: '#205638', backgroundColor: '#e5f4e9' },
	{ id: 'orange', label: '橙', color: '#783b13', backgroundColor: '#fff0df' },
	{ id: 'purple', label: '紫', color: '#583580', backgroundColor: '#f1e8fa' },
	{ id: 'red', label: '赤', color: '#842c38', backgroundColor: '#fde8ec' },
	{ id: 'teal', label: '青緑', color: '#155962', backgroundColor: '#e0f4f4' },
	{ id: 'yellow', label: '黄', color: '#635014', backgroundColor: '#fff7d6' },
	{ id: 'gray', label: '灰', color: '#374151', backgroundColor: '#edf0f4' },
] as const;
type HistoryPathColor = typeof historyPathColors[number]['id'] | 'none';
export interface HistoryPathRule { label: string; folder: string; color?: HistoryPathColor; filter?: 'ts' | 'none' }

export function historyPathColorStyle(color: HistoryPathColor = 'blue') {
	if (color === 'none') return {};
	const preset = historyPathColors.find(preset => preset.id === color) ?? historyPathColors[0];
	return { color: preset.color, backgroundColor: preset.backgroundColor };
}

export const historyPathRulesStorageKey = 'sddirecta-history-path-rules-v1';
export const fixedHistoryPathRules: readonly Readonly<HistoryPathRule>[] = [
	{ label: 'TS-02各社', folder: String.raw`\\130.2.1.173\disk\保存ファイル\ＴＳ－０２テスト用\各社設定`, color: 'red', filter: 'ts' },
	{ label: 'TS-01各社', folder: String.raw`\\130.2.1.173\disk\保存ファイル\ＴＳテスト用\各社設定20170314`, color: 'teal', filter: 'ts' },
	{ label: 'TS-02開発', folder: String.raw`\\130.2.1.173\disk\保存ファイル\ＴＳ－０２テスト用\TS02開発中HEX`, color: 'yellow', filter: 'ts' },
	{ label: 'TS-01開発', folder: String.raw`\\130.2.1.173\disk\保存ファイル\ＴＳテスト用\TS開発中HEX`, color: 'gray', filter: 'ts' },
];
export const defaultHistoryPathRules: readonly HistoryPathRule[] = [
	{ label: '共有', folder: String.raw`\\130.2.1.173\disk\保存ファイル` },
	{ label: 'ローカル', folder: String.raw`C:\Users\SYSTEM-PC7` },
];

function normalizedFolder(path: string): string {
	return path.replace(/\//g, '\\').replace(/\\+$/, '');
}

/** Fixed folders always use the application definition, including for older saved settings. */
export function editableHistoryPathRules(rules: readonly HistoryPathRule[]): HistoryPathRule[] {
	const fixedFolders = new Set(fixedHistoryPathRules.map(rule => normalizedFolder(rule.folder).toLowerCase()));
	return rules.filter(rule => !fixedFolders.has(normalizedFolder(rule.folder).toLowerCase()));
}

/** Validate the entire draft before saving; an empty list disables shortening. */
export function validateHistoryPathRules(value: unknown, rowOffset = 0): HistoryPathRule[] {
	if (!Array.isArray(value)) throw Error('省略設定の形式が不正です。');
	const labels = new Set<string>();
	const folders = new Set<string>();
	return value.map((rule, index) => {
		index += rowOffset;
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
		if (rule.color !== undefined && rule.color !== 'none' && !historyPathColors.some(preset => preset.id === rule.color)) throw Error(`${index + 1}行目の配色が不正です。`);
		if (rule.filter !== undefined && rule.filter !== 'ts' && rule.filter !== 'none') throw Error(`${index + 1}行目のフィルタが不正です。`);
		labels.add(label.toLowerCase());
		folders.add(folder.toLowerCase());
		return { label, folder: /^[a-z]:$/i.test(folder) ? folder + '\\' : folder, ...(rule.color === undefined ? {} : { color: rule.color }), ...(rule.filter === undefined ? {} : { filter: rule.filter }) };
	});
}

export function parseHistoryPathRules(raw: string): HistoryPathRule[] {
	const state = JSON.parse(raw);
	if (!state || state.version !== 1) throw Error('省略設定のバージョンが不正です。');
	return validateHistoryPathRules(state.rules);
}

/** Match whole directories and prefer the deepest rule, independent of list order. */
function matchingHistoryPathRule(path: string, rules: readonly HistoryPathRule[]) {
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
	return match;
}

export function historyDisplayPath(path: string, rules: readonly HistoryPathRule[] = [...fixedHistoryPathRules, ...defaultHistoryPathRules]): { label: string; path: string } {
	const match = matchingHistoryPathRule(path, rules);
	const length = match ? normalizedFolder(match.folder).length : 0;
	return match ? { label: match.label, path: normalizedFolder(path).length === length ? '.' : path.slice(length + 1) } : { label: '', path };
}

/** Color the entire entry only when every path has the same matching color. */
export function historyPathsColorStyle(paths: readonly string[], rules: readonly HistoryPathRule[]) {
	const colors = paths.map(path => {
		const rule = matchingHistoryPathRule(path, rules);
		return rule ? rule.color ?? 'blue' : undefined;
	});
	const color = colors[0];
	return color && colors.every(value => value === color) ? historyPathColorStyle(color) : {};
}
