//! SD -> exe/output. The elevated worker streams bytes locally, never through JSON.
use crate::sd_batch::{Buffer, Disk, Entry, Plan, Progress, CHUNK};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::windows::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    time::Instant,
};

const REPARSE_POINT: u32 = 0x400;

pub(crate) fn output_directory() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let parent = exe.parent().ok_or("exeの保存先を取得できませんでした")?;
    Ok(parent
        .canonicalize()
        .map_err(|e| e.to_string())?
        .join("output"))
}

fn areas(csv: &str) -> Result<Vec<Entry>, String> {
    if !csv
        .lines()
        .any(|l| l.split(',').next().is_some_and(|f| f.trim() == "E"))
    {
        return Err("CSVの終端Eがありません".into());
    }
    let mut names = std::collections::HashSet::new();
    crate::sd_batch::parse_areas(csv)?
        .into_iter()
        .map(|(_, filename, address, size)| {
            let stem = filename
                .split('.')
                .next()
                .unwrap_or("")
                .trim_end_matches(' ')
                .to_ascii_uppercase();
            let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$")
                || (stem.starts_with("COM") || stem.starts_with("LPT"))
                    && matches!(
                        &stem[3..],
                        "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                    );
            if reserved
                || filename.ends_with(['.', ' '])
                || filename
                    .chars()
                    .any(|c| c.is_control() || "<>\"|?*".contains(c))
                || !names.insert(filename.to_uppercase())
            {
                return Err(format!(
                    "出力ファイル名が不正または重複しています: {filename}"
                ));
            }
            Ok(Entry {
                filename,
                path: String::new(),
                address,
                size,
                file_size: 0,
                modified: 0,
            })
        })
        .collect()
}

pub fn prepare(source: &str, drive: &str, run_id: &str) -> Result<Plan, String> {
    if run_id.is_empty() || run_id.len() > 100 {
        return Err("実行IDが不正です".into());
    }
    let (disk, capacity) = crate::windows_api::removable_target(drive)?;
    let output = output_directory()?;
    // Saving onto the source physical disk would change what is being backed up.
    let display = output.to_string_lossy();
    let local_path = display.trim_start_matches("\\\\?\\");
    if local_path.as_bytes().get(1) == Some(&b':') {
        let volume = format!("\\\\.\\{}", &local_path[..2]);
        if crate::windows_api::get_disk_extents(&volume).map_err(|e| e.to_string())? == disk {
            return Err("exeとoutputは読込元SD以外のディスクへ置いてください".into());
        }
    }
    let csv = crate::startup_csv_source(Some(source.into()))?
        .read()
        .map_err(|e| e.to_string())?;
    let mut entries = areas(&csv)?;
    if entries.is_empty() {
        return Err("CSVに読込対象の領域がありません".into());
    }
    // Validate a sorted copy, but export in CSV order.
    let mut sorted = entries.clone();
    sorted.sort_by_key(|e| e.address);
    let total_bytes = crate::sd_batch::validate_entries(&sorted, capacity)?;
    for entry in &mut entries {
        entry.path = output.join(&entry.filename).to_string_lossy().into_owned();
    }
    inspect_output(&output, &entries)?;
    Ok(Plan {
        run_id: run_id.into(),
        source: source.into(),
        directory: output.to_string_lossy().into_owned(),
        drive: drive.into(),
        disk,
        capacity,
        total_bytes,
        entries,
    })
}

// Never follow output junctions/symlinks or delete directories. Inspect every
// collision before deleting any file, including case-insensitive Windows names.
fn inspect_output(output: &Path, entries: &[Entry]) -> Result<Vec<PathBuf>, String> {
    let metadata = match fs::symlink_metadata(output) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("outputを確認できません: {e}")),
    };
    if !metadata.is_dir() || metadata.file_attributes() & REPARSE_POINT != 0 {
        return Err(
            "outputは通常のフォルダーである必要があります（リンクは使用できません）".into(),
        );
    }
    let mut files = Vec::new();
    for item in fs::read_dir(output).map_err(|e| format!("outputを確認できません: {e}"))? {
        let item = item.map_err(|e| e.to_string())?;
        let m = fs::symlink_metadata(item.path()).map_err(|e| e.to_string())?;
        if m.file_attributes() & REPARSE_POINT != 0 {
            return Err(format!(
                "output内にリンクがあります: {}",
                item.path().display()
            ));
        }
        if m.is_dir() {
            if entries.iter().any(|e| {
                item.file_name()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&e.filename)
            }) {
                return Err(format!(
                    "保存先に同名のサブフォルダーがあります: {}",
                    item.path().display()
                ));
            }
        } else if m.is_file() {
            if m.permissions().readonly() {
                return Err(format!(
                    "削除できない読み取り専用ファイルがあります: {}",
                    item.path().display()
                ));
            }
            files.push(item.path());
        } else {
            return Err(format!(
                "output内に通常でないファイルがあります: {}",
                item.path().display()
            ));
        }
    }
    Ok(files)
}

