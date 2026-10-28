/** Keep a UNC server/share together as the root of the folder hierarchy. */
export function splitFolderPath(path: string): string[] {
	const normalized = path.replace(/^\\\\\?\\UNC\\/i, '\\\\').replace(/^\\\\\?\\/, '');
	const parts = normalized.split(/[\\/]+/).filter(Boolean);
	if (normalized.startsWith('\\\\')) {
		return parts.length ? ['\\\\' + parts.splice(0, 2).join('\\'), ...parts] : [];
	}
	if (/^[a-z]:$/i.test(parts[0] ?? '')) {
		parts[0] += '\\';
	} else if (normalized.startsWith('/')) {
		parts.unshift('/');
	}
	return parts;
}

/** Resolve a displayed hierarchy level, preserving drive and UNC roots. */
export function folderPathAt(path: string, index: number): string {
	const folders = splitFolderPath(path);
	if (!Number.isInteger(index) || index < 0 || index >= folders.length) return '';
	const separator = folders[0] === '/' ? '/' : '\\';
	return folders.slice(0, index + 1).reduce((parent, folder) =>
		parent.replace(/[\\/]+$/, '') + separator + folder);
}
