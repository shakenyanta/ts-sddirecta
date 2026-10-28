//! The GUI stays unelevated. An authenticated elevated worker lives until EOF.
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    io::{self, Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    os::windows::ffi::OsStrExt,
    sync::{
        atomic::{AtomicBool, AtomicU8, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0},
    Security::Cryptography::{BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG},
    System::{
        Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE},
        Threading::WaitForSingleObject,
    },
    UI::Shell::{ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW},
};

static LOG_ATTACHED: AtomicBool = AtomicBool::new(false);
static SESSION: Mutex<Option<Session>> = Mutex::new(None);
static BUSY: AtomicBool = AtomicBool::new(false);
static STOPPING: AtomicBool = AtomicBool::new(false);
static STATUS: AtomicU8 = AtomicU8::new(0);
// Leave room for the result envelope around a worst-case 64 MiB byte array.
const MAX_FRAME: usize = 256 * 1024 * 1024 + 1024;
const MAX_READ: usize = 64 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
pub enum Operation {
    FormatPrepare {
        drive: String,
        run_id: String,
    },
    Format {
        plan: crate::sd_format::Plan,
        label: String,
        #[serde(default)]
        simple_test: bool,
        log_path: std::path::PathBuf,
    },
    Batch {
        plan: crate::sd_batch::Plan,
        log_path: std::path::PathBuf,
    },
    ReadBatch {
        plan: crate::sd_batch::Plan,
        log_path: std::path::PathBuf,
    },
    FfBatch {
        plan: crate::sd_batch::Plan,
        log_path: std::path::PathBuf,
    },
    Read {
        drive: String,
        address: u64,
        size: usize,
    },
    Write {
        drive: String,
        address: u64,
        data: Vec<u8>,
    },
    Fill {
        drive: String,
        address: u64,
        size: usize,
        fill_byte: u8,
    },
    VerifyFill {
        drive: String,
        address: u64,
        size: usize,
        fill_byte: u8,
    },
}

fn disk_number(drive: &str) -> Result<u32, String> {
    if let Some(number) = drive.strip_prefix("PhysicalDrive") {
        number.parse::<u32>().map_err(|e| e.to_string())
    } else if drive.len() == 2 && drive.as_bytes()[0].is_ascii_alphabetic() && drive.ends_with(':')
    {
        crate::windows_api::get_disk_extents(&format!("\\\\.\\{drive}")).map_err(|e| e.to_string())
    } else {
        Err("SDドライブの指定が不正です".into())
    }
}

fn execute(operation: Operation) -> Result<serde_json::Value, String> {
    use crate::windows_api;
    match operation {
        Operation::FormatPrepare { drive, run_id } => {
            serde_json::to_value(crate::sd_format::prepare(&drive, &run_id)?)
                .map_err(|e| e.to_string())
        }
        Operation::Format { .. } => Err("フォーマットは専用の実行経路を使用してください".into()),
        Operation::Batch { .. } | Operation::ReadBatch { .. } | Operation::FfBatch { .. } => {
            Err("一括SD操作は専用の実行経路を使用してください".into())
        }
        Operation::Read {
            drive,
            address,
            size,
        } => {
            // JSON encodes each byte as up to four characters.
            if size > MAX_READ {
                return Err("1回の読込サイズが大きすぎます".into());
            }
            let data = windows_api::sd_read(disk_number(&drive)?, address, size)
                .map_err(|e| e.to_string())?;
            serde_json::to_value(data).map_err(|e| e.to_string())
        }
        Operation::Write {
            drive,
            address,
            data,
        } => {
            let count = windows_api::sd_write(&drive, disk_number(&drive)?, address, &data)
                .map_err(|e| e.to_string())?;
            Ok(count.into())
        }
        Operation::Fill {
            drive,
            address,
            size,
            fill_byte,
        } => {
            let count =
                windows_api::sd_fill(&drive, disk_number(&drive)?, address, size, fill_byte)
                    .map_err(|e| e.to_string())?;
            Ok(count.into())
        }
        Operation::VerifyFill {
            drive,
            address,
            size,
            fill_byte,
        } => {
            let count = windows_api::sd_verify_fill(disk_number(&drive)?, address, size, fill_byte)
                .map_err(|e| e.to_string())?;
            Ok(count.into())
        }
    }
}