fn reset_output(output: &Path, entries: &[Entry]) -> Result<(), String> {
    let files = inspect_output(output, entries)?;
    fs::create_dir_all(output).map_err(|e| format!("outputを作成できません: {e}"))?;
    for file in files {
        crate::diagnostics::step(format!("output delete path={}", file.display()), || {
            fs::remove_file(&file).map_err(|e| format!("{} を削除できません: {e}", file.display()))
        })?;
    }
    Ok(())
}

fn export_entry(
    disk: &mut impl Disk,
    entry: &Entry,
    file: &mut impl Write,
    buffer: &mut [u8],
    mut progress: impl FnMut(u64) -> Result<(), String>,
) -> Result<(), String> {
    let sector = disk.sector();
    if !sector.is_power_of_two() || sector > 65536 || buffer.len() < CHUNK + sector {
        return Err("対応していないSDバッファ・セクターサイズです".into());
    }
    let mut copied = 0u64;
    while copied < entry.size {
        let address = entry.address + copied;
        let aligned = address / sector as u64 * sector as u64;
        let offset = (address - aligned) as usize;
        let len = (entry.size - copied).min(CHUNK as u64) as usize;
        let physical_len = (offset + len).div_ceil(sector) * sector;
        disk.read(aligned, &mut buffer[..physical_len])?;
        file.write_all(&buffer[offset..offset + len])
            .map_err(|e| format!("保存失敗: {e}"))?;
        copied += len as u64;
        progress(copied)?;
    }
    file.flush().map_err(|e| format!("保存確定失敗: {e}"))
}

pub fn execute(
    plan: Plan,
    mut notify: impl FnMut(Progress) -> Result<(), String>,
) -> Result<Progress, String> {
    let started = Instant::now();
    // Re-resolve CSV, SD and the fixed exe-relative output inside the worker.
    if prepare(&plan.source, &plan.drive, &plan.run_id)? != plan {
        return Err("確認後にCSV・SD・保存先が変更されました。再実行してください".into());
    }
    let mut disk = crate::windows_api::BatchDisk::open_read(&plan.drive, plan.disk, plan.capacity)?;
    let done = run_export(&plan, &mut disk, started, &mut notify)?;
    drop(disk);
    notify(done.clone())?;
    Ok(done)
}

