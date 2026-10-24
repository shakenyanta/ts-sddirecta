export interface WriteEntry {
  filename: string; path: string; address: number; size: number; file_size: number;
}

export interface WritePlan {
  run_id: string; source: string; directory: string; drive: string;
  capacity: number; total_bytes: number; entries: WriteEntry[];
}

export interface WriteProgress {
  run_id: string; filename: string; index: number; total: number; completed: number;
  stage: string; processed_bytes: number; total_bytes: number;
  elapsed_ms: number; file_ms: number; error: string | null;
}

export function writePercent(progress: WriteProgress | null): number {
  if (!progress || progress.total_bytes === 0) return 0;
  if (progress.stage === 'done') return 100;
  return Math.min(99, Math.floor(progress.processed_bytes / progress.total_bytes * 100));
}

export function writeStageLabel(stage: string): string {
  return ({ prepare: '準備中', confirm: '確認中', waiting: '権限確認待ち',
    write: '書込中', verify: '照合中', file_done: '処理中', done: '完了', failed: '失敗' } as Record<string, string>)[stage] ?? '準備中';
}

export function sizeComment(entry: WriteEntry): string | null {
  const bytes = (n: number) => n.toLocaleString('ja-JP');
  if (entry.file_size > entry.size) return `注意: ${entry.filename} は領域サイズを超えています。ファイル: ${bytes(entry.file_size)}バイト、領域: ${bytes(entry.size)}バイト。末尾${bytes(entry.file_size - entry.size)}バイトを無視して書き込みます。`;
  if (entry.file_size === 0) return `${entry.filename}: 0バイトのため、対象領域${bytes(entry.size)}バイト全体を0xFFで埋めます。`;
  if (entry.file_size < entry.size) return `${entry.filename}: ファイル${bytes(entry.file_size)}バイトを書き込み、残り${bytes(entry.size - entry.file_size)}バイトを0xFFで埋めます。`;
  return null;
}
