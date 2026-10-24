/// Rust バックエンドから挨拶メッセージを返す Tauri コマンド
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! Rust バックエンドからこんにちは！", name)
}

mod matching_files;
mod sd_batch;
mod sd_worker;
mod source_directory;
mod startup_csv;
mod windows_api;
pub use sd_worker::run_if_worker;

use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};

static WRITE_RUNNING: AtomicBool = AtomicBool::new(false);
static EXIT_WAITING: AtomicBool = AtomicBool::new(false);
static EXIT_READY: AtomicBool = AtomicBool::new(false);
static WRITE_PLAN: Mutex<Option<sd_batch::Plan>> = Mutex::new(None);

#[tauri::command]
fn sd_admin_status() -> &'static str {
    sd_worker::status()
}

#[tauri::command]
async fn prepare_sd_write(
    directory: String,
    drive: String,
    run_id: String,
) -> Result<sd_batch::Plan, String> {
    if EXIT_WAITING.load(Ordering::SeqCst) {
        return Err("アプリの終了処理中です".into());
    }
    if WRITE_RUNNING.load(Ordering::SeqCst) || sd_worker::is_busy() {
        return Err("SD処理中です".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let plan = sd_batch::prepare(&directory, &drive, &run_id)?;
        *WRITE_PLAN.lock().map_err(|e| e.to_string())? = Some(plan.clone());
        Ok(plan)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn discard_sd_write(run_id: String) -> Result<(), String> {
    let mut pending = WRITE_PLAN.lock().map_err(|e| e.to_string())?;
    if pending.as_ref().is_some_and(|p| p.run_id == run_id) {
        *pending = None;
    }
    Ok(())
}

#[tauri::command]
async fn run_sd_write(run_id: String, app: tauri::AppHandle) -> Result<sd_batch::Progress, String> {
    if EXIT_WAITING.load(Ordering::SeqCst) {
        return Err("アプリの終了処理中です".into());
    }
    if WRITE_RUNNING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err("SD処理中です".into());
    }
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            WRITE_RUNNING.store(false, Ordering::SeqCst);
        }
    }
    let _guard = Guard;
    let plan = {
        let mut pending = WRITE_PLAN.lock().map_err(|e| e.to_string())?;
        if pending.as_ref().is_none_or(|p| p.run_id != run_id) {
            return Err("書込内容を再確認してください".into());
        }
        pending.take().unwrap()
    };
    let checked =
        tauri::async_runtime::spawn_blocking(move || -> Result<sd_batch::Plan, String> {
            let current = sd_batch::prepare(&plan.source, &plan.drive, &plan.run_id)?;
            if current != plan {
                return Err(
                    "確認後にCSV・ファイル・SDが変更されました。書込内容を再確認してください"
                        .into(),
                );
            }
            if plan.entries.is_empty() {
                return Err("書込対象なし".into());
            }
            Ok(plan)
        })
        .await
        .map_err(|e| e.to_string())??;
    sd_worker::write_batch(checked, app).await
}

#[tauri::command]
fn get_executable_directory() -> Result<String, String> {
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let directory = executable
        .parent()
        .ok_or("実行ファイルの保存先を取得できませんでした")?;
    directory
        .to_str()
        .map(str::to_owned)
        .ok_or_else(|| "保存先の文字コードを変換できませんでした".to_string())
}

#[derive(Serialize)]
struct DriveInfo {
    drive: String,
    #[serde(rename = "type")]
    kind: String,
    size: i64,
}

#[tauri::command]
fn resolve_drop_directory(paths: Vec<String>) -> Result<String, String> {
    source_directory::dropped_directory(paths)
}

#[tauri::command]
fn check_startup_csv(directory: Option<String>) -> Result<bool, String> {
    startup_csv_source(directory).map(|_| true)
}

fn startup_csv_source(directory: Option<String>) -> Result<startup_csv::CsvSource, String> {
    let source = source_directory::directory(directory)?;
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let directory = executable
        .parent()
        .ok_or("exeの保存先を取得できませんでした")?;
    startup_csv::resolve_for_source(&source, directory).map_err(|e| e.to_string())
}

#[tauri::command]
fn read_startup_csv(directory: Option<String>) -> Result<StartupCsv, String> {
    let source = startup_csv_source(directory)?;
    let executable_directory = get_executable_directory()?;
    let source_folder = source.folder_label(std::path::Path::new(&executable_directory));
    let contents = source.read().map_err(|e| e.to_string())?;
    Ok(StartupCsv {
        contents,
        source_folder,
    })
}

#[derive(Serialize)]
struct StartupCsv {
    contents: String,
    source_folder: &'static str,
}

#[derive(Serialize)]
struct MatchedFiles {
    files: Vec<String>,
    warning_files: Vec<String>,
    from_input: bool,
    input_exists: bool,
}

