/// Rust バックエンドから挨拶メッセージを返す Tauri コマンド
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! Rust バックエンドからこんにちは！", name)
}

mod diagnostics;
mod edition;
mod matching_files;
mod portable_update;
mod sd_batch;
mod sd_detection;
mod sd_ff_batch;
mod sd_format;
mod sd_read_batch;
mod sd_worker;
mod source_directory;
mod startup_csv;
mod windows_api;
pub use portable_update::run_if_helper;
pub use sd_worker::run_if_worker;

#[tauri::command]
fn get_edition() -> edition::EditionInfo {
    edition::current().clone()
}

use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};

static WRITE_RUNNING: AtomicBool = AtomicBool::new(false);
static EXIT_WAITING: AtomicBool = AtomicBool::new(false);
static EXIT_READY: AtomicBool = AtomicBool::new(false);
static WRITE_PLAN: Mutex<Option<sd_batch::Plan>> = Mutex::new(None);
static FORMAT_PLAN: Mutex<Option<sd_format::Plan>> = Mutex::new(None);

#[tauri::command]
async fn prepare_sd_format(drive: String, run_id: String) -> Result<sd_format::Plan, String> {
    if EXIT_WAITING.load(Ordering::SeqCst)
        || WRITE_RUNNING.load(Ordering::SeqCst)
        || sd_worker::is_busy()
    {
        return Err("SD処理中または終了処理中です".into());
    }
    let plan: sd_format::Plan =
        sd_worker::invoke(sd_worker::Operation::FormatPrepare { drive, run_id }).await?;
    *FORMAT_PLAN.lock().map_err(|e| e.to_string())? = Some(plan.clone());
    Ok(plan)
}

#[tauri::command]
fn discard_sd_format(run_id: String) -> Result<(), String> {
    let mut pending = FORMAT_PLAN.lock().map_err(|e| e.to_string())?;
    if pending.as_ref().is_some_and(|plan| plan.run_id == run_id) {
        *pending = None;
    }
    Ok(())
}

#[tauri::command]
async fn run_sd_format(
    run_id: String,
    label: String,
    simple_test: bool,
    app: tauri::AppHandle,
) -> Result<sd_format::ResultInfo, String> {
    sd_format::validate_label(&label)?;
    if EXIT_WAITING.load(Ordering::SeqCst)
        || WRITE_RUNNING
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
    {
        return Err("SD処理中または終了処理中です".into());
    }
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            WRITE_RUNNING.store(false, Ordering::SeqCst);
        }
    }
    let _guard = Guard;
    let plan = {
        let mut pending = FORMAT_PLAN.lock().map_err(|e| e.to_string())?;
        if pending.as_ref().is_none_or(|plan| plan.run_id != run_id) {
            return Err("フォーマット内容を再確認してください".into());
        }
        pending.take().unwrap()
    };
    let label = if plan.file_system == "exFAT" {
        label.trim().to_owned()
    } else {
        label.trim().to_ascii_uppercase()
    };
    sd_worker::format(plan, label, simple_test, app).await
}