fn run_export(
    plan: &Plan,
    disk: &mut impl Disk,
    started: Instant,
    notify: &mut impl FnMut(Progress) -> Result<(), String>,
) -> Result<Progress, String> {
    crate::diagnostics::phase("export");
    let mut buffer = Buffer::new(CHUNK + 65536)?;
    // Prove the SD is readable before clearing existing output files.
    let sector = disk.sector();
    let probe = plan.entries[0].address / sector as u64 * sector as u64;
    disk.read(probe, &mut buffer.bytes()[..sector])?;
    let mut event = Progress {
        run_id: plan.run_id.clone(),
        filename: String::new(),
        index: 0,
        total: plan.entries.len(),
        completed: 0,
        stage: "clean".into(),
        processed_bytes: 0,
        total_bytes: plan.total_bytes,
        elapsed_ms: 0,
        file_ms: 0,
        error: None,
    };
    notify(event.clone())?;
    crate::diagnostics::step("output reset", || {
        reset_output(Path::new(&plan.directory), &plan.entries)
    })?;
    for (i, entry) in plan.entries.iter().enumerate() {
        let file_started = Instant::now();
        event.filename = entry.filename.clone();
        event.index = i + 1;
        event.stage = "read".into();
        event.file_ms = 0;
        event.elapsed_ms = started.elapsed().as_millis() as u64;
        notify(event.clone())?;
        let base = event.processed_bytes;
        let mut last = Instant::now();
        let result = crate::diagnostics::step(
            format!(
                "output save filename={} path={}",
                entry.filename, entry.path
            ),
            || -> Result<(), String> {
                // create_new avoids following a file/link inserted after cleanup.
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .share_mode(0)
                    .open(&entry.path)
                    .map_err(|e| format!("{}: 保存先を開けません: {e}", entry.filename))?;
                export_entry(disk, entry, &mut file, buffer.bytes(), |copied| {
                    event.processed_bytes = base + copied;
                    if last.elapsed().as_millis() >= 150 {
                        event.elapsed_ms = started.elapsed().as_millis() as u64;
                        event.file_ms = file_started.elapsed().as_millis() as u64;
                        notify(event.clone())?;
                        last = Instant::now();
                    }
                    Ok(())
                })?;
                file.sync_all()
                    .map_err(|e| format!("{}: 保存確定失敗: {e}", entry.filename))?;
                Ok(())
            },
        );
        event.file_ms = file_started.elapsed().as_millis() as u64;
        event.elapsed_ms = started.elapsed().as_millis() as u64;
        event.error = result.as_ref().err().cloned();
        event.stage = if result.is_ok() {
            "file_done"
        } else {
            "failed"
        }
        .into();
        if result.is_ok() {
            event.completed = i + 1;
        }
        notify(event.clone())?;
        result?;
    }
    event.filename.clear();
    event.stage = "done".into();
    event.file_ms = 0;
    event.elapsed_ms = started.elapsed().as_millis() as u64;
    Ok(event)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct MemoryDisk {
        bytes: Vec<u8>,
        fail: bool,
        fail_at: Option<u64>,
    }
    impl Disk for MemoryDisk {
        fn sector(&self) -> usize {
            512
        }
        fn read(&mut self, address: u64, data: &mut [u8]) -> Result<(), String> {
            if self.fail || self.fail_at == Some(address) {
                return Err("injected read failure".into());
            }
            assert_eq!(address % 512, 0);
            assert_eq!(data.len() % 512, 0);
            data.copy_from_slice(&self.bytes[address as usize..address as usize + data.len()]);
            Ok(())
        }
        fn write(&mut self, _: u64, _: &[u8]) -> Result<(), String> {
            panic!("SD must never be written")
        }
        fn flush(&mut self) -> Result<(), String> {
            panic!("SD must never be flushed")
        }
    }
    #[test]
    fn exports_exact_unaligned_multichunk_bytes_without_writing_sd_and_stops_on_error() {
        let mut disk = MemoryDisk {
            bytes: (0..CHUNK + 2048).map(|i| (i % 251) as u8).collect(),
            fail: false,
            fail_at: None,
        };
        let mut entry = areas("N,test.bin,00000003,00400322\nE,,,\n")
            .unwrap()
            .remove(0);
        let mut buffer = Buffer::new(CHUNK + 65536).unwrap();
        let mut saved = Vec::new();
        let mut progress = Vec::new();
        export_entry(&mut disk, &entry, &mut saved, buffer.bytes(), |n| {
            progress.push(n);
            Ok(())
        })
        .unwrap();
        assert_eq!(saved, disk.bytes[3..3 + entry.size as usize]);
        assert_eq!(progress.len(), 2);
        assert_eq!(progress.last(), Some(&entry.size));
        disk.fail = true;
        saved.clear();
        entry.size = 10;
        assert!(export_entry(&mut disk, &entry, &mut saved, buffer.bytes(), |_| Ok(())).is_err());
        assert!(saved.is_empty());
        struct FailedSave;
        impl Write for FailedSave {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("disk full"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        disk.fail = false;
        assert!(export_entry(
            &mut disk,
            &entry,
            &mut FailedSave,
            buffer.bytes(),
            |_| panic!("no progress on failed save")
        )
        .is_err());
    }
    #[test]
    fn all_flags_csv_order_end_and_windows_names() {
        let e = areas("N,n,200,3FF\n0,z,0,1FF\nW,w,400,5FF\nR,r,600,7FF\nA,a,800,9FF\nE,,,\nA,ignored,A00,BFF").unwrap();
        assert_eq!(
            e.iter().map(|e| e.filename.as_str()).collect::<Vec<_>>(),
            ["n", "z", "w", "r", "a"]
        );
        for csv in [
            "A,a,0,1",
            "A,../x,0,1\nE,,,",
            "A,CON.txt,0,1\nE,,,",
            "A,a.,0,1\nE,,,",
            "A,a,0,1\nA,A,2,3\nE,,,",
        ] {
            assert!(areas(csv).is_err(), "{csv}");
        }
    }
    #[test]
    fn batch_preflight_preserves_old_files_and_export_counts_only_saved_files() {
        let root = std::env::temp_dir().join(format!(
            "sddirect-read-batch-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let output = root.join("output");
        fs::create_dir_all(&output).unwrap();
        fs::write(output.join("old"), b"old backup").unwrap();
        let mut entries = areas("N,one,0,1FF\n0,two,200,3FF\nR,three,400,5FF\nE,,,\n").unwrap();
        for entry in &mut entries {
            entry.path = output.join(&entry.filename).to_string_lossy().into_owned();
        }
        let plan = Plan {
            run_id: "test".into(),
            source: String::new(),
            directory: output.to_string_lossy().into_owned(),
            drive: "E:".into(),
            disk: 0,
            capacity: 1536,
            total_bytes: 1536,
            entries,
        };
        let mut disk = MemoryDisk {
            bytes: (0..1536).map(|n| (n % 251) as u8).collect(),
            fail: true,
            fail_at: None,
        };
        let mut events = Vec::new();
        assert!(run_export(&plan, &mut disk, Instant::now(), &mut |p| {
            events.push(p);
            Ok(())
        })
        .is_err());
        assert_eq!(fs::read(output.join("old")).unwrap(), b"old backup");
        assert!(events.is_empty());
        disk.fail = false;
        disk.fail_at = Some(512);
        assert!(run_export(&plan, &mut disk, Instant::now(), &mut |p| {
            events.push(p);
            Ok(())
        })
        .is_err());
        assert!(!output.join("old").exists());
        assert_eq!(fs::read(output.join("one")).unwrap(), disk.bytes[..512]);
        assert_eq!(fs::metadata(output.join("two")).unwrap().len(), 0);
        assert!(!output.join("three").exists());
        let failed = events.last().unwrap();
        assert_eq!(
            (failed.stage.as_str(), failed.completed, failed.index),
            ("failed", 1, 2)
        );
        assert!(events.iter().all(|e| e.stage != "done"));
        disk.fail_at = None;
        events.clear();
        let done = run_export(&plan, &mut disk, Instant::now(), &mut |p| {
            events.push(p);
            Ok(())
        })
        .unwrap();
        assert_eq!(
            (done.stage.as_str(), done.completed, done.processed_bytes),
            ("done", 3, 1536)
        );
        assert_eq!(
            events
                .iter()
                .filter(|e| e.stage == "file_done")
                .map(|e| e.filename.as_str())
                .collect::<Vec<_>>(),
            ["one", "two", "three"]
        );
        for (i, name) in ["one", "two", "three"].iter().enumerate() {
            assert_eq!(
                fs::read(output.join(name)).unwrap(),
                disk.bytes[i * 512..(i + 1) * 512]
            );
            fs::remove_file(output.join(name)).unwrap();
        }
        fs::remove_dir(output).unwrap();
        fs::remove_dir(root).unwrap();
    }
    #[test]
    fn output_reset_creates_directory_deletes_only_direct_files_and_preserves_subfolders() {
        let root = std::env::temp_dir().join(format!(
            "sddirect-read-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let output = root.join("output");
        reset_output(&output, &[]).unwrap();
        fs::write(root.join("outside"), b"keep").unwrap();
        fs::write(output.join("old"), b"delete").unwrap();
        fs::create_dir(output.join("sub")).unwrap();
        fs::write(output.join("sub/keep"), b"keep").unwrap();
        let conflict = areas("A,SUB,0,1\nE,,,\n").unwrap();
        assert!(reset_output(&output, &conflict).is_err());
        assert!(output.join("old").exists());
        reset_output(&output, &[]).unwrap();
        assert!(!output.join("old").exists());
        assert_eq!(fs::read(output.join("sub/keep")).unwrap(), b"keep");
        assert_eq!(fs::read(root.join("outside")).unwrap(), b"keep");
        fs::remove_file(output.join("sub/keep")).unwrap();
        fs::remove_dir(output.join("sub")).unwrap();
        fs::remove_dir(output).unwrap();
        fs::remove_file(root.join("outside")).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
