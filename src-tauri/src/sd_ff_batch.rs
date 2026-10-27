//! CSV-defined FF initialization, including address zero when present in the CSV.
use crate::sd_batch::{self, Disk, Entry, Plan, Progress};
use std::{io::Cursor, time::Instant};

fn entries(csv: &str, capacity: u64) -> Result<(Vec<Entry>, u64), String> {
    if !csv.lines().any(|line| {
        line.split(',')
            .next()
            .is_some_and(|flag| flag.trim() == "E")
    }) {
        return Err("CSVの終端Eがありません".into());
    }
    let entries: Vec<_> = sd_batch::parse_areas(csv)?
        .into_iter()
        .map(|(_, filename, address, size)| Entry {
            filename,
            address,
            size,
            path: String::new(),
            file_size: 0,
            modified: 0,
        })
        .collect();
    if entries.is_empty() {
        return Err("CSVにFF初期化対象の領域がありません".into());
    }
    // Validate overlap/capacity on a copy, keeping the original CSV order.
    let mut sorted = entries.clone();
    sorted.sort_by_key(|entry| entry.address);
    let total = sd_batch::validate_entries(&sorted, capacity)?;
    Ok((entries, total))
}

pub fn prepare(source: &str, drive: &str, run_id: &str) -> Result<Plan, String> {
    crate::diagnostics::step(
        format!("FF prepare run_id={run_id} source={source} drive={drive}"),
        || {
            if run_id.is_empty() || run_id.len() > 100 {
                return Err("実行IDが不正です".into());
            }
            let (disk, capacity) = crate::windows_api::removable_target(drive)?;
            // The program must not erase the disk it is running from.
            let exe = std::env::current_exe()
                .map_err(|e| e.to_string())?
                .canonicalize()
                .map_err(|e| e.to_string())?;
            let display = exe.to_string_lossy();
            let local = display.trim_start_matches("\\\\?\\");
            if local.as_bytes().get(1) == Some(&b':')
                && crate::windows_api::get_disk_extents(&format!("\\\\.\\{}", &local[..2]))
                    .map_err(|e| e.to_string())?
                    == disk
            {
                return Err("exeは初期化対象SD以外のディスクへ置いてください".into());
            }
            let csv = crate::startup_csv_source(Some(source.into()))?
                .read()
                .map_err(|e| e.to_string())?;
            let (entries, total_bytes) = entries(&csv, capacity)?;
            Ok(Plan {
                run_id: run_id.into(),
                source: source.into(),
                directory: String::new(),
                drive: drive.into(),
                disk,
                capacity,
                total_bytes,
                entries,
            })
        },
    )
}

pub fn execute(
    plan: Plan,
    mut notify: impl FnMut(Progress) -> Result<(), String>,
) -> Result<Progress, String> {
    crate::diagnostics::step(
        format!(
            "FF batch execute run_id={} drive={} areas={} bytes={}",
            plan.run_id,
            plan.drive,
            plan.entries.len(),
            plan.total_bytes
        ),
        || {
            let started = Instant::now();
            // All validation happens before any write, including the worker-side
            // recheck after UAC. Once address zero is erased, never reopen by letter.
            if prepare(&plan.source, &plan.drive, &plan.run_id)? != plan {
                return Err("確認後にCSV・SDが変更されました。FF初期化を再実行してください".into());
            }
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
            let mut disk =
                crate::windows_api::BatchDisk::open(&plan.drive, plan.disk, plan.capacity)?;
            let done = run_entries(&plan, &mut disk, started, &mut notify)?;
            drop(disk); // Close and unlock before reporting normal completion.
            notify(done.clone())?;
            Ok(done)
        },
    )
}

