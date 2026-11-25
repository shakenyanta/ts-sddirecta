use std::path::{Path, PathBuf};
use serde::{Serialize, Deserialize};
use encoding_rs::SHIFT_JIS;
#[derive(Serialize)]
struct FileBqReceiptPrepareResult {
    work_dir: String,
    bq_txt_dir: String,
    copied_file_path: String,
    file_count: usize,
    reused_existing: bool,
}
#[derive(Serialize, Deserialize)]
struct FileBqReceiptCacheMeta {
    source_size: u64,
    source_mtime_secs: i64,
    file_count: usize,
}
const FILE_BQ_RECORD_SIZE: usize = 32;
const FILE_BQ_MAX_RECORDS: usize = 4096;

fn normalize_windows_path(path: &str) -> String {
    path.replace('/', "\\").to_lowercase()
}

fn hash_file_bq_path(path: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in normalize_windows_path(path).bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn file_bq_split_root_dir() -> Result<PathBuf, String> {
    let local_app_data = std::env::var_os("LOCALAPPDATA")
        .ok_or_else(|| "%LOCALAPPDATA% を取得できませんでした".to_string())?;
    let root_dir = PathBuf::from(local_app_data)
        .join("ts-sddirecta")
        .join("tmp")
        .join("split");
    std::fs::create_dir_all(&root_dir).map_err(|e| e.to_string())?;
    Ok(root_dir)
}

fn file_bq_cache_meta_path(work_dir: &Path) -> PathBuf {
    work_dir.join("cache-meta.json")
}

fn file_size_and_mtime(path: &Path) -> Result<(u64, i64), String> {
    let metadata = std::fs::metadata(path).map_err(|e| e.to_string())?;
    let size = metadata.len();
    let modified = metadata.modified().map_err(|e| e.to_string())?;
    let mtime_secs = modified
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .map_err(|e| e.to_string())?;
    Ok((size, mtime_secs))
}

fn read_file_bq_cache_meta(work_dir: &Path) -> Result<FileBqReceiptCacheMeta, String> {
    let meta_path = file_bq_cache_meta_path(work_dir);
    let json = std::fs::read_to_string(&meta_path).map_err(|e| e.to_string())?;
    serde_json::from_str::<FileBqReceiptCacheMeta>(&json).map_err(|e| e.to_string())
}

fn write_file_bq_cache_meta(work_dir: &Path, source_path: &Path, file_count: usize) -> Result<(), String> {
    let (source_size, source_mtime_secs) = file_size_and_mtime(source_path)?;
    let meta = FileBqReceiptCacheMeta {
        source_size,
        source_mtime_secs,
        file_count,
    };
    let meta_path = file_bq_cache_meta_path(work_dir);
    let json = serde_json::to_string(&meta).map_err(|e| e.to_string())?;
    std::fs::write(meta_path, json).map_err(|e| e.to_string())
}

fn get_reusable_file_bq_cache_meta(source_path: &Path, work_dir: &Path) -> Result<Option<FileBqReceiptCacheMeta>, String> {
    if !work_dir.exists() {
        return Ok(None);
    }

    let copied_file_path = work_dir.join("FILE-BQ.001");
    let bq_txt_dir = work_dir.join("BQ_TXT");
    let (source_size, source_mtime_secs) = file_size_and_mtime(source_path)?;

    if !copied_file_path.is_file() || !bq_txt_dir.is_dir() {
        return Ok(None);
    }

    let cache_meta = match read_file_bq_cache_meta(work_dir) {
        Ok(meta) => meta,
        Err(_) => return Ok(None),
    };

    if cache_meta.source_size == source_size && cache_meta.source_mtime_secs == source_mtime_secs {
        return Ok(Some(cache_meta));
    }

    Ok(None)
}
fn decode_bq_single_byte(byte: u8) -> String {
    if byte.is_ascii_graphic() || byte == b' ' {
        return char::from(byte).to_string();
    }
    if (0xA1..=0xDF).contains(&byte) {
        let single = [byte];
        let (decoded, _, had_errors) = SHIFT_JIS.decode(&single);
        if !had_errors {
            return decoded.into_owned();
        }
    }
    ".".to_string()
}

fn decode_bq_name(bytes: &[u8]) -> String {
    let mut decoded = String::new();
    let mut lead_byte: Option<u8> = None;

    for &byte in bytes {
        if let Some(lead) = lead_byte.take() {
            let pair = [lead, byte];
            let (value, _, had_errors) = SHIFT_JIS.decode(&pair);
            if had_errors {
                decoded.push('.');
                decoded.push('.');
            } else {
                decoded.push_str(&value);
            }
            continue;
        }

        if (0x81..=0x9F).contains(&byte) {
            lead_byte = Some(byte);
            continue;
        }

        decoded.push_str(&decode_bq_single_byte(byte));
    }

    if lead_byte.is_some() {
        decoded.push('.');
    }

    decoded
}

fn parse_bq_record_name(record: &[u8]) -> String {
    let no = format!(
        "{}{}{}{}",
        char::from((record[0] >> 4) | 0x30),
        char::from((record[0] & 0x0f) | 0x30),
        char::from((record[1] >> 4) | 0x30),
        char::from((record[1] & 0x0f) | 0x30),
    );
    let name = decode_bq_name(&record[0x0a..0x20]);
    format!("{no}_{name}.txt")
}

fn parse_bq_record_range(record: &[u8]) -> (usize, usize) {
    let start = u32::from_be_bytes([record[2], record[3], record[4], record[5]]) as usize;
    let end = u32::from_be_bytes([record[6], record[7], record[8], record[9]]) as usize;
    (start, end)
}

fn file_bq_receipt_contents(source_path: &Path) -> Result<(usize, std::collections::HashMap<String, Vec<u8>>), String> {
    let file_bytes = std::fs::read(source_path).map_err(|e| e.to_string())?;
    let mut file_count = 0usize;
    let mut contents = std::collections::HashMap::new();

    for record_index in 0..FILE_BQ_MAX_RECORDS {
        let record_start = record_index * FILE_BQ_RECORD_SIZE;
        let record_end = record_start + FILE_BQ_RECORD_SIZE;
        if record_end > file_bytes.len() {
            break;
        }

        let record = &file_bytes[record_start..record_end];
        if record[0] == 0xff && record[1] == 0xff {
            break;
        }

        let file_name = parse_bq_record_name(record);
        if file_name.contains(['/', '\\', ':']) { return Err("Invalid receipt name".into()); }
        let (start_addr, end_addr) = parse_bq_record_range(record);
        if end_addr < start_addr || end_addr >= file_bytes.len() {
            return Err(format!("FILE-BQ.001 のレコード範囲が不正です: {file_name}"));
        }
        if start_addr >= file_bytes.len() {
            return Err(format!("FILE-BQ.001 の開始アドレスが範囲外です: {file_name}"));
        }

        let mut out = Vec::with_capacity(end_addr.saturating_sub(start_addr) + 1);
        let last_index = end_addr.min(file_bytes.len().saturating_sub(1));
        for &byte in &file_bytes[start_addr..=last_index] {
            if byte == 0xff {
                break;
            }
            out.push(byte);
        }

        contents.insert(file_name, out);
        file_count += 1;
    }

    Ok((file_count, contents))
}

fn split_file_bq_receipt(source_path: &Path, bq_txt_dir: &Path) -> Result<usize, String> {
    let (file_count, contents) = file_bq_receipt_contents(source_path)?;
    for (name, bytes) in contents {
        std::fs::write(bq_txt_dir.join(name), bytes).map_err(|e| e.to_string())?;
    }
    Ok(file_count)
}

fn prepare_file_bq_receipt(path: String, force_rebuild: Option<bool>) -> Result<FileBqReceiptPrepareResult, String> {
    let source_path = PathBuf::from(&path);
    let file_name = source_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "ファイル名を取得できませんでした".to_string())?;

    if !file_name.eq_ignore_ascii_case("FILE-BQ.001") {
        return Err("FILE-BQ.001 ではありません".to_string());
    }
    if !source_path.is_file() {
        return Err("指定パスはファイルではありません".to_string());
    }

    let work_dir = file_bq_split_root_dir()?.join(hash_file_bq_path(&path));
    let copied_file_path = work_dir.join("FILE-BQ.001");
    let bq_txt_dir = work_dir.join("BQ_TXT");
    let force_rebuild = force_rebuild.unwrap_or(false);

    if work_dir.exists() {
        if !force_rebuild {
            if let Some(cache_meta) = get_reusable_file_bq_cache_meta(&source_path, &work_dir)? {
                return Ok(FileBqReceiptPrepareResult {
                    work_dir: work_dir.to_string_lossy().to_string(),
                    bq_txt_dir: bq_txt_dir.to_string_lossy().to_string(),
                    copied_file_path: copied_file_path.to_string_lossy().to_string(),
                    file_count: cache_meta.file_count,
                    reused_existing: true,
                });
            }
        }

        std::fs::remove_dir_all(&work_dir).map_err(|e| e.to_string())?;
    }
    std::fs::create_dir_all(&work_dir).map_err(|e| e.to_string())?;

    std::fs::copy(&source_path, &copied_file_path).map_err(|e| e.to_string())?;

    std::fs::create_dir_all(&bq_txt_dir).map_err(|e| e.to_string())?;

    let file_count = split_file_bq_receipt(&copied_file_path, &bq_txt_dir)?;
    write_file_bq_cache_meta(&work_dir, &source_path, file_count)?;

    Ok(FileBqReceiptPrepareResult {
        work_dir: work_dir.to_string_lossy().to_string(),
        bq_txt_dir: bq_txt_dir.to_string_lossy().to_string(),
        copied_file_path: copied_file_path.to_string_lossy().to_string(),
        file_count,
        reused_existing: false,
    })
}


