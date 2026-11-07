export interface SavedFolderNode {
	path: string;
	children: string[];
	expanded: boolean;
	childrenListed: boolean;
}
export interface FolderTreeState {
	version: 1;
	root: string;
	selected: string;
	scrollTop: number;
	nodes: SavedFolderNode[];
	matches: [string, boolean][];
}

export const folderTreeStorageKey = 'sddirecta-folder-tree-v1';

// Saved UI data is optional and untrusted. Reject inconsistent trees as a whole.
export function parseFolderTreeState(raw: string): FolderTreeState {
	const state = JSON.parse(raw);
	const path = (value: unknown): value is string => typeof value === 'string'
		&& value.length > 0 && value.length <= 32768 && !/[\0]/.test(value)
		&& (/^[a-z]:\\/i.test(value) || /^\\\\[^\\]+\\[^\\]+/.test(value) || value.startsWith('/'));
	if (state?.version !== 1 || !path(state.root) || !path(state.selected)
		|| !Number.isFinite(state.scrollTop) || state.scrollTop < 0
		|| !Array.isArray(state.nodes) || state.nodes.length > 10000
		|| !Array.isArray(state.matches) || state.matches.length > 10000) throw new Error('保存形式が不正です');
	const nodes = new Map<string, SavedFolderNode>();
	for (const node of state.nodes) {
		if (!node || !path(node.path) || nodes.has(node.path) || typeof node.expanded !== 'boolean'
			|| (node.childrenListed !== undefined && typeof node.childrenListed !== 'boolean')
			|| !Array.isArray(node.children) || node.children.length > 10000
			|| !node.children.every(path)) throw new Error('保存されたフォルダーが不正です');
		// Older snapshots did not distinguish path-only ancestors from listed folders.
		node.childrenListed ??= false;
		nodes.set(node.path, node);
	}
	const pending = [state.root], visited = new Set<string>();
	while (pending.length) {
		const current = pending.pop()!;
		const node = nodes.get(current);
		if (!node || visited.has(current)) throw new Error('保存された階層が不正です');
		visited.add(current);
		for (const child of node.children) {
			const prefix = current.replace(/[\\/]+$/, '') + (current.startsWith('/') ? '/' : '\\');
			if (!child.startsWith(prefix) || /[\\/]/.test(child.slice(prefix.length))
				|| ['.', '..', ''].includes(child.slice(prefix.length))) throw new Error('保存された子階層が不正です');
			pending.push(child);
		}
	}
	if (visited.size !== nodes.size || !visited.has(state.selected)
		|| !state.matches.every((entry: unknown) => Array.isArray(entry) && entry.length === 2
			&& path(entry[0]) && typeof entry[1] === 'boolean')) throw new Error('保存された表示状態が不正です');
	return state;
}
