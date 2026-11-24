export interface IconFile {
    filename: string;
    modifiedAt: number | null;
    size: number | null;
}

/** Session-only LRU cache shared by both file tabs. Unknown metadata is never cached. */
export class FileIconCache {
    private entries = new Map<string, { modifiedAt: number; size: number; icon: string }>();

    constructor(private readonly capacity = 512) {}

    private key(directory: string, filename: string) {
        return `${directory.replace(/\//g, '\\').replace(/^\\\\\?\\UNC\\/i, '\\\\')
            .replace(/^\\\\\?\\/, '').replace(/\\+$/, '')}\\${filename}`.toLowerCase();
    }

    get(directory: string, file: IconFile): string | undefined {
        const key = this.key(directory, file.filename);
        const entry = this.entries.get(key);
        if (!entry) return undefined;
        this.entries.delete(key);
        if (entry.modifiedAt !== file.modifiedAt || entry.size !== file.size) return undefined;
        this.entries.set(key, entry);
        return entry.icon;
    }

    set(directory: string, file: IconFile, icon: string) {
        if (file.modifiedAt == null || file.size == null || !icon) return;
        const key = this.key(directory, file.filename);
        this.entries.delete(key);
        this.entries.set(key, { modifiedAt: file.modifiedAt, size: file.size, icon });
        while (this.entries.size > this.capacity) this.entries.delete(this.entries.keys().next().value!);
    }
}
