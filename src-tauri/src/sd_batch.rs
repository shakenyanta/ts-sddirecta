//! Bounded-memory write/verify engine. Tests use an in-memory disk, never an SD.
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom},
    os::windows::fs::OpenOptionsExt,
    path::{Component, Path},
    time::{Instant, UNIX_EPOCH},
};
use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ;

pub const CHUNK: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Entry {
    pub filename: String,
    pub path: String,
    pub address: u64,
    pub size: u64,
    pub file_size: u64,
    pub modified: u128,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Plan {
    pub run_id: String,
    pub source: String,
    pub directory: String,
    pub drive: String,
    pub disk: u32,
    pub capacity: u64,
    pub total_bytes: u64,
    pub entries: Vec<Entry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Progress {
    pub run_id: String,
    pub filename: String,
    pub index: usize,
    pub total: usize,
    pub completed: usize,
    pub stage: String,
    pub processed_bytes: u64,
    pub total_bytes: u64,
    pub elapsed_ms: u64,
    pub file_ms: u64,
    pub error: Option<String>,
}

pub fn all_area_target(flag: &str) -> bool {
    flag != "E"
}

fn hex(value: &str) -> Result<u64, String> {
    let value = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value);
    let n = u64::from_str_radix(value, 16).map_err(|_| "CSVのアドレスが不正です")?;
    // The frontend displays these values as exact JavaScript numbers.
    if n > 9_007_199_254_740_991 {
        return Err("CSVのアドレスが対応範囲を超えています".into());
    }
    Ok(n)
}

pub fn parse_areas(csv: &str) -> Result<Vec<(String, String, u64, u64)>, String> {
    let mut result = Vec::new();
    let mut names = std::collections::HashSet::new();
    for (i, line) in csv.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let f: Vec<_> = line.split(',').map(str::trim).collect();
        if f[0] == "E" {
            break;
        }
        if f.len() < 4 || f[1].is_empty() {
            return Err(format!("CSV {}行目: 領域定義が不足しています", i + 1));
        }
        let mut components = Path::new(f[1]).components();
        if !matches!(components.next(), Some(Component::Normal(_)))
            || components.next().is_some()
            || f[1].contains(['/', '\\', ':'])
            || !names.insert(f[1].to_string())
        {
            return Err(format!(
                "CSV {}行目: ファイル名が不正または重複しています",
                i + 1
            ));
        }
        let start = hex(f[2])?;
        let end = hex(f[3])?;
        if end < start {
            return Err(format!(
                "CSV {}行目: 終了アドレスが開始アドレスより小さいです",
                i + 1
            ));
        }
        result.push((f[0].to_string(), f[1].to_string(), start, end - start + 1));
    }
    Ok(result)
}

fn stamp(file: &File) -> Result<(u64, u128), String> {
    let m = file.metadata().map_err(|e| e.to_string())?;
    if !m.is_file() {
        return Err("対象が通常のファイルではありません".into());
    }
    let modified = m
        .modified()
        .map_err(|e| e.to_string())?
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    Ok((m.len(), modified))
}

fn source_file(path: &Path) -> Result<File, String> {
    // Keep every source open without write/delete sharing for the whole batch.
    OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))
}

pub fn validate_entries(entries: &[Entry], capacity: u64) -> Result<u64, String> {
    let mut end = 0;
    let mut total = 0u64;
    for (i, entry) in entries.iter().enumerate() {
        let next = entry
            .address
            .checked_add(entry.size)
            .ok_or("領域サイズが大きすぎます")?;
        if entry.size == 0 || next > capacity || (i > 0 && entry.address < end) {
            return Err(format!(
                "{}: 領域が重複しているかSD容量外です",
                entry.filename
            ));
        }
        end = next;
        total = total
            .checked_add(entry.size)
            .ok_or("総書込サイズが大きすぎます")?;
    }
    total.checked_mul(2).ok_or("総処理量が大きすぎます")?;
    Ok(total)
}

