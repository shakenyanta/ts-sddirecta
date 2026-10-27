//! Unbuffered, per-session diagnostic log shared with the authenticated worker.
use std::{
    cell::RefCell,
    collections::BTreeMap,
    fmt::Display,
    fs::{File, OpenOptions},
    io::{self, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc, Mutex, OnceLock,
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct OperationStatus {
    pub run_id: String,
    pub operation_id: String,
    pub detail: String,
    pub operation_ms: u64,
    pub waiting: bool,
}

struct Active {
    id: String,
    name: String,
    started: Instant,
}
struct Context {
    run_id: String,
    segment: &'static str,
    started: Instant,
    active: Mutex<Vec<Active>>,
    totals: Mutex<BTreeMap<String, u128>>,
}
thread_local! {
    static CONTEXT: RefCell<Option<Arc<Context>>> = const { RefCell::new(None) };
    static QUIET: RefCell<bool> = const { RefCell::new(false) };
    static PHASE: RefCell<String> = const { RefCell::new(String::new()) };
    static POLL_NOTES: RefCell<Vec<String>> = const { RefCell::new(vec![]) };
}
static NEXT_OPERATION: AtomicU64 = AtomicU64::new(1);

pub struct Run {
    context: Arc<Context>,
    previous: Option<Arc<Context>>,
}
impl Run {
    #[cfg(test)]
    pub fn start(run_id: &str) -> Self {
        Self::start_segment(run_id, "test")
    }

    pub fn start_segment(run_id: &str, segment: &'static str) -> Self {
        let context = Arc::new(Context {
            run_id: run_id.into(),
            segment,
            started: Instant::now(),
            active: Mutex::new(vec![]),
            totals: Mutex::new(BTreeMap::new()),
        });
        let previous = CONTEXT.with(|c| c.replace(Some(context.clone())));
        record("INFO", &format!("run segment started segment={segment}"));
        Self { context, previous }
    }

    pub fn monitor(&self, mut notify: impl FnMut(OperationStatus) + Send + 'static) -> Monitor {
        let context = self.context.clone();
        let (stop, rx) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            let mut warned = String::new();
            loop {
                let status = context.active.lock().ok().and_then(|active| {
                    active.last().map(|a| {
                        (
                            a.id.clone(),
                            a.name.clone(),
                            a.started.elapsed().as_millis() as u64,
                        )
                    })
                });
                if let Some((id, name, elapsed)) = status {
                    // The worker reports its own device operation while the GUI receives IPC.
                    if name != "worker wait response" && name != "worker request" {
                        if elapsed >= 5000 && warned != id {
                            write_record("WARN", &format!("operation_id={id} device response pending elapsed_ms={elapsed} operation={name}"), Some(&context.run_id));
                            warned = id.clone();
                        }
                        notify(OperationStatus {
                            run_id: context.run_id.clone(),
                            operation_id: id,
                            detail: operation_label(&name),
                            operation_ms: elapsed,
                            waiting: elapsed >= 5000,
                        });
                    }
                }
                if rx
                    .recv_timeout(std::time::Duration::from_millis(250))
                    .is_ok()
                {
                    break;
                }
            }
        });
        Monitor {
            stop,
            thread: Some(thread),
        }
    }
}
impl Drop for Run {
    fn drop(&mut self) {
        if let Ok(totals) = self.context.totals.lock() {
            record(
                "SUMMARY",
                &format!(
                    "segment={} total_ms={} timings_ms={}",
                    self.context.segment,
                    self.context.started.elapsed().as_millis(),
                    serde_json::to_string(&*totals).unwrap_or_default()
                ),
            );
        }
        CONTEXT.with(|c| c.replace(self.previous.take()));
    }
}
pub struct Monitor {
    stop: mpsc::Sender<()>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Drop for Monitor {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub fn phase(name: &str) {
    PHASE.with(|p| *p.borrow_mut() = name.into());
}

pub fn quiet<T>(action: impl FnOnce() -> T) -> (T, Vec<String>) {
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            QUIET.with(|q| q.replace(self.0));
        }
    }
    let _reset = Reset(QUIET.with(|q| q.replace(true)));
    POLL_NOTES.with(|n| n.borrow_mut().clear());
    let result = action();
    (result, POLL_NOTES.with(|n| n.take()))
}

