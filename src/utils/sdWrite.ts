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

export interface OperationStatus {
  run_id: string; operation_id: string; detail: string; operation_ms: number; waiting: boolean;
}

export function explainSdFailure(message: string): { message: string; cause: string; action: string } {
  // Only classify the primary failure. A later diagnostic query can have its own error code.
  const code = Number(message.split('\n', 1)[0].match(/os error (\d+)/)?.[1]);
  let cause = 'このエラー情報だけでは原因を特定できません。';
  let action = '「診断ログを開く」で詳細を確認し、必要に応じてログを共有してください。';
  if (message.split('\n', 1)[0].includes('SD書込確定失敗')) {
    cause = '書き込み後の確定処理に失敗しました。データの保存と照合は完了していません。';
    action = '診断ログを保存してください。書込API・確定処理・照合のどの段階まで成功したか確認が必要です。';
  } else if (code === 5) {
    cause = 'Windowsがアクセスを拒否しました。使用中の状態やアクセス権、カードリーダー側の状態などが原因候補です。';
    action = '対象を使用中のアプリを閉じてください。SDへのアクセスで再発する場合は、処理終了後に本アプリを終了し、リーダーをUSBごと抜き差ししてください。内蔵リーダーではPCを再起動してください。';
  } else if (code === 19) {
    cause = 'デバイスから書き込み禁止が報告されました。';
    action = 'SDカードやアダプターのLOCKスイッチを確認してください。';
  } else if ([21, 1167].includes(code)) {
    cause = 'デバイスが準備できていないか、接続が失われた可能性があります。';
    action = '処理終了後にカード・リーダーの接続を確認し、接続し直してください。';
  } else if (code === 32) {
    cause = '別の処理による使用が報告されました。';
    action = '対象を開いているエクスプローラーやアプリを閉じてください。';
  } else if ([23, 1117].includes(code)) {
    cause = 'データ転送・読み書きのエラーが報告されました。カード・リーダー・接続経路などが原因候補です。';
    action = '処理終了後に接続を確認し、別のリーダーで結果が変わるか切り分けてください。';
  }
  return { message, cause, action };
}

export function writePercent(progress: WriteProgress | null): number {
  if (!progress || progress.total_bytes === 0) return 0;
  if (progress.stage === 'done') return 100;
  return Math.min(99, Math.floor(progress.processed_bytes / progress.total_bytes * 100));
}

export function writeStageLabel(stage: string): string {
  return ({ prepare: '準備中', format_prepare: 'フォーマット準備中', confirm: '確認中', waiting: '権限確認待ち',
    clean: '出力先を初期化中', format_partition: 'パーティション作成中', format_volume: 'フォーマット中', format_verify: '動作確認中', format_finalize: 'フォーマット完了処理中', format_test: 'FAT簡易テスト中', read: '読込中', write: '書込中', verify: '照合中', file_done: '処理中', final_verify: '最終照合中',
    final_file_done: '最終照合中', final_failed: '最終照合失敗', done: '完了', failed: '失敗' } as Record<string, string>)[stage] ?? '準備中';
}

export function formatByteCount(bytes: number): string {
  return `${bytes.toLocaleString('ja-JP')}(0x${bytes.toString(16)})`;
}

// Match the card selector's capacity class, e.g. 497,549,312 bytes -> 512M.
export function formatCardCapacity(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return '不明';
  let exponent = 20;
  for (let i = 21; i <= 42; i++) {
    if (Math.abs(bytes - 2 ** i) < Math.abs(bytes - 2 ** exponent)) exponent = i;
  }
  const unitExponent = exponent >= 40 ? 40 : exponent >= 30 ? 30 : 20;
  return `${2 ** (exponent - unitExponent)}${unitExponent === 40 ? 'T' : unitExponent === 30 ? 'G' : 'M'}`;
}

export function sizeComment(entry: WriteEntry): string | null {
  if (entry.file_size > entry.size) return `${entry.filename}:[警告]容量オーバー`;
  if (entry.file_size === 0 || entry.file_size < entry.size) return `${entry.filename}:[注意]容量不足 → FF埋め`;
  return null;
}
