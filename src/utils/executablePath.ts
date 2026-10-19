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
