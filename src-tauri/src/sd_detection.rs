//! SD media detection does not inspect FAT, partitions, or filesystem health.
use crate::windows_api;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct DriveInfo {
    pub drive: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub size: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

fn merge(mut logical: Vec<(DriveInfo, Option<u32>)>, physical: Vec<(u32, u64)>) -> Vec<DriveInfo> {
    for (disk, capacity) in physical {
        let mut mapped = false;
        for (entry, number) in &mut logical {
            if *number == Some(disk) {
                entry.kind = "Removable".into();
                entry.size = capacity as i64;
                mapped = true;
            }
        }
        if !mapped {
            logical.push((
                DriveInfo {
                    drive: format!("PhysicalDrive{disk}"),
                    kind: "Removable".into(),
                    size: capacity as i64,
                    label: Some("?:".into()),
                },
                Some(disk),
            ));
        }
    }
    logical.into_iter().map(|(entry, _)| entry).collect()
}

pub fn list_drives() -> Result<Vec<DriveInfo>, String> {
    use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};
    let mut logical = Vec::new();
    let mask = unsafe { GetLogicalDrives() };
    for i in 0..26 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let drive = format!("{}:", (b'A' + i) as char);
        let root: Vec<u16> = format!("{drive}\\\0").encode_utf16().collect();
        let kind = unsafe { GetDriveTypeW(root.as_ptr()) };
        if kind != 2 && kind != 3 {
            continue;
        }
        let disk = windows_api::get_disk_extents(&format!("\\\\.\\{drive}"))
            .ok()
            .filter(|n| *n != 0xff);
        let size = disk
            .and_then(|n| windows_api::get_physical_size(&format!("\\\\.\\PhysicalDrive{n}")).ok())
            .unwrap_or(0);
        logical.push((
            DriveInfo {
                drive,
                kind: if kind == 2 { "Removable" } else { "Fixed" }.into(),
                size: size as i64,
                label: None,
            },
            disk,
        ));
    }
    let physical = windows_api::list_removable_disks()
        .map_err(|e| format!("SDデバイス情報を取得できません: {e}"))?;
    Ok(merge(logical, physical))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detects_unassigned_media_without_a_filesystem_and_avoids_duplicates() {
        let out = merge(
            vec![(
                DriveInfo {
                    drive: "H:".into(),
                    kind: "Removable".into(),
                    size: 1,
                    label: None,
                },
                Some(1),
            )],
            vec![(1, 497549312), (2, 1024)],
        );
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].drive, "H:");
        assert_eq!(out[0].size, 497549312);
        assert_eq!(out[1].drive, "PhysicalDrive2");
        assert_eq!(out[1].label.as_deref(), Some("?:"));
        let empty = merge(vec![], vec![(1, 497549312)]);
        assert_eq!(empty[0].drive, "PhysicalDrive1");
    }
    #[test]
    fn physical_ids_are_strict() {
        assert_eq!(
            windows_api::physical_drive_number("PhysicalDrive1"),
            Some(1)
        );
        for invalid in [
            "H:",
            "PhysicalDrive01",
            "PhysicalDrive-1",
            "PhysicalDrive1;",
            "PhysicalDrive",
        ] {
            assert!(windows_api::physical_drive_number(invalid).is_none());
        }
    }
    #[test]
    #[ignore = "Read-only hardware inspection; run explicitly on Windows"]
    fn inspect_connected_media_read_only() {
        println!("{:?}", windows_api::list_removable_disks().unwrap());
    }
}
