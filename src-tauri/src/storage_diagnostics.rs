//! Best-effort Windows evidence after an SD operation has released its handles.
use base64::{engine::general_purpose::STANDARD, Engine};
use std::{
    io::{Read, Seek, SeekFrom, Write},
    os::windows::process::CommandExt,
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

static COLLECTING: AtomicBool = AtomicBool::new(false);
const SCRIPT: &str = include_str!("storage_diagnostics.ps1");
const TIMEOUT: Duration = Duration::from_secs(15);
const OUTPUT_LIMIT: u64 = 96 * 1024;

// Result is inspected by reference: diagnostics cannot replace the original error.
pub fn on_failure<T>(
    result: &Result<T, String>,
    run_id: String,
    drive: String,
    disk: u32,
    capacity: u64,
    operation: &'static str,
) {
    let Err(error) = result else { return };
    let failure_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let request = serde_json::json!({
        "run_id": run_id, "drive": drive, "disk": disk, "capacity": capacity,
        "operation": operation, "failure_ms": failure_ms, "version": env!("CARGO_PKG_VERSION"),
        "original_error_excerpt": error.chars().take(2048).collect::<String>(),
        "executable": std::env::current_exe().ok(),
    });
    if COLLECTING.swap(true, Ordering::SeqCst) {
        crate::diagnostics::record(
            "WARN",
            &format!("Windows evidence skipped: another collection is running; request={request}"),
        );
        return;
    }
    crate::diagnostics::record(
        "INFO",
        &format!("Windows evidence queued asynchronously request={request}"),
    );
    if let Err(error) = std::thread::Builder::new()
        .name("sd-evidence".into())
        .spawn(move || {
            struct Reset;
            impl Drop for Reset {
                fn drop(&mut self) {
                    COLLECTING.store(false, Ordering::SeqCst);
                }
            }
            let _reset = Reset;
            let _run = crate::diagnostics::Run::start_segment(&run_id, "windows_failure_evidence");
            // Allow delayed Windows events to arrive; never delay the SD result/UI.
            std::thread::sleep(Duration::from_secs(3));
            if let Err(error) = collect(&request) {
                crate::diagnostics::record(
                    "WARN",
                    &format!("Windows evidence unavailable: {error}"),
                );
            }
        })
    {
        COLLECTING.store(false, Ordering::SeqCst);
        crate::diagnostics::record(
            "WARN",
            &format!("Windows evidence thread unavailable: {error}"),
        );
    }
}

fn collect(request: &serde_json::Value) -> Result<(), String> {
    let powershell =
        std::path::PathBuf::from(std::env::var_os("SystemRoot").ok_or("SystemRoot missing")?)
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let encoded = STANDARD.encode(
        SCRIPT
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>(),
    );
    // An anonymous temporary file avoids pipe deadlocks and persistent extra logs.
    let mut input = tempfile::tempfile().map_err(|e| e.to_string())?;
    input
        .write_all(request.to_string().as_bytes())
        .map_err(|e| e.to_string())?;
    input.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut output = tempfile::tempfile().map_err(|e| e.to_string())?;
    let mut child = Command::new(powershell)
        .args(["-NoProfile", "-NonInteractive", "-EncodedCommand", &encoded])
        .creation_flags(0x08000000)
        .stdin(input)
        .stdout(output.try_clone().map_err(|e| e.to_string())?)
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    let status = wait_collector(&mut child, TIMEOUT);
    output.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    output
        .take(OUTPUT_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let truncated = bytes.len() as u64 > OUTPUT_LIMIT;
    bytes.truncate(OUTPUT_LIMIT as usize);
    let mut complete = false;
    for line in String::from_utf8_lossy(&bytes).lines() {
        match serde_json::from_str::<serde_json::Value>(line) {
            Ok(event) => {
                complete |= event["kind"] == "collection_complete";
                crate::diagnostics::record("INFO", &format!("Windows evidence {event}"));
            }
            Err(_) => {
                crate::diagnostics::record("WARN", "Windows evidence invalid/truncated JSON record")
            }
        }
    }
    if truncated {
        crate::diagnostics::record("WARN", "Windows evidence output truncated at 96 KiB");
    }
    let status = status?;
    if !status.success() {
        return Err(format!("collector exit code {:?}", status.code()));
    }
    if !complete {
        return Err("collector output incomplete".into());
    }
    crate::diagnostics::record(
        "INFO",
        "Windows evidence collection finished; time-window events are not proof of cause",
    );
    Ok(())
}

fn wait_collector(
    child: &mut std::process::Child,
    timeout: Duration,
) -> Result<std::process::ExitStatus, String> {
    let started = Instant::now();
    let result = (|| loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            return Ok(status);
        }
        if started.elapsed() >= timeout {
            return Err(format!(
                "collector timed out after {} ms",
                timeout.as_millis()
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    })();
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn successful_sd_result_does_not_start_collection() {
        on_failure(&Ok(()), "test".into(), "G:".into(), 77, 512000000, "write");
        assert!(!COLLECTING.load(Ordering::SeqCst));
    }

    #[test]
    fn embedded_script_fits_windows_command_line() {
        let encoded = STANDARD.encode(
            SCRIPT
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>(),
        );
        assert!(encoded.len() + 1024 < 32767);
    }

    #[test]
    fn real_windows_collector_completes_with_an_absent_disk() {
        // Only queries Windows metadata/events; never opens or changes an SD.
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        collect(&serde_json::json!({"disk": u32::MAX, "capacity": 1, "failure_ms": now})).unwrap();
    }

    #[test]
    fn stalled_collector_is_killed_and_reaped() {
        let powershell = std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let mut child = Command::new(powershell)
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Sleep -Seconds 30",
            ])
            .creation_flags(0x08000000)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let result = wait_collector(&mut child, Duration::from_millis(100));
        assert!(result.unwrap_err().contains("timed out"));
        assert!(child.try_wait().unwrap().is_some());
    }
}