fn send<T: Serialize>(stream: &mut impl Write, value: &T) -> Result<(), String> {
    let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_FRAME {
        return Err("SD操作の転送データが大きすぎます".into());
    }
    stream
        .write_all(&(bytes.len() as u32).to_le_bytes())
        .map_err(|e| e.to_string())?;
    stream.write_all(&bytes).map_err(|e| e.to_string())
}

fn receive<T: DeserializeOwned>(stream: &mut impl Read) -> Result<T, String> {
    let mut length = [0; 4];
    stream.read_exact(&mut length).map_err(|e| e.to_string())?;
    let length = u32::from_le_bytes(length) as usize;
    if length > MAX_FRAME {
        return Err("SD操作の受信データが大きすぎます".into());
    }
    let mut bytes = vec![0; length];
    stream.read_exact(&mut bytes).map_err(|e| e.to_string())?;
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}

struct Process(HANDLE);
struct Session {
    stream: TcpStream,
    process: Process,
}

impl Session {
    fn close_and_wait(&self) -> Result<(), String> {
        // EOF releases the worker's request loop. Keep the process handle alive
        // until Windows confirms that the worker itself (not just its socket) exited.
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
        if unsafe { WaitForSingleObject(self.process.0, u32::MAX) } != WAIT_OBJECT_0 {
            return Err(format!(
                "SD操作プロセスの終了を確認できませんでした: {}",
                io::Error::last_os_error()
            ));
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
enum Response {
    Ready(bool),
    Progress(crate::sd_batch::Progress),
    Status(crate::diagnostics::OperationStatus),
    Finished(Result<serde_json::Value, String>),
}

pub fn is_busy() -> bool {
    BUSY.load(Ordering::SeqCst)
}

pub fn status() -> &'static str {
    if let Ok(mut session) = SESSION.try_lock() {
        if session
            .as_ref()
            .is_some_and(|s| unsafe { WaitForSingleObject(s.process.0, 0) } == WAIT_OBJECT_0)
        {
            *session = None;
            STATUS.store(0, Ordering::SeqCst);
        }
    }
    match STATUS.load(Ordering::SeqCst) {
        1 => "checking",
        2 => "ready",
        3 => "error",
        _ => "idle",
    }
}

pub fn shutdown() -> Result<(), String> {
    request_shutdown();
    let mut session = SESSION.lock().map_err(|e| e.to_string())?;
    if let Some(worker) = session.as_ref() {
        if let Err(error) = worker.close_and_wait() {
            STATUS.store(3, Ordering::SeqCst);
            return Err(error);
        }
    }
    *session = None;
    STATUS.store(0, Ordering::SeqCst);
    Ok(())
}

pub fn request_shutdown() {
    STOPPING.store(true, Ordering::SeqCst);
}

pub fn resume_after_update_failure() {
    // Only resume after shutdown actually released the worker session.
    if SESSION.lock().is_ok_and(|session| session.is_none()) {
        STOPPING.store(false, Ordering::SeqCst);
    }
}

fn elevated() -> Result<bool, String> {
    use windows_sys::Win32::{
        Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY},
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };
    let mut token = 0;
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error().to_string());
    }
    let token = Process(token);
    let mut elevation: TOKEN_ELEVATION = unsafe { std::mem::zeroed() };
    let mut size = 0;
    if unsafe {
        GetTokenInformation(
            token.0,
            TokenElevation,
            &mut elevation as *mut _ as *mut _,
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut size,
        )
    } == 0
    {
        return Err(io::Error::last_os_error().to_string());
    }
    Ok(elevation.TokenIsElevated != 0)
}
impl Drop for Process {
    fn drop(&mut self) {
        if self.0 != 0 {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

fn launch(address: SocketAddr, token: &str) -> Result<Process, String> {
    crate::diagnostics::step("worker launch / UAC", || {
        struct Com(bool);
        impl Drop for Com {
            fn drop(&mut self) {
                if self.0 {
                    unsafe {
                        CoUninitialize();
                    }
                }
            }
        }
        let _com = Com(unsafe {
            CoInitializeEx(
                std::ptr::null(),
                (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32,
            )
        } >= 0);
        let executable = std::env::current_exe().map_err(|e| e.to_string())?;
        let executable: Vec<u16> = executable
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        let verb: Vec<u16> = "runas\0".encode_utf16().collect();
        // Parameters contain only a loopback address and a random hexadecimal token.
        let arguments: Vec<u16> = format!("--sd-worker {address} {token}\0")
            .encode_utf16()
            .collect();
        let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
        info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
        info.lpVerb = verb.as_ptr();
        info.lpFile = executable.as_ptr();
        info.lpParameters = arguments.as_ptr();
        info.nShow = 0;
        if unsafe { ShellExecuteExW(&mut info) } == 0 {
            let error = io::Error::last_os_error();
            return Err(if error.raw_os_error() == Some(1223) {
                "管理者権限の確認がキャンセルされました".into()
            } else {
                format!("SD操作プロセスを起動できませんでした: {error}")
            });
        }
        Ok(Process(info.hProcess))
    })
}

fn connect() -> Result<Session, String> {
    crate::diagnostics::step("worker connect / authentication", || {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|e| e.to_string())?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let mut random = [0u8; 32];
        if unsafe {
            BCryptGenRandom(
                std::ptr::null_mut(),
                random.as_mut_ptr(),
                random.len() as u32,
                BCRYPT_USE_SYSTEM_PREFERRED_RNG,
            )
        } < 0
        {
            return Err("SD操作の認証情報を生成できませんでした".into());
        }
        let token: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let process = launch(listener.local_addr().map_err(|e| e.to_string())?, &token)?;
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(30) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_nonblocking(false).map_err(|e| e.to_string())?;
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .map_err(|e| e.to_string())?;
                    let mut received = [0; 64];
                    if stream.read_exact(&mut received).is_err() || received != token.as_bytes() {
                        continue;
                    }
                    stream.set_read_timeout(None).map_err(|e| e.to_string())?;
                    stream
                        .set_read_timeout(Some(Duration::from_secs(10)))
                        .map_err(|e| e.to_string())?;
                    if !matches!(receive::<Response>(&mut stream)?, Response::Ready(true)) {
                        return Err("SD操作プロセスの管理者権限を確認できませんでした".into());
                    }
                    stream.set_read_timeout(None).map_err(|e| e.to_string())?;
                    stream.set_nodelay(true).map_err(|e| e.to_string())?;
                    return Ok(Session { stream, process });
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    if process.0 != 0
                        && unsafe { WaitForSingleObject(process.0, 0) } == WAIT_OBJECT_0
                    {
                        return Err("SD操作プロセスが接続前に終了しました".into());
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(error) => return Err(error.to_string()),
            }
        }
        Err("SD操作プロセスへの接続がタイムアウトしました".into())
    })
}

fn invoke_blocking(
    operation: Operation,
    mut progress: impl FnMut(crate::sd_batch::Progress),
    status: impl Fn(crate::diagnostics::OperationStatus) + Send + Sync + 'static,
) -> Result<serde_json::Value, String> {
    let run_id = match &operation {
        Operation::Format { plan, .. } => plan.run_id.as_str(),
        Operation::FormatPrepare { run_id, .. } => run_id.as_str(),
        Operation::Batch { plan, .. }
        | Operation::ReadBatch { plan, .. }
        | Operation::FfBatch { plan, .. } => plan.run_id.as_str(),
        _ => "single",
    };
    let run = crate::diagnostics::Run::start_segment(run_id, "gui_worker_request");
    let status = Arc::new(status);
    let monitor_status = status.clone();
    let _monitor = run.monitor(move |event| monitor_status(event));
    crate::diagnostics::step("worker request", || {
        if BUSY
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err("別のSD操作を実行中です".into());
        }
        struct Guard;
        impl Drop for Guard {
            fn drop(&mut self) {
                BUSY.store(false, Ordering::SeqCst);
            }
        }
        let _busy = Guard;
        if STOPPING.load(Ordering::SeqCst) {
            return Err("アプリの終了処理中です".into());
        }
        let mut session = SESSION.lock().map_err(|e| e.to_string())?;
        if session
            .as_ref()
            .is_some_and(|s| unsafe { WaitForSingleObject(s.process.0, 0) } == WAIT_OBJECT_0)
        {
            *session = None;
        }
        if session.is_none() {
            STATUS.store(1, Ordering::SeqCst);
            match connect() {
                Ok(connected) => {
                    *session = Some(connected);
                    STATUS.store(2, Ordering::SeqCst);
                }
                Err(error) => {
                    STATUS.store(
                        if error.contains("キャンセル") {
                            0
                        } else {
                            3
                        },
                        Ordering::SeqCst,
                    );
                    return Err(error);
                }
            }
        }
        let stream = &mut session.as_mut().unwrap().stream;
        let exchange = (|| -> Result<Result<serde_json::Value, String>, String> {
            crate::diagnostics::step("worker send request", || send(stream, &operation))?;
            loop {
                match crate::diagnostics::step("worker wait response", || {
                    receive::<Response>(stream)
                })? {
                    Response::Progress(event) => progress(event),
                    Response::Status(event) => status(event),
                    Response::Finished(result) => return Ok(result),
                    Response::Ready(_) => return Err("SD操作プロセスの応答が不正です".into()),
                }
            }
        })();
        match exchange {
            Ok(result) => result,
            Err(error) => {
                if let Some(disconnected) = session.take() {
                    crate::diagnostics::record(
                        "BEGIN",
                        "disconnected worker: wait for process exit",
                    );
                    let _ = disconnected.stream.shutdown(std::net::Shutdown::Both);
                    // Do not unlock the UI while a disconnected worker may still write.
                    unsafe {
                        WaitForSingleObject(disconnected.process.0, u32::MAX);
                    }
                }
                crate::diagnostics::record("END", "disconnected worker: process exited");
                STATUS.store(3, Ordering::SeqCst);
                Err(format!("SD操作プロセスとの通信に失敗しました: {error}"))
            }
        }
    })
}

pub async fn invoke<T: DeserializeOwned>(operation: Operation) -> Result<T, String> {
    let result =
        tauri::async_runtime::spawn_blocking(move || invoke_blocking(operation, |_| {}, |_| {}))
            .await
            .map_err(|e| e.to_string())??;
    serde_json::from_value(result).map_err(|e| e.to_string())
}

pub async fn write_batch(
    plan: crate::sd_batch::Plan,
    app: tauri::AppHandle,
) -> Result<crate::sd_batch::Progress, String> {
    use tauri::Emitter;
    let result = tauri::async_runtime::spawn_blocking(move || {
        let log_path = crate::diagnostics::path()?;
        let status_app = app.clone();
        invoke_blocking(
            Operation::Batch { plan, log_path },
            |event| {
                let _ = app.emit("sd-progress", event);
            },
            move |status| {
                let _ = status_app.emit("sd-operation", status);
            },
        )
    })
    .await
    .map_err(|e| e.to_string())??;
    serde_json::from_value(result).map_err(|e| e.to_string())
}

pub async fn read_batch(
    plan: crate::sd_batch::Plan,
    app: tauri::AppHandle,
) -> Result<crate::sd_batch::Progress, String> {
    use tauri::Emitter;
    let result = tauri::async_runtime::spawn_blocking(move || {
        let log_path = crate::diagnostics::path()?;
        let status_app = app.clone();
        invoke_blocking(
            Operation::ReadBatch { plan, log_path },
            |event| {
                let _ = app.emit("sd-progress", event);
            },
            move |status| {
                let _ = status_app.emit("sd-operation", status);
            },
        )
    })
    .await
    .map_err(|e| e.to_string())??;
    serde_json::from_value(result).map_err(|e| e.to_string())
}

fn serve_requests(stream: &mut TcpStream) -> Result<(), String> {
    loop {
        let operation = receive::<Operation>(stream)?;
        let reading = matches!(&operation, Operation::ReadBatch { .. });
        let initializing = matches!(&operation, Operation::FfBatch { .. });
        let result = match operation {
            Operation::Format {
                plan,
                label,
                simple_test,
                log_path,
            } => {
                if !LOG_ATTACHED.swap(true, Ordering::SeqCst) {
                    crate::diagnostics::attach(log_path)?;
                }
                let run = crate::diagnostics::Run::start_segment(&plan.run_id, "worker_format");
                let writer = Arc::new(Mutex::new(stream.try_clone().map_err(|e| e.to_string())?));
                let monitor_writer = writer.clone();
                let _monitor = run.monitor(move |status| {
                    if let Ok(mut writer) = monitor_writer.lock() {
                        let _ = send(&mut *writer, &Response::Status(status));
                    }
                });
                crate::sd_format::execute(plan, &label, simple_test, |event| {
                    send(
                        &mut *writer.lock().map_err(|e| e.to_string())?,
                        &Response::Progress(event),
                    )
                })
                .and_then(|info| serde_json::to_value(info).map_err(|e| e.to_string()))
            }
            Operation::Batch { plan, log_path }
            | Operation::ReadBatch { plan, log_path }
            | Operation::FfBatch { plan, log_path } => {
                if !LOG_ATTACHED.swap(true, Ordering::SeqCst) {
                    crate::diagnostics::attach(log_path)?;
                }
                let run = crate::diagnostics::Run::start_segment(
                    &plan.run_id,
                    if reading {
                        "worker_read_batch"
                    } else if initializing {
                        "worker_ff_batch"
                    } else {
                        "worker_batch"
                    },
                );
                let writer = Arc::new(Mutex::new(stream.try_clone().map_err(|e| e.to_string())?));
                let monitor_writer = writer.clone();
                let _monitor = run.monitor(move |status| {
                    if let Ok(mut writer) = monitor_writer.lock() {
                        let _ = send(&mut *writer, &Response::Status(status));
                    }
                });
                let notify = |event| {
                    send(
                        &mut *writer.lock().map_err(|e| e.to_string())?,
                        &Response::Progress(event),
                    )
                };
                (if reading {
                    crate::sd_read_batch::execute(plan, notify)
                } else if initializing {
                    crate::sd_ff_batch::execute(plan, notify)
                } else {
                    crate::sd_batch::execute(plan, notify)
                })
                .and_then(|done| serde_json::to_value(done).map_err(|e| e.to_string()))
            }
            operation => execute(operation),
        };
        send(stream, &Response::Finished(result))?;
    }
}

pub async fn ff_batch(
    plan: crate::sd_batch::Plan,
    app: tauri::AppHandle,
) -> Result<crate::sd_batch::Progress, String> {
    use tauri::Emitter;
    let result = tauri::async_runtime::spawn_blocking(move || {
        let log_path = crate::diagnostics::path()?;
        let status_app = app.clone();
        invoke_blocking(
            Operation::FfBatch { plan, log_path },
            |event| {
                let _ = app.emit("sd-progress", event);
            },
            move |status| {
                let _ = status_app.emit("sd-operation", status);
            },
        )
    })
    .await
    .map_err(|e| e.to_string())??;
    serde_json::from_value(result).map_err(|e| e.to_string())
}

pub async fn format(
    plan: crate::sd_format::Plan,
    label: String,
    simple_test: bool,
    app: tauri::AppHandle,
) -> Result<crate::sd_format::ResultInfo, String> {
    use tauri::Emitter;
    let result = tauri::async_runtime::spawn_blocking(move || {
        let log_path = crate::diagnostics::path()?;
        let status_app = app.clone();
        invoke_blocking(
            Operation::Format {
                plan,
                label,
                simple_test,
                log_path,
            },
            |event| {
                let _ = app.emit("sd-progress", event);
            },
            move |status| {
                let _ = status_app.emit("sd-operation", status);
            },
        )
    })
    .await
    .map_err(|e| e.to_string())??;
    serde_json::from_value(result).map_err(|e| e.to_string())
}

/// Handle worker mode before starting Tauri/WebView2, including malformed requests.
pub fn run_if_worker() -> bool {
    let arguments: Vec<String> = std::env::args().collect();
    if arguments.get(1).map(String::as_str) != Some("--sd-worker") {
        return false;
    }
    let _ = (|| -> Result<(), String> {
        if arguments.len() != 4 {
            return Err("SD操作引数が不正です".into());
        }
        let address: SocketAddr = arguments[2].parse().map_err(|_| "接続先が不正です")?;
        if address.ip() != Ipv4Addr::LOCALHOST
            || arguments[3].len() != 64
            || !arguments[3].bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err("SD操作の接続情報が不正です".into());
        }
        let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(10))
            .map_err(|e| e.to_string())?;
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .map_err(|e| e.to_string())?;
        stream
            .write_all(arguments[3].as_bytes())
            .map_err(|e| e.to_string())?;
        let is_elevated = elevated()?;
        send(&mut stream, &Response::Ready(is_elevated))?;
        if !is_elevated {
            return Err("管理者権限がありません".into());
        }
        // Idle reads block until another request or the GUI closes the socket.
        stream.set_read_timeout(None).map_err(|e| e.to_string())?;
        stream.set_nodelay(true).map_err(|e| e.to_string())?;
        serve_requests(&mut stream)
    })();
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_round_trip_and_reject_truncated_or_oversized_frames() {
        let mut bytes = Vec::new();
        send(
            &mut bytes,
            &Operation::Write {
                drive: "E:".into(),
                address: 512,
                data: vec![0, 255],
            },
        )
        .unwrap();
        let operation: Operation = receive(&mut bytes.as_slice()).unwrap();
        assert!(
            matches!(operation, Operation::Write { drive, address: 512, data } if drive == "E:" && data == [0, 255])
        );
        assert!(receive::<Operation>(&mut &bytes[..bytes.len() - 1]).is_err());
        assert!(
            receive::<Operation>(&mut ((MAX_FRAME + 1) as u32).to_le_bytes().as_slice()).is_err()
        );
    }

    #[test]
    fn invalid_drive_is_rejected_before_disk_access() {
        assert!(disk_number("C:\\file").is_err());
        assert!(disk_number("PhysicalDriveinvalid").is_err());
        assert_eq!(disk_number("PhysicalDrive12").unwrap(), 12);
        assert!(execute(Operation::Read {
            drive: "PhysicalDrive0".into(),
            address: 0,
            size: MAX_READ + 1,
        })
        .is_err());
    }

    #[test]
    fn worker_reuses_connection_after_operation_errors_and_exits_on_disconnect() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            // No disk is touched: malformed drives fail before the Windows API.
            serve_requests(&mut stream)
        });
        for _ in 0..2 {
            send(
                &mut client,
                &Operation::Read {
                    drive: "invalid".into(),
                    address: 0,
                    size: 512,
                },
            )
            .unwrap();
            assert!(matches!(
                receive::<Response>(&mut client).unwrap(),
                Response::Finished(Err(_))
            ));
        }
        drop(client);
        assert!(server.join().unwrap().is_err());
    }

    #[test]
    fn shutdown_closes_connection_then_waits_for_windows_exit_signal() {
        use windows_sys::Win32::System::Threading::{CreateEventW, SetEvent};
        // A Windows event simulates the process-exit signal. No elevation or SD
        // is needed, and the test controls exactly when the wait can finish.
        let signal = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
        assert_ne!(signal, 0);
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut peer, _) = listener.accept().unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let session = Session {
            stream: client,
            process: Process(signal),
        };
        let (sent, received) = std::sync::mpsc::channel();
        let waiting = std::thread::spawn(move || {
            let result = session.close_and_wait();
            sent.send(result).unwrap();
        });
        assert_eq!(peer.read(&mut [0u8; 1]).unwrap(), 0);
        assert!(matches!(
            received.recv_timeout(Duration::from_millis(50)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ));
        assert_ne!(unsafe { SetEvent(signal) }, 0);
        received
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        waiting.join().unwrap();
    }

    #[test]
    fn shutdown_reports_invalid_process_handle_instead_of_claiming_success() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (_peer, _) = listener.accept().unwrap();
        let session = Session {
            stream: client,
            process: Process(0),
        };
        assert!(session
            .close_and_wait()
            .unwrap_err()
            .contains("終了を確認できませんでした"));
    }
}