pub fn operation_label(name: &str) -> String {
    if let Some(rest) = name.strip_prefix("CreateFileW path=") {
        let path = rest.split(" access=").next().unwrap_or(rest);
        let path = path.strip_prefix("\\\\.\\").unwrap_or(path);
        return format!("{path}を開いています");
    }
    let label = if name == "FSCTL_LOCK_VOLUME" {
        "SDをロックしています"
    } else if name.starts_with("get_physical_size") || name.starts_with("IOCTL_DISK_GET_LENGTH") {
        "SD容量を確認しています"
    } else if name.starts_with("IOCTL_DISK_GET_DRIVE_GEOMETRY") {
        "SDのセクター情報を確認しています"
    } else if name.starts_with("get_disk_extents") {
        "ドライブと物理ディスクの対応を確認しています"
    } else if name.starts_with("ReadFile") {
        "SDから読み込んでいます"
    } else if name.starts_with("output reset") || name.starts_with("output delete") {
        "output直下のファイルを初期化しています"
    } else if name.starts_with("output save") {
        "読み込んだデータをファイルへ保存しています"
    } else if name.starts_with("WriteFile") {
        "SDへ書き込んでいます"
    } else if name == "FlushFileBuffers" {
        "SDへの書込を確定しています"
    } else if name.starts_with("worker launch") {
        "管理者権限を確認しています"
    } else if name.starts_with("worker connect") {
        "SD操作プロセスへ接続しています"
    } else if name.starts_with("source validation") {
        "入力ファイルを確認しています"
    } else if name.starts_with("batch execute") {
        "SD書込・照合を実行しています"
    } else if name.starts_with("FF batch execute") {
        "SDのFF初期化・照合を実行しています"
    } else if name.starts_with("FF prepare") {
        "FF初期化のCSV・対象領域を確認しています"
    } else if name.starts_with("prepare ") {
        "CSV・対象ファイルを確認しています"
    } else {
        "SD処理の準備をしています"
    };
    label.into()
}

fn jst_timestamp(ms: u128) -> String {
    let seconds = (ms / 1000) as i64 + 9 * 3600;
    let z = seconds / 86400 + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += if month <= 2 { 1 } else { 0 };
    let tod = seconds % 86400;
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}+09:00",
        tod / 3600,
        tod / 60 % 60,
        tod % 60,
        ms % 1000
    )
}

struct Log {
    path: PathBuf,
    file: Mutex<File>,
}
static LOG: OnceLock<Log> = OnceLock::new();
static INIT: Mutex<()> = Mutex::new(());