#[derive(Serialize)]
pub struct ReceiptRow { name: String, bytes: Vec<u8> }
#[derive(Serialize)]
pub struct ReceiptData { rows: Vec<ReceiptRow>, source_size: u64, reused: bool }
#[tauri::command]
pub async fn read_receipts(directory: String) -> Result<ReceiptData, String> {
 tauri::async_runtime::spawn_blocking(move || {
 let source = crate::source_directory::file_to_open(directory, "FILE-BQ.001")?;
 let prepared = prepare_file_bq_receipt(source.to_string_lossy().into_owned(), None)?;
 let mut rows = Vec::new();
 for entry in std::fs::read_dir(&prepared.bq_txt_dir).map_err(|e| e.to_string())? {
 let entry = entry.map_err(|e| e.to_string())?;
 if entry.file_type().map_err(|e| e.to_string())?.is_file() {
 rows.push(ReceiptRow { name: entry.file_name().to_string_lossy().into_owned(), bytes: std::fs::read(entry.path()).map_err(|e| e.to_string())? });
 }}
 rows.sort_by(|a,b| a.name.cmp(&b.name));
 Ok(ReceiptData { rows, source_size: std::fs::metadata(source).map_err(|e| e.to_string())?.len(), reused: prepared.reused_existing })
 }).await.map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_receipt_and_rejects_invalid_ranges_and_names() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("FILE-BQ.001");
        let mut bytes = vec![0xff; 128];
        bytes[..32].fill(0);
        bytes[2..6].copy_from_slice(&64u32.to_be_bytes());
        bytes[6..10].copy_from_slice(&67u32.to_be_bytes());
        bytes[10..32].fill(b' ');
        bytes[64..68].copy_from_slice(b"abc\xff");
        std::fs::write(&source, &bytes).unwrap();
        let (count, contents) = file_bq_receipt_contents(&source).unwrap();
        assert_eq!(count, 1);
        assert_eq!(contents.values().next().unwrap(), b"abc");
        bytes[6..10].copy_from_slice(&200u32.to_be_bytes());
        std::fs::write(&source, &bytes).unwrap();
        assert!(file_bq_receipt_contents(&source).is_err());
        bytes[6..10].copy_from_slice(&67u32.to_be_bytes());
        bytes[10] = b'/';
        std::fs::write(&source, &bytes).unwrap();
        assert!(file_bq_receipt_contents(&source).is_err());
    }
}