fn run_entries(
    plan: &Plan,
    disk: &mut impl Disk,
    started: Instant,
    notify: &mut impl FnMut(Progress) -> Result<(), String>,
) -> Result<Progress, String> {
    if plan.entries.is_empty()
        || plan
            .entries
            .iter()
            .any(|e| e.file_size != 0 || !e.path.is_empty())
    {
        return Err("FF初期化の領域計画が不正です".into());
    }
    let mut sorted = plan.entries.clone();
    sorted.sort_by_key(|e| e.address);
    if sd_batch::validate_entries(&sorted, plan.capacity)? != plan.total_bytes {
        return Err("FF初期化の総サイズが一致しません".into());
    }
    // No input files are created or opened. A zero-length source causes the
    // shared engine to generate FF for every expected byte, with the same
    // aligned I/O, neighbor preservation, flush and two verification passes.
    let mut sources: Vec<_> = plan.entries.iter().map(|_| Cursor::new([])).collect();
    sd_batch::run_entries(plan, &mut sources, disk, started, notify)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sd_batch::CHUNK;

    #[derive(Clone, Copy)]
    enum Fault {
        None,
        Write,
        Read,
        NoOpRead,
        Verify,
        FinalVerify,
    }
    struct MemoryDisk {
        bytes: Vec<u8>,
        fault: Fault,
        flushes: usize,
        writes: Vec<(usize, usize)>,
    }
    impl Disk for MemoryDisk {
        fn sector(&self) -> usize {
            512
        }
        fn read(&mut self, address: u64, data: &mut [u8]) -> Result<(), String> {
            assert_eq!(address % 512, 0);
            assert_eq!(data.len() % 512, 0);
            if matches!(self.fault, Fault::Read) {
                return Err("injected short/read failure".into());
            }
            if matches!(self.fault, Fault::NoOpRead) {
                return Ok(());
            }
            let at = address as usize;
            data.copy_from_slice(&self.bytes[at..at + data.len()]);
            if at == 0
                && (matches!(self.fault, Fault::Verify) && self.flushes > 0
                    || matches!(self.fault, Fault::FinalVerify) && self.flushes == 2)
            {
                data[0] ^= 1;
            }
            Ok(())
        }
        fn write(&mut self, address: u64, data: &[u8]) -> Result<(), String> {
            if matches!(self.fault, Fault::Write) {
                return Err("injected write failure".into());
            }
            let at = address as usize;
            assert_eq!(at % 512, 0);
            assert_eq!(data.len() % 512, 0);
            self.bytes[at..at + data.len()].copy_from_slice(data);
            self.writes.push((at, data.len()));
            Ok(())
        }
        fn flush(&mut self) -> Result<(), String> {
            self.flushes += 1;
            Ok(())
        }
    }
    fn plan(csv: &str, capacity: usize) -> Plan {
        let (entries, total_bytes) = entries(csv, capacity as u64).unwrap();
        Plan {
            run_id: "ff-test".into(),
            source: String::new(),
            directory: String::new(),
            drive: "H:".into(),
            disk: 1,
            capacity: capacity as u64,
            total_bytes,
            entries,
        }
    }
    #[test]
    fn includes_zero_first_32k_all_flags_and_csv_order_and_rejects_invalid_plans() {
        let (e, total) = entries("N,later,9000,91FF\n0,first,0,7FFF\nR,r,9200,93FF\nW,w,9400,95FF\nA,a,9600,97FF\nE,,,\nA,ignored,0,0", 0x10000).unwrap();
        assert_eq!(e.len(), 5);
        assert_eq!(e[0].filename, "later");
        assert_eq!((e[1].address, e[1].size), (0, 32768));
        assert_eq!(total, 32768 + 4 * 512);
        assert!(e.iter().all(|e| e.path.is_empty() && e.file_size == 0));
        for csv in [
            "A,a,0,1",
            "E,,,",
            "A,a,1,0\nE,,,",
            "A,a,0,1FFFF\nE,,,",
            "A,a,0,100\nN,b,100,200\nE,,,",
        ] {
            assert!(entries(csv, 0x10000).is_err(), "{csv}");
        }
        let (e, total) = entries(
            include_str!("../../config/sdDirectaxs.csv"),
            256 * 1024 * 1024,
        )
        .unwrap();
        assert_eq!(e.len(), 94);
        assert_eq!(total, 40_611_328);
        assert_eq!((e[0].address, e[0].size), (0, 32768));
    }
    #[test]
    fn ff_fills_first_32k_and_multiple_chunks_preserves_gaps_and_finishes_only_after_final_verify()
    {
        let size = CHUNK + 0x10000;
        let plan = plan("A,head,0,7FFF\nN,unaligned,9003,409027\nE,,,", size);
        let mut disk = MemoryDisk {
            bytes: vec![0x5A; size],
            fault: Fault::None,
            flushes: 0,
            writes: vec![],
        };
        let mut events = vec![];
        let done = run_entries(&plan, &mut disk, Instant::now(), &mut |p| {
            events.push(p);
            Ok(())
        })
        .unwrap();
        for (at, byte) in disk.bytes.iter().enumerate() {
            let target = plan
                .entries
                .iter()
                .any(|e| (at as u64) >= e.address && (at as u64) < e.address + e.size);
            assert_eq!(*byte, if target { 0xFF } else { 0x5A }, "{at:#X}");
        }
        assert_eq!(disk.flushes, 2);
        assert!(disk.writes.len() >= 3);
        assert_eq!(done.stage, "done");
        assert_eq!(done.completed, 2);
        assert_eq!(done.processed_bytes, plan.total_bytes * 3);
        assert_eq!(
            events
                .iter()
                .filter(|e| e.stage == "final_file_done")
                .count(),
            2
        );
        assert!(events
            .iter()
            .filter(|e| e.stage == "file_done")
            .all(|e| e.completed == 0));
    }
    #[test]
    fn write_read_and_both_verification_failures_stop_without_normal_completion() {
        let plan = plan("A,head,0,7FFF\n0,next,9000,91FF\nE,,,", 0x10000);
        for fault in [
            Fault::Write,
            Fault::Read,
            Fault::NoOpRead,
            Fault::Verify,
            Fault::FinalVerify,
        ] {
            let mut disk = MemoryDisk {
                bytes: vec![0x5A; 0x10000],
                fault,
                flushes: 0,
                writes: vec![],
            };
            let mut events = vec![];
            assert!(run_entries(&plan, &mut disk, Instant::now(), &mut |p| {
                events.push(p);
                Ok(())
            })
            .is_err());
            let failed = events.last().unwrap();
            assert!(failed.stage == "failed" || failed.stage == "final_failed");
            assert_eq!(failed.completed, 0);
            assert!(events.iter().all(|e| e.stage != "done"));
            if matches!(fault, Fault::FinalVerify) {
                assert_eq!(disk.writes.len(), 2);
            } else {
                assert!(disk.writes.len() <= 1);
            }
        }
    }
}