pub fn path() -> Result<PathBuf, String> {
    if let Some(log) = LOG.get() {
        return Ok(log.path.clone());
    }
    let _lock = INIT.lock().map_err(|e| e.to_string())?;
    if let Some(log) = LOG.get() {
        return Ok(log.path.clone());
    }
    #[cfg(not(test))]
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    #[cfg(test)]
    let base = std::env::temp_dir();
    let directory = base.join("ts-sddirecta").join("logs");
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = directory.join(format!("sd-{timestamp}-{}.txt", std::process::id()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .append(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    file.write_all(b"\xEF\xBB\xBF").map_err(|e| e.to_string())?;
    let _ = LOG.set(Log {
        path: path.clone(),
        file: Mutex::new(file),
    });
    record(
        "INFO",
        "session started; timestamps include JST (+09:00) and Unix milliseconds",
    );
    Ok(path)
}

// Called only after the GUI/worker connection has been authenticated.
pub fn attach(path: PathBuf) -> Result<(), String> {
    let file = OpenOptions::new()
        .append(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    LOG.set(Log {
        path,
        file: Mutex::new(file),
    })
    .map_err(|_| "ログは初期化済みです".to_string())?;
    record("INFO", "worker log attached");
    Ok(())
}

pub fn record(level: &str, message: &str) {
    if QUIET.with(|q| *q.borrow()) {
        return;
    }
    let context = CONTEXT.with(|c| c.borrow().clone());
    let operation = context
        .as_ref()
        .and_then(|c| {
            c.active
                .lock()
                .ok()
                .and_then(|a| a.last().map(|a| a.id.clone()))
        })
        .unwrap_or_else(|| "-".into());
    let message = if message.contains("operation_id=") {
        message.to_string()
    } else {
        format!("{message} operation_id={operation}")
    };
    write_record(level, &message, context.as_ref().map(|c| c.run_id.as_str()));
}

fn write_record(level: &str, message: &str, run_id: Option<&str>) {
    let Some(log) = LOG.get() else { return };
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    // Escape line breaks so a filename cannot create misleading log entries.
    let message = message.replace('\r', "\\r").replace('\n', "\\n");
    let line = format!(
        "{} unix_ms={timestamp} pid={} thread={:?} run_id={} {level} {message}\r\n",
        jst_timestamp(timestamp),
        std::process::id(),
        std::thread::current().id(),
        run_id.unwrap_or("-")
    );
    if let Ok(mut file) = log.file.lock() {
        // File is not buffered: BEGIN remains readable while a device call blocks.
        if let Err(error) = file.write_all(line.as_bytes()) {
            eprintln!("diagnostic log write failed: {error}");
        }
    }
}

pub fn step<T, E: Display>(
    name: impl AsRef<str>,
    action: impl FnOnce() -> Result<T, E>,
) -> Result<T, E> {
    if QUIET.with(|q| *q.borrow()) {
        let started = Instant::now();
        let result = action();
        let elapsed = started.elapsed().as_millis();
        if let Err(error) = &result {
            POLL_NOTES.with(|n| {
                n.borrow_mut()
                    .push(format!("{} error={error}", name.as_ref()))
            });
        }
        if elapsed >= 1000 {
            POLL_NOTES.with(|n| {
                n.borrow_mut()
                    .push(format!("{} slow elapsed_ms={elapsed}", name.as_ref()))
            });
        }
        return result;
    }
    let name = name.as_ref();
    let id = format!(
        "{}-{}",
        std::process::id(),
        NEXT_OPERATION.fetch_add(1, Ordering::Relaxed)
    );
    let context = CONTEXT.with(|c| c.borrow().clone());
    if let Some(c) = &context {
        if let Ok(mut active) = c.active.lock() {
            active.push(Active {
                id: id.clone(),
                name: name.into(),
                started: Instant::now(),
            });
        }
    }
    let log_step = name != "worker wait response";
    if log_step {
        record("BEGIN", &format!("{name} operation_id={id}"));
    }
    let started = Instant::now();
    let result = action();
    if log_step {
        match &result {
            Ok(_) => record(
                "END",
                &format!(
                    "{name} elapsed_ms={} operation_id={id}",
                    started.elapsed().as_millis()
                ),
            ),
            Err(error) => record(
                "ERROR",
                &format!(
                    "{name} elapsed_ms={} operation_id={id} error={error}",
                    started.elapsed().as_millis()
                ),
            ),
        }
    }
    if let Some(c) = &context {
        if let Ok(mut active) = c.active.lock() {
            active.retain(|a| a.id != id);
        }
        let category = if name.starts_with("WriteFile") {
            Some("write")
        } else if name.starts_with("ReadFile") {
            Some("read")
        } else if name == "FlushFileBuffers" {
            Some("flush")
        } else if name.starts_with("BatchDisk::open") || name.starts_with("prepare ") {
            Some("prepare")
        } else {
            None
        };
        if let Some(category) = category {
            let phase = PHASE.with(|p| p.borrow().clone());
            let key = if category == "read" && !phase.is_empty() {
                format!("{phase}_read")
            } else {
                category.into()
            };
            if let Ok(mut totals) = c.totals.lock() {
                *totals.entry(key).or_default() += started.elapsed().as_millis();
            }
        }
    }
    result
}

pub fn device_control(name: &str, action: impl FnOnce() -> i32) -> io::Result<()> {
    step(name, || {
        if action() == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jst_timestamps_include_date_rollover_and_milliseconds() {
        assert_eq!(jst_timestamp(0), "1970-01-01T09:00:00.000+09:00");
        assert_eq!(jst_timestamp(54_000_123), "1970-01-02T00:00:00.123+09:00");
    }

    #[test]
    fn monitor_reports_blocked_operation_with_run_and_operation_ids() {
        let run = Run::start("monitor-test");
        let (tx, rx) = mpsc::channel();
        let _monitor = run.monitor(move |status| {
            let _ = tx.send(status);
        });
        let result: Result<(), String> =
            step("CreateFileW path=H: access=0xC0000000 flags=0x80", || {
                let status = rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
                assert_eq!(status.run_id, "monitor-test");
                assert!(!status.operation_id.is_empty());
                assert_eq!(status.detail, "H:を開いています");
                Ok(())
            });
        result.unwrap();
    }

    #[test]
    fn quiet_polling_preserves_errors_as_change_detection_notes() {
        let (result, notes) = quiet(|| step("poll metadata", || Err::<(), _>("not ready")));
        assert_eq!(result, Err("not ready"));
        assert_eq!(notes, vec!["poll metadata error=not ready"]);
    }

    #[test]
    fn run_log_pairs_operations_and_emits_segment_summary() {
        let path = path().unwrap();
        {
            let _run = Run::start_segment("log-correlation-test", "worker_batch");
            let result: Result<(), &str> = step("WriteFile audit", || Ok(()));
            result.unwrap();
        }
        let contents = std::fs::read_to_string(path).unwrap();
        let lines: Vec<_> = contents
            .lines()
            .filter(|l| l.contains("run_id=log-correlation-test"))
            .collect();
        let begin = lines
            .iter()
            .find(|l| l.contains("BEGIN WriteFile audit"))
            .unwrap();
        let end = lines
            .iter()
            .find(|l| l.contains("END WriteFile audit"))
            .unwrap();
        assert_eq!(
            begin.split("operation_id=").last(),
            end.split("operation_id=").last()
        );
        assert!(begin.contains("+09:00 unix_ms="));
        assert!(lines
            .iter()
            .any(|l| l.contains("SUMMARY segment=worker_batch") && l.contains("\"write\":")));
    }

    #[test]
    fn log_is_readable_before_operation_finishes_and_records_errors() {
        let path = path().unwrap();
        let result: Result<(), &str> = step("test blocking operation", || {
            let contents = std::fs::read_to_string(&path).unwrap();
            assert!(contents.contains("BEGIN test blocking operation"));
            assert!(!contents.contains("END test blocking operation"));
            Err("device failure")
        });
        assert_eq!(result, Err("device failure"));
        record("INFO", "filename\r\nnext line");
        let contents = std::fs::read_to_string(path).unwrap();
        assert!(contents.contains("ERROR test blocking operation elapsed_ms="));
        assert!(contents.contains("error=device failure"));
        assert!(contents.contains("filename\\r\\nnext line"));
    }
}