pub fn prepare(source: &str, drive: &str, run_id: &str) -> Result<Plan, String> {
    if run_id.is_empty() || run_id.len() > 100 {
        return Err("実行IDが不正です".into());
    }
    // Query only the selected target, not every drive on the computer.
    let (disk, capacity) = crate::windows_api::removable_target(drive)?;
    let contents = crate::startup_csv_source(Some(source.to_string()))?
        .read()
        .map_err(|e| e.to_string())?;
    let areas = parse_areas(&contents)?;
    let (matched, from_input, _) = crate::matching_files::find_matches(
        Path::new(source),
        areas.iter().map(|a| a.1.clone()).collect(),
    )
    .map_err(|e| e.to_string())?;
    let directory = if from_input {
        Path::new(source).join("input")
    } else {
        Path::new(source).to_path_buf()
    };
    let directory = directory.canonicalize().map_err(|e| e.to_string())?;
    let display_directory = directory.to_string_lossy();
    if display_directory
        .trim_start_matches("\\\\?\\")
        .to_uppercase()
        .starts_with(&format!("{}\\", drive.to_uppercase()))
    {
        return Err("書込元ファイルは書込先SD以外のフォルダーへ置いてください".into());
    }
    let mut entries = Vec::new();
    for (flag, filename, address, size) in areas {
        if !all_area_target(&flag) || !matched.contains(&filename) {
            continue;
        }
        let path = directory.join(&filename);
        let file = source_file(&path)?;
        let (file_size, modified) = stamp(&file)?;
        entries.push(Entry {
            filename,
            path: path.to_string_lossy().into_owned(),
            address,
            size,
            file_size,
            modified,
        });
    }
    entries.sort_by_key(|e| e.address);
    let total_bytes = validate_entries(&entries, capacity)?;
    Ok(Plan {
        run_id: run_id.into(),
        source: source.into(),
        directory: directory.to_string_lossy().into_owned(),
        drive: drive.into(),
        disk,
        capacity,
        total_bytes,
        entries,
    })
}

pub trait Disk {
    fn sector(&self) -> usize;
    fn read(&mut self, address: u64, data: &mut [u8]) -> Result<(), String>;
    fn write(&mut self, address: u64, data: &[u8]) -> Result<(), String>;
    fn flush(&mut self) -> Result<(), String>;
}

// No-buffering disk I/O requires aligned memory as well as aligned offsets/lengths.
pub struct Buffer {
    ptr: std::ptr::NonNull<u8>,
    layout: std::alloc::Layout,
}
impl Buffer {
    pub fn new(size: usize) -> Result<Self, String> {
        let layout = std::alloc::Layout::from_size_align(size, 65536).map_err(|e| e.to_string())?;
        let ptr = std::ptr::NonNull::new(unsafe { std::alloc::alloc_zeroed(layout) })
            .ok_or("SDバッファを確保できませんでした")?;
        Ok(Self { ptr, layout })
    }
    pub fn bytes(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.layout.size()) }
    }
}
impl Drop for Buffer {
    fn drop(&mut self) {
        unsafe {
            std::alloc::dealloc(self.ptr.as_ptr(), self.layout);
        }
    }
}

fn expected(
    source: &mut (impl Read + Seek),
    entry: &Entry,
    offset: u64,
    dest: &mut [u8],
) -> Result<(), String> {
    dest.fill(0xFF);
    let len = entry
        .file_size
        .min(entry.size)
        .saturating_sub(offset)
        .min(dest.len() as u64) as usize;
    if len > 0 {
        source
            .seek(SeekFrom::Start(offset))
            .map_err(|e| e.to_string())?;
        source
            .read_exact(&mut dest[..len])
            .map_err(|e| format!("入力ファイルの読込に失敗: {e}"))?;
    }
    Ok(())
}

/// Each file is fully written, flushed, then verified. The two buffers are reused.
fn transfer(
    disk: &mut impl Disk,
    source: &mut (impl Read + Seek),
    entry: &Entry,
    io: &mut [u8],
    compare: &mut [u8],
    mut progress: impl FnMut(&str, u64) -> Result<(), String>,
) -> Result<(), String> {
    let sector = disk.sector();
    if !sector.is_power_of_two()
        || sector > 65536
        || io.len() < CHUNK + sector
        || compare.len() < CHUNK
    {
        return Err("セクターサイズが不正です".into());
    }
    for verify in [false, true] {
        let stage = if verify { "verify" } else { "write" };
        progress(stage, 0)?;
        let mut offset = 0u64;
        while offset < entry.size {
            let address = entry.address + offset;
            let aligned = address / sector as u64 * sector as u64;
            let prefix = (address - aligned) as usize;
            // End each non-final chunk on a sector boundary.
            let count = (entry.size - offset).min((CHUNK - prefix) as u64) as usize;
            let physical_len = (prefix + count).div_ceil(sector) * sector;
            let buffer = &mut io[..physical_len];
            if verify || prefix > 0 || prefix + count != physical_len {
                disk.read(aligned, buffer)?;
            }
            let payload = &mut buffer[prefix..prefix + count];
            if verify {
                expected(source, entry, offset, &mut compare[..count])?;
                if let Some(mismatch) = payload
                    .iter()
                    .zip(&compare[..count])
                    .position(|(a, b)| a != b)
                {
                    return Err(format!(
                        "照合不一致: アドレス 0x{:X}",
                        address + mismatch as u64
                    ));
                }
            } else {
                expected(source, entry, offset, payload)?;
                disk.write(aligned, buffer)?;
            }
            offset += count as u64;
            progress(stage, offset)?;
        }
        if !verify {
            disk.flush()?;
        }
    }
    Ok(())
}

