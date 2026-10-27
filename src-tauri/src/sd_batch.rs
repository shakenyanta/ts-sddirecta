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
    total.checked_mul(3).ok_or("総処理量が大きすぎます")?;
    Ok(total)
}

pub fn prepare(source: &str, drive: &str, run_id: &str) -> Result<Plan, String> {
    crate::diagnostics::step(
        format!("prepare run_id={run_id} source={source} drive={drive}"),
        || {
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
        },
    )
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

/// Each file is fully written, flushed, then verified using three independent buffers.
#[cfg(test)]
fn transfer(
    disk: &mut impl Disk,
    source: &mut (impl Read + Seek),
    entry: &Entry,
    write_buffer: &mut [u8],
    read_buffer: &mut [u8],
    compare: &mut [u8],
    progress: impl FnMut(&str, u64) -> Result<(), String>,
) -> Result<(), String> {
    transfer_phases(
        disk,
        source,
        entry,
        write_buffer,
        read_buffer,
        compare,
        &[false, true],
        progress,
    )
}

fn transfer_phases(
    disk: &mut impl Disk,
    source: &mut (impl Read + Seek),
    entry: &Entry,
    write_buffer: &mut [u8],
    read_buffer: &mut [u8],
    compare: &mut [u8],
    phases: &[bool],
    mut progress: impl FnMut(&str, u64) -> Result<(), String>,
) -> Result<(), String> {
    let sector = disk.sector();
    if !sector.is_power_of_two()
        || sector > 65536
        || write_buffer.len() < CHUNK + sector
        || read_buffer.len() < CHUNK + sector
        || compare.len() < CHUNK
    {
        return Err("セクターサイズが不正です".into());
    }
    for &verify in phases {
        let stage = if verify { "verify" } else { "write" };
        crate::diagnostics::phase(stage);
        progress(stage, 0)?;
        let mut offset = 0u64;
        while offset < entry.size {
            let address = entry.address + offset;
            let aligned = address / sector as u64 * sector as u64;
            let prefix = (address - aligned) as usize;
            // End each non-final chunk on a sector boundary.
            let count = (entry.size - offset).min((CHUNK - prefix) as u64) as usize;
            let physical_len = (prefix + count).div_ceil(sector) * sector;
            if verify {
                expected(source, entry, offset, &mut compare[..count])?;
                let buffer = &mut read_buffer[..physical_len];
                // An incomplete/no-op read must not match stale contents, even
                // when the expected data is all zeroes or all 0xFF.
                for (dest, expected) in buffer[prefix..prefix + count]
                    .iter_mut()
                    .zip(&compare[..count])
                {
                    *dest = !expected;
                }
                disk.read(aligned, buffer)?;
                let payload = &buffer[prefix..prefix + count];
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
                let buffer = &mut write_buffer[..physical_len];
                if prefix > 0 || prefix + count != physical_len {
                    // Preserve neighboring bytes without using the write
                    // buffer as the destination of any SD read.
                    disk.read(aligned, &mut read_buffer[..physical_len])?;
                    buffer.copy_from_slice(&read_buffer[..physical_len]);
                }
                let payload = &mut buffer[prefix..prefix + count];
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
    crate::diagnostics::step(
        format!(
            "batch execute run_id={} drive={} files={} bytes={}",
            plan.run_id,
            plan.drive,
            plan.entries.len(),
            plan.total_bytes
        ),
        || {
            let started = Instant::now();
            notify(Progress {
                run_id: plan.run_id.clone(),
                filename: String::new(),
                index: 0,
                total: plan.entries.len(),
                completed: 0,
                stage: "prepare".into(),
                processed_bytes: 0,
                total_bytes: plan.total_bytes * 3,
                elapsed_ms: 0,
                file_ms: 0,
                error: None,
            })?;
            // Validate all sources and pin them before touching the SD. Changed or missing
            // sources abort the entire batch, even if another file could be written.
            let mut files = Vec::new();
            for entry in &plan.entries {
                let file = crate::diagnostics::step(
                    format!(
                        "source validation filename={} path={}",
                        entry.filename, entry.path
                    ),
                    || source_file(Path::new(&entry.path)),
                )?;
                if stamp(&file)? != (entry.file_size, entry.modified) {
                    return Err(format!(
                        "{}: 確認後にファイルが変更されました",
                        entry.filename
                    ));
                }
                files.push(file);
            }
            let mut disk =
                crate::windows_api::BatchDisk::open(&plan.drive, plan.disk, plan.capacity)?;
            if validate_entries(&plan.entries, disk.capacity)? != plan.total_bytes {
                return Err("総書込量が一致しません".into());
            }
            let done = run_entries(&plan, &mut files, &mut disk, started, &mut notify)?;
            drop(disk); // Unlock/close before announcing normal completion.
            notify(done.clone())?;
            Ok(done)
        },
    )
}

// Shared with fault-injection tests so final verification and completion rules
// are exercised without touching a physical disk.
pub(crate) fn run_entries(
    plan: &Plan,
    files: &mut [impl Read + Seek],
    disk: &mut impl Disk,
    started: Instant,
    notify: &mut impl FnMut(Progress) -> Result<(), String>,
) -> Result<Progress, String> {
    if files.len() != plan.entries.len() {
        return Err("入力ファイル数が一致しません".into());
    }
    let mut write_buffer = Buffer::new(CHUNK + 65536)?;
    let mut read_buffer = Buffer::new(CHUNK + 65536)?;
    let mut compare = Buffer::new(CHUNK)?;
    let mut processed = 0;
    for final_pass in [false, true] {
        for (i, (entry, source)) in plan.entries.iter().zip(files.iter_mut()).enumerate() {
            let file_started = Instant::now();
            let mut last = Instant::now();
            let mut last_stage = String::new();
            let mut confirmed = 0;
            crate::diagnostics::record("INFO", &format!("file start final_pass={final_pass} index={} filename={} address={:#X} bytes={}", i + 1, entry.filename, entry.address, entry.size));
            let result = transfer_phases(
                disk,
                source,
                entry,
                write_buffer.bytes(),
                read_buffer.bytes(),
                compare.bytes(),
                if final_pass { &[true] } else { &[false, true] },
                |stage, bytes| {
                    if final_pass {
                        crate::diagnostics::phase("final_verify");
                    }
                    confirmed = bytes
                        + if !final_pass && stage == "verify" {
                            entry.size
                        } else {
                            0
                        };
                    let stage = if final_pass { "final_verify" } else { stage };
                    if stage != last_stage || last.elapsed().as_millis() >= 150 {
                        notify(Progress {
                            run_id: plan.run_id.clone(),
                            filename: entry.filename.clone(),
                            index: i + 1,
                            total: plan.entries.len(),
                            completed: if final_pass { i } else { 0 },
                            stage: stage.into(),
                            processed_bytes: processed + confirmed,
                            total_bytes: plan.total_bytes * 3,
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
                completed: if final_pass {
                    i + usize::from(error.is_none())
                } else {
                    0
                },
                stage: match (final_pass, error.is_none()) {
                    (true, true) => "final_file_done",
                    (true, false) => "final_failed",
                    (false, true) => "file_done",
                    (false, false) => "failed",
                }
                .into(),
                processed_bytes: processed + confirmed,
                total_bytes: plan.total_bytes * 3,
                elapsed_ms: started.elapsed().as_millis() as u64,
                file_ms: file_started.elapsed().as_millis() as u64,
                error,
            })?;
            result?;
            processed += entry.size * if final_pass { 1 } else { 2 };
        }
    }
    Ok(Progress {
        run_id: plan.run_id.clone(),
        filename: String::new(),
        index: plan.entries.len(),
        total: plan.entries.len(),
        completed: plan.entries.len(),
        stage: "done".into(),
        processed_bytes: processed,
        total_bytes: plan.total_bytes * 3,
        elapsed_ms: started.elapsed().as_millis() as u64,
        file_ms: 0,
        error: None,
    })
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
            let mut write_buffer = Buffer::new(CHUNK + 65536).unwrap();
            let mut read_buffer = Buffer::new(CHUNK + 65536).unwrap();
            let mut compare = Buffer::new(CHUNK).unwrap();
            transfer(
                &mut disk,
                &mut source,
                &e,
                write_buffer.bytes(),
                read_buffer.bytes(),
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
        let mut write_buffer = Buffer::new(CHUNK + 65536).unwrap();
        let mut read_buffer = Buffer::new(CHUNK + 65536).unwrap();
        let mut compare = Buffer::new(CHUNK).unwrap();
        let mut events = vec![];
        transfer(
            &mut disk,
            &mut Cursor::new(vec![0xAA; size]),
            &e,
            write_buffer.bytes(),
            read_buffer.bytes(),
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
            write_buffer.bytes(),
            read_buffer.bytes(),
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

    // Fault injection for the verification audit. This never accesses a real SD.
    #[derive(Clone, Copy)]
    enum AuditFault {
        None,
        Corrupt(usize),
        ReadError,
        WriteError,
        FlushError,
        IgnoreWrites,
        Alias,
        // Intentionally violates the read contract, modelling a lying I/O layer.
        NoOpIo,
        PartialRead,
    }

    struct AuditDisk {
        data: Vec<u8>,
        fault: AuditFault,
        flushed: bool,
        events: Vec<(&'static str, u64, usize)>,
        read_memory: Vec<(usize, usize)>,
        write_memory: Vec<(usize, usize)>,
    }

    impl AuditDisk {
        fn new(size: usize, fault: AuditFault) -> Self {
            Self {
                data: vec![0x33; size],
                fault,
                flushed: false,
                events: vec![],
                read_memory: vec![],
                write_memory: vec![],
            }
        }
    }

    impl Disk for AuditDisk {
        fn sector(&self) -> usize {
            512
        }

        fn read(&mut self, address: u64, dest: &mut [u8]) -> Result<(), String> {
            self.events.push(("read", address, dest.len()));
            self.read_memory.push((dest.as_ptr() as usize, dest.len()));
            if matches!(self.fault, AuditFault::NoOpIo) {
                return Ok(());
            }
            if self.flushed && matches!(self.fault, AuditFault::ReadError) {
                return Err("injected read error".into());
            }
            let offset = if matches!(self.fault, AuditFault::Alias) {
                address as usize % 512
            } else {
                address as usize
            };
            let copied = if self.flushed && matches!(self.fault, AuditFault::PartialRead) {
                dest.len() / 2
            } else {
                dest.len()
            };
            dest[..copied].copy_from_slice(&self.data[offset..offset + copied]);
            if self.flushed {
                if let AuditFault::Corrupt(at) = self.fault {
                    if (address as usize..address as usize + dest.len()).contains(&at) {
                        dest[at - address as usize] ^= 1;
                    }
                }
            }
            Ok(())
        }

        fn write(&mut self, address: u64, data: &[u8]) -> Result<(), String> {
            self.events.push(("write", address, data.len()));
            self.write_memory.push((data.as_ptr() as usize, data.len()));
            self.flushed = false;
            if matches!(self.fault, AuditFault::WriteError) {
                return Err("injected write error".into());
            }
            if matches!(self.fault, AuditFault::IgnoreWrites | AuditFault::NoOpIo) {
                return Ok(());
            }
            let offset = if matches!(self.fault, AuditFault::Alias) {
                address as usize % 512
            } else {
                address as usize
            };
            self.data[offset..offset + data.len()].copy_from_slice(data);
            Ok(())
        }

        fn flush(&mut self) -> Result<(), String> {
            self.events.push(("flush", 0, 0));
            if matches!(self.fault, AuditFault::FlushError) {
                return Err("injected flush error".into());
            }
            self.flushed = true;
            Ok(())
        }
    }

    fn audit_transfer(disk: &mut AuditDisk, e: &Entry, contents: Vec<u8>) -> Result<(), String> {
        let mut write_buffer = Buffer::new(CHUNK + 65536)?;
        let mut read_buffer = Buffer::new(CHUNK + 65536)?;
        let mut compare = Buffer::new(CHUNK)?;
        assert_ne!(write_buffer.bytes().as_ptr(), compare.bytes().as_ptr());
        assert_ne!(write_buffer.bytes().as_ptr(), read_buffer.bytes().as_ptr());
        assert_ne!(read_buffer.bytes().as_ptr(), compare.bytes().as_ptr());
        transfer(
            disk,
            &mut Cursor::new(contents),
            e,
            write_buffer.bytes(),
            read_buffer.bytes(),
            compare.bytes(),
            |_, _| Ok(()),
        )
    }

    #[test]
    fn audit_verification_reads_all_chunks_after_flush() {
        let address = 17;
        let size = CHUNK + 777;
        let file_size = CHUNK + 531;
        let contents: Vec<u8> = (0..file_size)
            .map(|i| ((i * 131) ^ (i >> 9)) as u8)
            .collect();
        let mut disk = AuditDisk::new(size + 2048, AuditFault::None);
        audit_transfer(
            &mut disk,
            &entry(address, size as u64, file_size as u64),
            contents.clone(),
        )
        .unwrap();
        let flush = disk.events.iter().position(|e| e.0 == "flush").unwrap();
        let mut covered = address as usize;
        let end = address as usize + size;
        for &(stage, start, len) in &disk.events[flush + 1..] {
            assert_eq!(stage, "read");
            assert!(start as usize <= covered);
            covered = (start as usize + len).min(end);
        }
        assert_eq!(covered, end);
        assert_eq!(&disk.data[17..17 + file_size], contents.as_slice());
        assert!(disk.data[17 + file_size..end].iter().all(|b| *b == 0xFF));
        // Includes the initial read that preserves unaligned neighboring bytes.
        for &(read, read_len) in &disk.read_memory {
            for &(write, write_len) in &disk.write_memory {
                assert!(read + read_len <= write || write + write_len <= read);
            }
        }
    }

    #[test]
    fn audit_detects_first_boundary_last_and_padding_byte_corruption() {
        let address = 17;
        let size = CHUNK + 777;
        for at in [address, CHUNK - 1, CHUNK, address + size - 1] {
            let mut disk = AuditDisk::new(size + 2048, AuditFault::Corrupt(at));
            let error = audit_transfer(
                &mut disk,
                &entry(address as u64, size as u64, (size - 100) as u64),
                vec![0xA5; size - 100],
            )
            .unwrap_err();
            assert_eq!(error, format!("照合不一致: アドレス 0x{at:X}"));
        }
    }

    #[test]
    fn audit_rejects_io_errors_and_silently_ignored_writes() {
        for (fault, expected) in [
            (AuditFault::ReadError, "injected read error"),
            (AuditFault::WriteError, "injected write error"),
            (AuditFault::FlushError, "injected flush error"),
            (AuditFault::IgnoreWrites, "照合不一致"),
        ] {
            let mut disk = AuditDisk::new(512, fault);
            assert!(
                audit_transfer(&mut disk, &entry(0, 512, 512), vec![0xA5; 512])
                    .unwrap_err()
                    .contains(expected)
            );
        }
    }

    #[test]
    fn audit_documents_cross_file_aliasing_blind_spot() {
        let mut disk = AuditDisk::new(512, AuditFault::Alias);
        // Two different logical sectors map to the same physical storage.
        // Each immediate verification passes, but the second write destroys the first.
        audit_transfer(&mut disk, &entry(0, 512, 512), vec![0xA1; 512]).unwrap();
        audit_transfer(&mut disk, &entry(512, 512, 512), vec![0xB2; 512]).unwrap();
        let mut reread = vec![0; 512];
        disk.read(0, &mut reread).unwrap();
        assert_eq!(reread, vec![0xB2; 512]);
        assert_ne!(reread, vec![0xA1; 512]);
    }

    fn audit_plan() -> Plan {
        let mut entries = vec![entry(0, 512, 512), entry(512, 512, 512)];
        entries[0].filename = "first".into();
        entries[1].filename = "second".into();
        Plan {
            run_id: "audit-final".into(),
            source: String::new(),
            directory: String::new(),
            drive: "H:".into(),
            disk: 1,
            capacity: 1024,
            total_bytes: 1024,
            entries,
        }
    }

    #[test]
    fn final_verification_detects_cross_file_aliasing_before_success() {
        let plan = audit_plan();
        let mut disk = AuditDisk::new(512, AuditFault::Alias);
        let mut files = [Cursor::new(vec![0xA1; 512]), Cursor::new(vec![0xB2; 512])];
        let mut events = vec![];
        let error = run_entries(&plan, &mut files, &mut disk, Instant::now(), &mut |event| {
            events.push(event);
            Ok(())
        })
        .unwrap_err();
        assert!(error.contains("照合不一致: アドレス 0x0"));
        assert_eq!(events.iter().filter(|e| e.stage == "file_done").count(), 2);
        assert!(events.iter().all(|e| e.stage != "done" && e.completed == 0));
        let failed = events.last().unwrap();
        assert_eq!(failed.stage, "final_failed");
        assert_eq!(failed.filename, "first");
        assert_eq!(failed.total_bytes, 3072);
    }

    #[test]
    fn final_verification_reads_all_entries_without_additional_writes() {
        let plan = audit_plan();
        let mut disk = AuditDisk::new(1024, AuditFault::None);
        let mut files = [Cursor::new(vec![0xA1; 512]), Cursor::new(vec![0xB2; 512])];
        let mut events = vec![];
        let done = run_entries(&plan, &mut files, &mut disk, Instant::now(), &mut |event| {
            events.push(event);
            Ok(())
        })
        .unwrap();
        assert_eq!(done.stage, "done");
        assert_eq!(done.completed, 2);
        assert_eq!(done.processed_bytes, 3072);
        assert_eq!(done.total_bytes, 3072);
        let read_addresses: Vec<_> = disk
            .events
            .iter()
            .filter(|e| e.0 == "read")
            .map(|e| e.1)
            .collect();
        assert_eq!(read_addresses, vec![0, 512, 0, 512]);
        assert_eq!(disk.events.iter().filter(|e| e.0 == "write").count(), 2);
        assert!(events
            .iter()
            .filter(|e| e.stage == "file_done")
            .all(|e| e.completed == 0));
        assert_eq!(
            events
                .iter()
                .filter(|e| e.stage == "final_file_done")
                .map(|e| e.completed)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
    }

    #[test]
    fn audit_rejects_stale_buffer_if_read_lies_about_success() {
        // Also exercise values matching a newly allocated zero buffer and 0xFF
        // padding, plus partial reads that falsely report full success.
        for value in [0x00, 0xFF, 0xA5] {
            for fault in [AuditFault::NoOpIo, AuditFault::PartialRead] {
                let mut disk = AuditDisk::new(512, fault);
                let error =
                    audit_transfer(&mut disk, &entry(0, 512, 512), vec![value; 512]).unwrap_err();
                let address = if matches!(fault, AuditFault::PartialRead) {
                    256
                } else {
                    0
                };
                assert_eq!(error, format!("照合不一致: アドレス 0x{address:X}"));
            }
        }
    }

    #[test]
    fn audit_same_contents_cannot_prove_new_writes_were_persisted() {
        let mut disk = AuditDisk::new(512, AuditFault::IgnoreWrites);
        // Readback equality alone cannot distinguish a successful rewrite from
        // ignored writes when the old contents already equal the expected data.
        audit_transfer(&mut disk, &entry(0, 512, 512), vec![0x33; 512]).unwrap();
    }

    #[test]
    fn input_read_failure_and_lost_parent_stop_before_writing() {
        let mut disk = MemoryDisk {
            data: vec![0x33; 2048],
            corrupt: false,
        };
        let e = entry(0, 512, 512);
        let mut write_buffer = Buffer::new(CHUNK + 65536).unwrap();
        let mut read_buffer = Buffer::new(CHUNK + 65536).unwrap();
        let mut compare = Buffer::new(CHUNK).unwrap();
        assert!(transfer(
            &mut disk,
            &mut Cursor::new(vec![0xAA; 128]),
            &e,
            write_buffer.bytes(),
            read_buffer.bytes(),
            compare.bytes(),
            |_, _| Ok(())
        )
        .is_err());
        assert!(disk.data.iter().all(|b| *b == 0x33));
        assert!(transfer(
            &mut disk,
            &mut Cursor::new(vec![0xAA; 512]),
            &e,
            write_buffer.bytes(),
            read_buffer.bytes(),
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
