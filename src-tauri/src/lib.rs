/// Rust バックエンドから挨拶メッセージを返す Tauri コマンド
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! Rust バックエンドからこんにちは！", name)
}

mod windows_api;
use std::io;

use serde::Serialize;

#[derive(Serialize)]
struct DriveInfo { drive: String, #[serde(rename = "type")] kind: String, size: i64 }

#[tauri::command]
fn get_drives() -> Result<Vec<DriveInfo>, String> {
    use windows_sys::Win32::Storage::FileSystem::{GetLogicalDrives, GetDriveTypeW};
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
                let kind = if drive_type == DRIVE_REMOVABLE { "Removable" } else { "Fixed" };

                let volume_path = format!("\\\\.\\{}:", drive_str);
                let size = match windows_api::get_disk_extents(&volume_path) {
                    Ok(disk_num) if disk_num != 0xFF => {
                        let physical_path = format!("\\\\.\\physicaldrive{}", disk_num);
                        windows_api::get_physical_size(&physical_path).unwrap_or(0) as i64
                    }
                    _ => -1,
                };

                out.push(DriveInfo { drive: drive_str, kind: kind.to_string(), size });
            }
        }
    }

    Ok(out)
}

#[tauri::command]
fn sd_read(drive: String, address: u64, size: usize) -> Result<Vec<u8>, String> {
    let disk_num = if drive.starts_with("PhysicalDrive") {
        drive["PhysicalDrive".len()..].parse::<u32>().map_err(|e| e.to_string())?
    } else {
        let volume_path = format!("\\\\.\\{}:", drive.trim_end_matches(':'));
        windows_api::get_disk_extents(&volume_path).map_err(|e: io::Error| e.to_string())?
    };

    windows_api::sd_read(disk_num, address, size).map_err(|e: io::Error| e.to_string())
}

#[tauri::command]
fn sd_write(drive: String, address: u64, data: Vec<u8>) -> Result<u32, String> {
    let disk_num = if drive.starts_with("PhysicalDrive") {
        drive["PhysicalDrive".len()..].parse::<u32>().map_err(|e| e.to_string())?
    } else {
        let volume_path = format!("\\\\.\\{}:", drive.trim_end_matches(':'));
        windows_api::get_disk_extents(&volume_path).map_err(|e: io::Error| e.to_string())?
    };

    windows_api::sd_write(&drive, disk_num, address, &data).map_err(|e: io::Error| e.to_string())
}

#[tauri::command]
fn sd_fill(drive: String, address: u64, size: usize, fill_byte: u8) -> Result<usize, String> {
    let disk_num = if drive.starts_with("PhysicalDrive") {
        drive["PhysicalDrive".len()..].parse::<u32>().map_err(|e| e.to_string())?
    } else {
        let volume_path = format!("\\\\.\\{}:", drive.trim_end_matches(':'));
        windows_api::get_disk_extents(&volume_path).map_err(|e: io::Error| e.to_string())?
    };

    windows_api::sd_fill(&drive, disk_num, address, size, fill_byte).map_err(|e: io::Error| e.to_string())
}

#[tauri::command]
fn sd_verify_fill(drive: String, address: u64, size: usize, fill_byte: u8) -> Result<usize, String> {
    let disk_num = if drive.starts_with("PhysicalDrive") {
        drive["PhysicalDrive".len()..].parse::<u32>().map_err(|e| e.to_string())?
    } else {
        let volume_path = format!("\\\\.\\{}:", drive.trim_end_matches(':'));
        windows_api::get_disk_extents(&volume_path).map_err(|e: io::Error| e.to_string())?
    };

    windows_api::sd_verify_fill(disk_num, address, size, fill_byte).map_err(|e: io::Error| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![greet, get_drives, sd_read, sd_write, sd_fill, sd_verify_fill])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
