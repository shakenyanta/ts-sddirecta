//! Windows formats a single, previously inspected removable disk.
use crate::sd_batch::Progress;
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use std::{
    io::{BufRead, BufReader, Write},
    os::windows::process::CommandExt,
    process::{Command, Stdio},
    time::Instant,
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Target {
    pub disk: u32,
    pub capacity: u64,
    pub path: String,
    pub unique_id: String,
    pub serial: String,
    pub sector_size: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Plan {
    pub run_id: String,
    pub drive: String,
    pub target: Target,
    pub file_system: String,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct ResultInfo {
    pub drive: String,
    pub file_system: String,
    pub capacity: u64,
    pub label: String,
}

pub fn file_system(capacity: u64, sector: u32) -> Result<&'static str, String> {
    if sector != 512 || !(16 * 1024 * 1024..=2 * 1024_u64.pow(4)).contains(&capacity) {
        return Err("対応容量は16MiB〜2TiB、論理セクターサイズは512バイトです".into());
    }
    Ok(if capacity <= 2 * 1024_u64.pow(3) {
        "FAT"
    } else if capacity <= 32 * 1024_u64.pow(3) {
        "FAT32"
    } else {
        "exFAT"
    })
}
pub fn validate_label(label: &str) -> Result<(), String> {
    if label.len() > 11
        || !label
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b" _-".contains(&c))
    {
        return Err("ラベルは半角英数字・空白・_・-で11文字以内にしてください".into());
    }
    Ok(())
}

fn script(
    request: serde_json::Value,
    mut stage: impl FnMut(&str, &str) -> Result<(), String>,
) -> Result<serde_json::Value, String> {
    let encoded = STANDARD.encode(
        include_str!("sd_format.ps1")
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>(),
    );
    // Absolute system path, no user-supplied shell fragments or script files.
    let powershell = std::path::PathBuf::from(
        std::env::var_os("SystemRoot").ok_or("Windowsの保存先を取得できません")?,
    )
    .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let mut child = Command::new(powershell)
        .args(["-NoProfile", "-NonInteractive", "-EncodedCommand", &encoded])
        .creation_flags(0x08000000)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Windowsフォーマット機能を起動できません: {e}"))?;
    let input = serde_json::to_vec(&request).map_err(|e| e.to_string())?;
    let sent = child
        .stdin
        .take()
        .ok_or("入力を開けません")?
        .write_all(&input);
    let mut result = None;
    let mut failure = sent.err().map(|e| e.to_string());
    for line in BufReader::new(child.stdout.take().ok_or("出力を開けません")?).lines() {
        match line
            .map_err(|e| e.to_string())
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).map_err(|e| e.to_string()))
        {
            Ok(event) => {
                if let Some(error) = event["error"].as_str() {
                    failure = Some(error.to_owned());
                }
                if let Some(value) = event.get("result") {
                    result = Some(value.clone());
                }
                if let Some(name) = event["stage"].as_str() {
                    if let Err(error) = stage(name, event["message"].as_str().unwrap_or(name)) {
                        failure = Some(error);
                    }
                }
            }
            Err(error) => {
                failure = Some(format!("フォーマット処理の応答を取得できません: {error}"));
            }
        }
    }
    // Never release the worker's busy guard while PowerShell might still write.
    let status = child.wait().map_err(|e| e.to_string())?;
    if let Some(error) = failure {
        return Err(error);
    }
    if !status.success() {
        return Err("Windowsのフォーマット処理が失敗しました".into());
    }
    result.ok_or_else(|| "Windowsから完了結果を取得できませんでした".into())
}

pub fn prepare(drive: &str, run_id: &str) -> Result<Plan, String> {
    if run_id.is_empty() || run_id.len() > 100 {
        return Err("実行IDが不正です".into());
    }
    let (disk, capacity) = if let Some(disk) = crate::windows_api::physical_drive_number(drive) {
        (
            disk,
            crate::windows_api::removable_physical_size(disk).map_err(|e| e.to_string())?,
        )
    } else {
        crate::windows_api::removable_target(drive)?
    };
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
        return Err("exeはフォーマット対象SD以外のディスクへ置いてください".into());
    }
    let target: Target = serde_json::from_value(script(
        serde_json::json!({ "mode": "inspect", "disk": disk }),
        |_, _| Ok(()),
    )?)
    .map_err(|e| e.to_string())?;
    if target.disk != disk || target.capacity != capacity {
        return Err("SDの情報が変わりました。選択し直してください".into());
    }
    let fs = file_system(capacity, target.sector_size)?;
    Ok(Plan {
        run_id: run_id.into(),
        drive: drive.into(),
        target,
        file_system: fs.into(),
    })
}

pub fn execute(
    plan: Plan,
    label: &str,
    mut notify: impl FnMut(Progress) -> Result<(), String>,
) -> Result<ResultInfo, String> {
    validate_label(label)?;
    if prepare(&plan.drive, &plan.run_id)? != plan {
        return Err("確認後にSDの情報が変わりました。選択し直してください".into());
    }
    let started = Instant::now();
    let value = script(
        serde_json::json!({ "mode": "format", "target": plan.target, "drive": plan.drive, "file_system": plan.file_system, "label": label }),
        |stage, message| {
            crate::diagnostics::record("INFO", &format!("format stage={stage} {message}"));
            notify(Progress {
                run_id: plan.run_id.clone(),
                filename: message.into(),
                stage: stage.into(),
                index: 0,
                total: 0,
                completed: 0,
                processed_bytes: 0,
                total_bytes: 0,
                elapsed_ms: started.elapsed().as_millis() as u64,
                file_ms: 0,
                error: None,
            })
        },
    )?;
    let info: ResultInfo = serde_json::from_value(value).map_err(|e| e.to_string())?;
    if info.file_system != plan.file_system
        || info.label != label
        || info.capacity == 0
        || info.capacity > plan.target.capacity
    {
        return Err("フォーマット結果が確認内容と一致しません".into());
    }
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capacity_boundaries_and_labels() {
        for (size, fs) in [
            (16 * 1024 * 1024, "FAT"),
            (2 * 1024_u64.pow(3), "FAT"),
            (2 * 1024_u64.pow(3) + 512, "FAT32"),
            (32 * 1024_u64.pow(3), "FAT32"),
            (32 * 1024_u64.pow(3) + 512, "exFAT"),
            (2 * 1024_u64.pow(4), "exFAT"),
        ] {
            assert_eq!(file_system(size, 512).unwrap(), fs);
        }
        assert!(file_system(16 * 1024 * 1024 - 512, 512).is_err());
        assert!(file_system(2 * 1024_u64.pow(4) + 512, 512).is_err());
        assert!(file_system(512 * 1024 * 1024, 4096).is_err());
        for label in ["", "SD_CARD", "TEST 123-45"] {
            validate_label(label).unwrap();
        }
        for label in ["ABCDEFGHIJKL", "日本語", "x;Remove", "a/b", "\n"] {
            assert!(validate_label(label).is_err());
        }
    }
}