#[tauri::command]
fn find_dictionary_files(
    filenames: Vec<String>,
    directory: Option<String>,
) -> Result<MatchedFiles, String> {
    let directory = source_directory::directory(directory)?;
    let input_exists = source_directory::input_exists(&directory)?;
    let (files, from_input, warning_files) =
        matching_files::find_matches(&directory, filenames).map_err(|e| e.to_string())?;
    Ok(MatchedFiles {
        files,
        warning_files,
        from_input,
        input_exists,
    })
}

#[tauri::command]
async fn get_drives() -> Result<Vec<DriveInfo>, String> {
    tauri::async_runtime::spawn_blocking(list_drives)
        .await
        .map_err(|e| e.to_string())?
}

fn list_drives() -> Result<Vec<DriveInfo>, String> {
    use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};
    const DRIVE_REMOVABLE: u32 = 2;
    const DRIVE_FIXED: u32 = 3;

    let mut out = Vec::new();
    let drives = unsafe { GetLogicalDrives() };

    for i in 0..26 {
        if (drives >> i) & 1 == 1 {
            let drive_letter = (b'A' + i) as char;
            let drive_path = format!("{}:\\", drive_letter);
            let mut wide: Vec<u16> = drive_path.encode_utf16().collect();
            wide.push(0);

            let drive_type = unsafe { GetDriveTypeW(wide.as_ptr()) };

            if drive_type == DRIVE_REMOVABLE || drive_type == DRIVE_FIXED {
                let drive_str = format!("{}:", drive_letter);
                let kind = if drive_type == DRIVE_REMOVABLE {
                    "Removable"
                } else {
                    "Fixed"
                };

                let volume_path = format!("\\\\.\\{}", drive_str);
                let size = match windows_api::get_disk_extents(&volume_path) {
                    Ok(disk_num) if disk_num != 0xFF => {
                        let physical_path = format!("\\\\.\\physicaldrive{}", disk_num);
                        windows_api::get_physical_size(&physical_path).unwrap_or(0) as i64
                    }
                    _ => -1,
                };

                out.push(DriveInfo {
                    drive: drive_str,
                    kind: kind.to_string(),
                    size,
                });
            }
        }
    }

    Ok(out)
}

#[tauri::command]
async fn sd_read(drive: String, address: u64, size: usize) -> Result<Vec<u8>, String> {
    sd_worker::invoke(sd_worker::Operation::Read {
        drive,
        address,
        size,
    })
    .await
}

#[tauri::command]
async fn sd_write(drive: String, address: u64, data: Vec<u8>) -> Result<u32, String> {
    sd_worker::invoke(sd_worker::Operation::Write {
        drive,
        address,
        data,
    })
    .await
}

#[tauri::command]
async fn sd_fill(drive: String, address: u64, size: usize, fill_byte: u8) -> Result<usize, String> {
    sd_worker::invoke(sd_worker::Operation::Fill {
        drive,
        address,
        size,
        fill_byte,
    })
    .await
}

#[tauri::command]
async fn sd_verify_fill(
    drive: String,
    address: u64,
    size: usize,
    fill_byte: u8,
) -> Result<usize, String> {
    sd_worker::invoke(sd_worker::Operation::VerifyFill {
        drive,
        address,
        size,
        fill_byte,
    })
    .await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            greet,
            get_executable_directory,
            resolve_drop_directory,
            check_startup_csv,
            read_startup_csv,
            find_dictionary_files,
            get_drives,
            sd_admin_status,
            prepare_sd_write,
            discard_sd_write,
            run_sd_write,
            sd_read,
            sd_write,
            sd_fill,
            sd_verify_fill
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                use tauri::Emitter;
                if EXIT_READY.load(Ordering::SeqCst) {
                    return;
                }
                api.prevent_close();
                if WRITE_RUNNING.load(Ordering::SeqCst) || sd_worker::is_busy() {
                    let _ = window.emit("sd-close-blocked", ());
                } else if EXIT_WAITING
                    .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok()
                {
                    sd_worker::request_shutdown();
                    let _ = window.emit("sd-shutdown-started", ());
                    let window = window.clone();
                    // Process waiting must not block the WebView/event-loop thread.
                    tauri::async_runtime::spawn(async move {
                        let result = tauri::async_runtime::spawn_blocking(sd_worker::shutdown)
                            .await
                            .map_err(|e| e.to_string())
                            .and_then(|r| r);
                        match result {
                            Ok(()) => {
                                EXIT_READY.store(true, Ordering::SeqCst);
                                if let Err(error) = window.close() {
                                    EXIT_WAITING.store(false, Ordering::SeqCst);
                                    EXIT_READY.store(false, Ordering::SeqCst);
                                    let _ = window.emit("sd-shutdown-error", error.to_string());
                                }
                            }
                            Err(error) => {
                                EXIT_WAITING.store(false, Ordering::SeqCst);
                                let _ = window.emit("sd-shutdown-error", error);
                            }
                        }
                    });
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                let _ = sd_worker::shutdown();
            }
        });
}