#[tauri::command]
async fn check_portable_update(
    app: tauri::AppHandle,
) -> Result<portable_update::UpdateCheck, String> {
    let version = app.package_info().version.to_string();
    tauri::async_runtime::spawn_blocking(move || portable_update::check(&version))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn install_portable_update(version: String, app: tauri::AppHandle) -> Result<(), String> {
    if EXIT_WAITING.load(Ordering::SeqCst)
        || sd_worker::is_busy()
        || WRITE_RUNNING
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
    {
        return Err("SD処理中または終了処理中のため更新できません".into());
    }
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            WRITE_RUNNING.store(false, Ordering::SeqCst);
        }
    }
    let _guard = Guard;
    tauri::async_runtime::spawn_blocking(move || {
        let prepared = portable_update::prepare(&version)?;
        if EXIT_WAITING
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err("アプリの終了処理中です".into());
        }
        if let Err(error) = sd_worker::shutdown() {
            EXIT_WAITING.store(false, Ordering::SeqCst);
            return Err(format!(
                "SD処理用プロセスの終了に失敗しました。アプリを再起動してください: {error}"
            ));
        }
        if let Err(error) = prepared.launch() {
            sd_worker::resume_after_update_failure();
            EXIT_WAITING.store(false, Ordering::SeqCst);
            return Err(error);
        }
        EXIT_READY.store(true, Ordering::SeqCst);
        diagnostics::record("INFO", "portable update helper ready; exiting application");
        app.exit(0);
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn open_diagnostic_log() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(|| {
        let path = diagnostics::path()?;
        diagnostics::record("INFO", "open log in associated editor");
        tauri_plugin_opener::open_path(path, None::<&str>).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn sd_admin_status() -> &'static str {
    sd_worker::status()
}

#[tauri::command]
async fn open_folder(directory: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = std::path::PathBuf::from(directory);
        if !path.is_absolute() || !path.is_dir() {
            return Err("指定したフォルダーが存在しないか、アクセスできません".into());
        }
        tauri_plugin_opener::open_path(path, None::<&str>).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn open_sd_output() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(|| {
        let path = sd_read_batch::output_directory()?;
        if !path.is_dir() {
            return Err("outputフォルダーがありません。SD読み込みを実行してください".into());
        }
        tauri_plugin_opener::open_path(path, None::<&str>).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn prepare_sd_write(
    app: tauri::AppHandle,
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
        diagnostics::path().map_err(|e| format!("診断ログを作成できません: {e}"))?;
        let run = diagnostics::Run::start_segment(&run_id, "prepare");
        let _monitor = run.monitor(move |status| {
            use tauri::Emitter;
            let _ = app.emit("sd-operation", status);
        });
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
    let prepare_app = app.clone();
    let checked =
        tauri::async_runtime::spawn_blocking(move || -> Result<sd_batch::Plan, String> {
            let run = diagnostics::Run::start_segment(&plan.run_id, "revalidate");
            let _monitor = run.monitor(move |status| {
                use tauri::Emitter;
                let _ = prepare_app.emit("sd-operation", status);
            });
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
async fn run_sd_read(
    directory: String,
    drive: String,
    run_id: String,
    app: tauri::AppHandle,
) -> Result<sd_batch::Progress, String> {
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
    let prepare_app = app.clone();
    let plan = tauri::async_runtime::spawn_blocking(move || {
        diagnostics::path()?;
        let run = diagnostics::Run::start_segment(&run_id, "read_prepare");
        let _monitor = run.monitor(move |status| {
            use tauri::Emitter;
            let _ = prepare_app.emit("sd-operation", status);
        });
        sd_read_batch::prepare(&directory, &drive, &run_id)
    })
    .await
    .map_err(|e| e.to_string())??;
    sd_worker::read_batch(plan, app).await
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

#[tauri::command]
async fn run_sd_ff_initialize(
    directory: String,
    drive: String,
    run_id: String,
    app: tauri::AppHandle,
) -> Result<sd_batch::Progress, String> {
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
    let prepare_app = app.clone();
    let plan = tauri::async_runtime::spawn_blocking(move || {
        diagnostics::path()?;
        let run = diagnostics::Run::start_segment(&run_id, "ff_prepare");
        let _monitor = run.monitor(move |status| {
            use tauri::Emitter;
            let _ = prepare_app.emit("sd-operation", status);
        });
        sd_ff_batch::prepare(&directory, &drive, &run_id)
    })
    .await
    .map_err(|e| e.to_string())??;
    sd_worker::ff_batch(plan, app).await
}

use sd_detection::DriveInfo;

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
    tauri::async_runtime::spawn_blocking(|| {
        static LAST: Mutex<Option<String>> = Mutex::new(None);
        let (result, notes) = diagnostics::quiet(list_drives);
        let snapshot = serde_json::to_string(&(&result, &notes)).unwrap_or_default();
        if let Ok(mut previous) = LAST.lock() {
            if previous.as_ref() != Some(&snapshot) {
                diagnostics::record("INFO", &format!("drive state changed {snapshot}"));
                *previous = Some(snapshot);
            }
        }
        result
    })
    .await
    .map_err(|e| e.to_string())?
}

fn list_drives() -> Result<Vec<DriveInfo>, String> {
    sd_detection::list_drives()
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
    let edition = edition::current();
    tauri::Builder::default()
        .setup(move |app| {
            use tauri::Manager;
            if let Some(window) = app.get_webview_window("main") {
                window.set_title(&format!("{} {}", app.package_info().name, edition.label()))?;
            }
            Ok(())
        })
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            greet,
            get_edition,
            check_portable_update,
            install_portable_update,
            get_executable_directory,
            resolve_drop_directory,
            check_startup_csv,
            read_startup_csv,
            find_dictionary_files,
            get_drives,
            sd_admin_status,
            open_folder,
            open_diagnostic_log,
            open_sd_output,
            prepare_sd_write,
            discard_sd_write,
            run_sd_write,
            run_sd_read,
            run_sd_ff_initialize,
            prepare_sd_format,
            discard_sd_format,
            run_sd_format,
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