pub fn execute(
    plan: Plan,
    mut notify: impl FnMut(Progress) -> Result<(), String>,
) -> Result<Progress, String> {
    let started = Instant::now();
    notify(Progress {
        run_id: plan.run_id.clone(),
        filename: String::new(),
        index: 0,
        total: plan.entries.len(),
        completed: 0,
        stage: "prepare".into(),
        processed_bytes: 0,
        total_bytes: plan.total_bytes * 2,
        elapsed_ms: 0,
        file_ms: 0,
        error: None,
    })?;
    // Validate all sources and pin them before touching the SD. Changed or missing
    // sources abort the entire batch, even if another file could be written.
    let mut files = Vec::new();
    for entry in &plan.entries {
        let file = source_file(Path::new(&entry.path))?;
        if stamp(&file)? != (entry.file_size, entry.modified) {
            return Err(format!(
                "{}: 確認後にファイルが変更されました",
                entry.filename
            ));
        }
        files.push(file);
    }
    let mut disk = crate::windows_api::BatchDisk::open(&plan.drive, plan.disk, plan.capacity)?;
    if validate_entries(&plan.entries, disk.capacity)? != plan.total_bytes {
        return Err("総書込量が一致しません".into());
    }
    let mut io = Buffer::new(CHUNK + 65536)?;
    let mut compare = Buffer::new(CHUNK)?;
    let mut processed = 0;
    for (i, (entry, source)) in plan.entries.iter().zip(files.iter_mut()).enumerate() {
        let file_started = Instant::now();
        let mut last = Instant::now();
        let mut last_stage = String::new();
        let mut confirmed = 0;
        let result = transfer(
            &mut disk,
            source,
            entry,
            io.bytes(),
            compare.bytes(),
            |stage, bytes| {
                confirmed = bytes + if stage == "verify" { entry.size } else { 0 };
                if stage != last_stage || last.elapsed().as_millis() >= 150 {
                    notify(Progress {
                        run_id: plan.run_id.clone(),
                        filename: entry.filename.clone(),
                        index: i + 1,
                        total: plan.entries.len(),
                        completed: i,
                        stage: stage.into(),
                        processed_bytes: processed + confirmed,
                        total_bytes: plan.total_bytes * 2,
                        elapsed_ms: started.elapsed().as_millis() as u64,
                        file_ms: file_started.elapsed().as_millis() as u64,
                        error: None,
                    })?;
                    last = Instant::now();
                    last_stage = stage.into();
                }
                Ok(())
            },
        );
        let error = result.as_ref().err().cloned();
        notify(Progress {
            run_id: plan.run_id.clone(),
            filename: entry.filename.clone(),
            index: i + 1,
            total: plan.entries.len(),
            completed: if error.is_none() { i + 1 } else { i },
            stage: if error.is_none() {
                "file_done"
            } else {
                "failed"
            }
            .into(),
            processed_bytes: processed + confirmed,
            total_bytes: plan.total_bytes * 2,
            elapsed_ms: started.elapsed().as_millis() as u64,
            file_ms: file_started.elapsed().as_millis() as u64,
            error: error.clone(),
        })?;
        result?;
        processed += entry.size * 2;
    }
    drop(disk); // Unlock/close before announcing normal completion.
    let done = Progress {
        run_id: plan.run_id,
        filename: String::new(),
        index: plan.entries.len(),
        total: plan.entries.len(),
        completed: plan.entries.len(),
        stage: "done".into(),
        processed_bytes: processed,
        total_bytes: plan.total_bytes * 2,
        elapsed_ms: started.elapsed().as_millis() as u64,
        file_ms: 0,
        error: None,
    };
    notify(done.clone())?;
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    struct MemoryDisk {
        data: Vec<u8>,
        corrupt: bool,
    }
    impl Disk for MemoryDisk {
        fn sector(&self) -> usize {
            512
        }
        fn read(&mut self, a: u64, d: &mut [u8]) -> Result<(), String> {
            d.copy_from_slice(&self.data[a as usize..a as usize + d.len()]);
            if self.corrupt {
                d[11] ^= 1;
            }
            Ok(())
        }
        fn write(&mut self, a: u64, d: &[u8]) -> Result<(), String> {
            self.data[a as usize..a as usize + d.len()].copy_from_slice(d);
            Ok(())
        }
        fn flush(&mut self) -> Result<(), String> {
            Ok(())
        }
    }
    fn entry(address: u64, size: u64, file_size: u64) -> Entry {
        Entry {
            filename: "a".into(),
            path: "a".into(),
            address,
            size,
            file_size,
            modified: 0,
        }
    }
    #[test]
    fn truncate_pad_empty_and_preserve_neighbors_with_unaligned_ranges() {
        for file_size in [0, 512, 1024, 1536] {
            let mut disk = MemoryDisk {
                data: vec![0x33; 2048],
                corrupt: false,
            };
            let mut source = Cursor::new(vec![0xAA; file_size]);
            let e = entry(17, 1024, file_size as u64);
            let mut io = Buffer::new(CHUNK + 65536).unwrap();
            let mut compare = Buffer::new(CHUNK).unwrap();
            transfer(
                &mut disk,
                &mut source,
                &e,
                io.bytes(),
                compare.bytes(),
                |_, _| Ok(()),
            )
            .unwrap();
            assert!(disk.data[..17].iter().all(|b| *b == 0x33));
            assert!(disk.data[17..17 + file_size.min(1024)]
                .iter()
                .all(|b| *b == 0xAA));
            assert!(disk.data[17 + file_size.min(1024)..1041]
                .iter()
                .all(|b| *b == 0xFF));
            assert!(disk.data[1041..].iter().all(|b| *b == 0x33));
        }
    }
    #[test]
    fn multiple_chunks_and_verify_mismatch() {
        let size = CHUNK + 777;
        let mut disk = MemoryDisk {
            data: vec![0x33; size + 2048],
            corrupt: false,
        };
        let e = entry(0, size as u64, size as u64);
        let mut io = Buffer::new(CHUNK + 65536).unwrap();
        let mut compare = Buffer::new(CHUNK).unwrap();
        let mut events = vec![];
        transfer(
            &mut disk,
            &mut Cursor::new(vec![0xAA; size]),
            &e,
            io.bytes(),
            compare.bytes(),
            |stage, n| {
                events.push((stage.to_owned(), n));
                Ok(())
            },
        )
        .unwrap();
        assert!(events.contains(&("write".into(), CHUNK as u64)));
        assert_eq!(events.last().unwrap(), &("verify".into(), size as u64));
        disk.corrupt = true;
        assert!(transfer(
            &mut disk,
            &mut Cursor::new(vec![0xAA; size]),
            &e,
            io.bytes(),
            compare.bytes(),
            |_, _| Ok(())
        )
        .unwrap_err()
        .contains("照合不一致"));
    }
    #[test]
    fn reject_invalid_csv_overlap_and_capacity_before_writing() {
        assert!(parse_areas("A,../bad,0,FF").is_err());
        assert!(parse_areas("A,a,0,FF\nW,a,100,1FF").is_err());
        assert!(parse_areas("A,a,100,FF").is_err());
        assert_eq!(parse_areas("A,a,0,FF\nE\ninvalid").unwrap().len(), 1);
        assert!(validate_entries(&[entry(0, 1024, 0), entry(512, 512, 0)], 2048).is_err());
        assert!(validate_entries(&[entry(1024, 1025, 0)], 2048).is_err());
        assert_eq!(
            validate_entries(&[entry(0, 1024, 0), entry(1024, 512, 0)], 2048).unwrap(),
            1536
        );
        for flag in ["A", "W", "R", "N", "0", "X"] {
            assert!(all_area_target(flag));
        }
        assert!(!all_area_target("E"));
    }

    #[test]
    fn input_read_failure_and_lost_parent_stop_before_writing() {
        let mut disk = MemoryDisk {
            data: vec![0x33; 2048],
            corrupt: false,
        };
        let e = entry(0, 512, 512);
        let mut io = Buffer::new(CHUNK + 65536).unwrap();
        let mut compare = Buffer::new(CHUNK).unwrap();
        assert!(transfer(
            &mut disk,
            &mut Cursor::new(vec![0xAA; 128]),
            &e,
            io.bytes(),
            compare.bytes(),
            |_, _| Ok(())
        )
        .is_err());
        assert!(disk.data.iter().all(|b| *b == 0x33));
        assert!(transfer(
            &mut disk,
            &mut Cursor::new(vec![0xAA; 512]),
            &e,
            io.bytes(),
            compare.bytes(),
            |_, _| Err("親プロセス切断".into())
        )
        .is_err());
        assert!(disk.data.iter().all(|b| *b == 0x33));
        // Capacity/progress math is 64-bit even when a single area exceeds 4GiB.
        assert_eq!(
            validate_entries(
                &[entry(0, 5 * 1024 * 1024 * 1024, 0)],
                8 * 1024 * 1024 * 1024
            )
            .unwrap(),
            5 * 1024 * 1024 * 1024
        );
    }
}
