#![allow(non_snake_case)]
use std::ffi::OsStr;
use std::io;
use std::iter::once;
use std::os::windows::prelude::*;
use windows_sys::Win32::Foundation::{
    CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    ReadFile, SetFilePointerEx, WriteFile, FILE_ATTRIBUTE_NORMAL, FILE_BEGIN,
    FILE_FLAG_NO_BUFFERING, FILE_FLAG_WRITE_THROUGH, FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_EXISTING,
};
use windows_sys::Win32::System::Ioctl::{
    DISK_GEOMETRY, IOCTL_DISK_GET_DRIVE_GEOMETRY, VOLUME_DISK_EXTENTS,
};
use windows_sys::Win32::System::IO::DeviceIoControl;

const IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS: u32 = 0x00560000;
const FSCTL_LOCK_VOLUME: u32 = 0x00090018;
const FSCTL_UNLOCK_VOLUME: u32 = 0x0009001c;
// Whole-device volume I/O completes each write synchronously with a write-through request.
const VOLUME_WRITE_FLAGS: u32 =
    FILE_ATTRIBUTE_NORMAL | FILE_FLAG_NO_BUFFERING | FILE_FLAG_WRITE_THROUGH;

extern "system" {
    pub fn CreateFileW(
        lpFileName: *const u16,
        dwDesiredAccess: u32,
        dwShareMode: u32,
        lpSecurityAttributes: *const std::ffi::c_void,
        dwCreationDisposition: u32,
        dwFlagsAndAttributes: u32,
        hTemplateFile: HANDLE,
    ) -> HANDLE;
}

pub fn get_disk_extents(drive_path: &str) -> Result<u32, io::Error> {
    crate::diagnostics::step(format!("get_disk_extents path={drive_path}"), || {
        let volume = DiskHandle::open(drive_path, 0, FILE_ATTRIBUTE_NORMAL)?;
        crate::diagnostics::poll_note(format!("volume extents query path={drive_path}"));
        volume_mapping(volume.0).map(|extent| extent.disk)
    })
}

#[derive(Debug, PartialEq)]
struct VolumeExtent {
    disk: u32,
    offset: u64,
    length: u64,
}

// Disk identity is usable even when Windows reports an empty volume range.
// Capacity and I/O routing are checked separately before device access.
fn parse_volume_mapping(bytes: &[u8]) -> io::Result<VolumeExtent> {
    let expected = std::mem::size_of::<VOLUME_DISK_EXTENTS>();
    // Decode only the count when a complete structure was not returned.
    let count = bytes.get(..4).map(|b| u32::from_ne_bytes(b.try_into().unwrap()));
    crate::diagnostics::poll_note(format!(
        "volume extents response returned_bytes={} expected_struct_bytes={expected} extent_count={count:?}",
        bytes.len()
    ));
    let invalid = |reason: &str| {
        crate::diagnostics::poll_note(format!("volume extents rejected reason={reason}"));
        io::Error::new(
            io::ErrorKind::InvalidData,
            "単一ディスクの有効なボリューム範囲を確認できません",
        )
    };
    if bytes.len() < expected {
        return Err(invalid("response_shorter_than_struct"));
    }
    let extents = unsafe { std::ptr::read_unaligned(bytes.as_ptr().cast::<VOLUME_DISK_EXTENTS>()) };
    let extent = extents.Extents[0];
    crate::diagnostics::poll_note(format!(
        "volume extents first disk_number={} starting_offset={} extent_length={} extent_count={}",
        extent.DiskNumber, extent.StartingOffset, extent.ExtentLength, extents.NumberOfDiskExtents
    ));
    if extents.NumberOfDiskExtents != 1 {
        return Err(invalid("extent_count_not_one"));
    }
    if extent.StartingOffset < 0 {
        return Err(invalid("negative_starting_offset"));
    }
    if extent.ExtentLength < 0 {
        return Err(invalid("negative_extent_length"));
    }
    if extent.ExtentLength == 0 {
        crate::diagnostics::poll_note(
            "volume mapping accepted with zero extent length; disk identity only, capacity requires physical query".into()
        );
    }
    Ok(VolumeExtent {
        disk: extent.DiskNumber,
        offset: extent.StartingOffset as u64,
        length: extent.ExtentLength as u64,
    })
}

fn volume_mapping(handle: HANDLE) -> io::Result<VolumeExtent> {
    crate::diagnostics::step("IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS", || {
        let mut bytes = [0_u8; 256];
        let mut returned = 0;
        let result = unsafe {
            DeviceIoControl(
                handle,
                IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS,
                std::ptr::null(),
                0,
                bytes.as_mut_ptr().cast(),
                bytes.len() as u32,
                &mut returned,
                std::ptr::null_mut(),
            )
        };
        if result == 0 {
            let error = io::Error::last_os_error();
            crate::diagnostics::poll_note(format!(
                "volume extents ioctl failed returned_bytes={returned} buffer_bytes={} win32_error={:?} error={error}",
                bytes.len(), error.raw_os_error()
            ));
            return Err(error);
        }
        crate::diagnostics::poll_note(format!(
            "volume extents ioctl ok returned_bytes={returned} buffer_bytes={}", bytes.len()
        ));
        let bytes = bytes.get(..returned as usize).ok_or_else(|| {
            crate::diagnostics::poll_note("volume extents rejected reason=returned_bytes_exceed_buffer".into());
            io::Error::new(
                io::ErrorKind::InvalidData,
                "ボリューム範囲の応答長が不正です",
            )
        })?;
        parse_volume_mapping(bytes)
    })
}

fn whole_device_volume(extent: &VolumeExtent, disk: u32, capacity: u64) -> Result<bool, String> {
    if extent.disk != disk
        || capacity == 0
        || (extent.length == 0 && extent.offset != 0)
        || extent
            .offset
            .checked_add(extent.length)
            .is_none_or(|end| end > capacity)
    {
        return Err("SDボリュームと物理ディスクの対応範囲が一致しません".into());
    }
    if extent.length == 0 {
        crate::diagnostics::record("INFO", &format!(
            "SD write zero volume extent disk={disk} physical_capacity={capacity} io_route=physical_disk; volume range is not used as capacity"
        ));
        return Ok(false);
    }
    Ok(extent.offset == 0 && extent.length == capacity)
}

fn validate_io_range(address: u64, length: usize, capacity: u64) -> Result<(), String> {
    if length == 0
        || address > i64::MAX as u64
        || address
            .checked_add(length as u64)
            .is_none_or(|end| end > capacity)
    {
        return Err("SDの読書込範囲が容量外または不正です".into());
    }
    Ok(())
}

pub fn get_physical_size(physical_path: &str) -> Result<u64, io::Error> {
    crate::diagnostics::step(format!("get_physical_size path={physical_path}"), || {
        let handle = DiskHandle::open(physical_path, 0, FILE_ATTRIBUTE_NORMAL)?;
        // GEOMETRY_EX permits a metadata-only handle in the unelevated GUI and
        // provides exact DiskSize instead of the rounded CHS geometry product.
        use windows_sys::Win32::System::Ioctl::{
            DISK_GEOMETRY_EX, IOCTL_DISK_GET_DRIVE_GEOMETRY_EX,
        };
        let mut buffer = [0u64; 128];
        let mut count = 0;
        if unsafe {
            DeviceIoControl(
                handle.0,
                IOCTL_DISK_GET_DRIVE_GEOMETRY_EX,
                std::ptr::null(),
                0,
                buffer.as_mut_ptr() as *mut _,
                std::mem::size_of_val(&buffer) as u32,
                &mut count,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        if count < 32 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "SD容量の応答が不正です",
            ));
        }
        let geometry =
            unsafe { std::ptr::read_unaligned(buffer.as_ptr() as *const DISK_GEOMETRY_EX) };
        if geometry.DiskSize <= 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "SDメディアがありません",
            ));
        }
        Ok(geometry.DiskSize as u64)
    })
}

pub fn removable_target(drive: &str) -> Result<(u32, u64), String> {
    crate::diagnostics::step(format!("removable_target drive={drive}"), || {
        use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;
        let root: Vec<u16> = format!("{drive}\\\0").encode_utf16().collect();
        if drive.len() != 2
            || !drive.as_bytes()[0].is_ascii_uppercase()
            || !drive.ends_with(':')
            || unsafe { GetDriveTypeW(root.as_ptr()) } != 2
        {
            return Err("書込可能なリムーバブルSDドライブを選択してください".into());
        }
        let disk = get_disk_extents(&format!("\\\\.\\{drive}")).map_err(|e| e.to_string())?;
        if disk == 0xFF {
            return Err("SDディスクを特定できませんでした".into());
        }
        let capacity =
            get_physical_size(&format!("\\\\.\\PhysicalDrive{disk}")).map_err(|e| e.to_string())?;
        Ok((disk, capacity))
    })
}

pub fn physical_drive_number(name: &str) -> Option<u32> {
    let number = name.strip_prefix("PhysicalDrive")?.parse::<u32>().ok()?;
    (name == format!("PhysicalDrive{number}")).then_some(number)
}

/// Inspect device metadata only. No volume, drive letter or filesystem is needed.
pub fn removable_physical_size(disk: u32) -> Result<u64, io::Error> {
    use windows_sys::Win32::System::Ioctl::{
        PropertyStandardQuery, StorageDeviceProperty, IOCTL_STORAGE_QUERY_PROPERTY,
        STORAGE_DEVICE_DESCRIPTOR, STORAGE_PROPERTY_QUERY,
    };
    let path = format!("\\\\.\\PhysicalDrive{disk}");
    let handle = DiskHandle::open(&path, 0, FILE_ATTRIBUTE_NORMAL)?;
    let query = STORAGE_PROPERTY_QUERY {
        PropertyId: StorageDeviceProperty,
        QueryType: PropertyStandardQuery,
        AdditionalParameters: [0],
    };
    let mut buffer = [0u64; 512];
    let mut count = 0;
    let ok = unsafe {
        DeviceIoControl(
            handle.0,
            IOCTL_STORAGE_QUERY_PROPERTY,
            &query as *const _ as *const _,
            std::mem::size_of_val(&query) as u32,
            buffer.as_mut_ptr() as *mut _,
            std::mem::size_of_val(&buffer) as u32,
            &mut count,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    if (count as usize) < std::mem::size_of::<STORAGE_DEVICE_DESCRIPTOR>() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "デバイス情報が不足しています",
        ));
    }
    let descriptor =
        unsafe { std::ptr::read_unaligned(buffer.as_ptr() as *const STORAGE_DEVICE_DESCRIPTOR) };
    if descriptor.DeviceType != 0 || descriptor.RemovableMedia == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "リムーバブルメディアではありません",
        ));
    }
    get_physical_size(&path)
}

pub fn list_removable_disks() -> Result<Vec<(u32, u64)>, io::Error> {
    use windows_sys::Win32::Storage::FileSystem::QueryDosDeviceW;
    let mut names = vec![0u16; 65536];
    let count = loop {
        let count =
            unsafe { QueryDosDeviceW(std::ptr::null(), names.as_mut_ptr(), names.len() as u32) };
        if count != 0 {
            break count as usize;
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(122) || names.len() >= 1024 * 1024 {
            return Err(error);
        }
        names.resize(names.len() * 2, 0);
    };
    let mut disks = Vec::new();
    for name in names[..count].split(|c| *c == 0).filter(|s| !s.is_empty()) {
        if let Some(disk) = physical_drive_number(&String::from_utf16_lossy(name)) {
            if let Ok(capacity) = removable_physical_size(disk) {
                if capacity > 0 {
                    disks.push((disk, capacity));
                }
            }
        }
    }
    disks.sort_unstable();
    disks.dedup_by_key(|(disk, _)| *disk);
    Ok(disks)
}

// Reuse one I/O handle per batch; writes require a volume lock and dismount. Read-only
// exports request no write access. Aligned, unbuffered I/O avoids stale cache data.
struct DiskHandle(HANDLE);
impl DiskHandle {
    fn open(path: &str, access: u32, flags: u32) -> io::Result<Self> {
        crate::diagnostics::step(
            format!("CreateFileW path={path} access={access:#X} flags={flags:#X}"),
            || {
                let wide: Vec<u16> = OsStr::new(path).encode_wide().chain(once(0)).collect();
                let handle = unsafe {
                    CreateFileW(
                        wide.as_ptr(),
                        access,
                        FILE_SHARE_READ | FILE_SHARE_WRITE,
                        std::ptr::null(),
                        OPEN_EXISTING,
                        flags,
                        0,
                    )
                };
                if handle == INVALID_HANDLE_VALUE {
                    Err(io::Error::last_os_error())
                } else {
                    crate::diagnostics::record(
                        "INFO",
                        &format!("disk handle opened path={path} handle={handle:#X}"),
                    );
                    Ok(Self(handle))
                }
            },
        )
    }
}
impl Drop for DiskHandle {
    fn drop(&mut self) {
        let result = unsafe { CloseHandle(self.0) };
        let error = if result == 0 {
            Some(io::Error::last_os_error())
        } else {
            None
        };
        crate::diagnostics::record(
            if error.is_some() { "WARN" } else { "INFO" },
            &format!(
                "disk handle closed handle={:#X} success={} error={error:?}",
                self.0,
                result != 0
            ),
        );
    }
}

// Observe through the existing handles so diagnostics do not reopen, unlock,
// dismount, refresh or reposition a device while it is being written.
fn diagnostic_ioctl(handle: HANDLE, code: u32, size: usize) -> Result<Vec<u8>, serde_json::Value> {
    crate::diagnostics::step(
        format!("diagnostic DeviceIoControl handle={handle:#X} code={code:#X}"),
        || {
            let mut output = vec![0_u8; size];
            let mut returned = 0;
            let started = std::time::Instant::now();
            let ok = unsafe {
                DeviceIoControl(
                    handle,
                    code,
                    std::ptr::null(),
                    0,
                    if size == 0 {
                        std::ptr::null_mut()
                    } else {
                        output.as_mut_ptr().cast()
                    },
                    size as u32,
                    &mut returned,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                let error = io::Error::last_os_error();
                return Err(
                    serde_json::json!({"win32_error": error.raw_os_error(), "error": error.to_string(),
            "elapsed_ms": started.elapsed().as_millis()}),
                );
            }
            if returned as usize > output.len() {
                return Err(
                    serde_json::json!({"error": "invalid diagnostic response length", "returned": returned}),
                );
            }
            output.truncate(returned as usize);
            Ok(output)
        },
    )
}

fn partition_snapshot(bytes: &[u8]) -> Result<serde_json::Value, String> {
    use windows_sys::Win32::System::Ioctl::PARTITION_INFORMATION_EX;
    if bytes.len() < std::mem::size_of::<PARTITION_INFORMATION_EX>() {
        return Err(format!("short partition response bytes={}", bytes.len()));
    }
    // SDK structure contains integers and a POD union; the response may be unaligned.
    let partition =
        unsafe { std::ptr::read_unaligned(bytes.as_ptr().cast::<PARTITION_INFORMATION_EX>()) };
    Ok(
        serde_json::json!({"style": partition.PartitionStyle, "number": partition.PartitionNumber,
        "offset": partition.StartingOffset, "size": partition.PartitionLength,
        "rewrite": partition.RewritePartition != 0}),
    )
}

fn layout_snapshot(bytes: &[u8]) -> Result<serde_json::Value, String> {
    use windows_sys::Win32::System::Ioctl::{
        DRIVE_LAYOUT_INFORMATION_EX, PARTITION_INFORMATION_EX,
    };
    let offset = std::mem::offset_of!(DRIVE_LAYOUT_INFORMATION_EX, PartitionEntry);
    if bytes.len() < offset {
        return Err(format!("short layout response bytes={}", bytes.len()));
    }
    let style = u32::from_ne_bytes(bytes[0..4].try_into().unwrap());
    let count = u32::from_ne_bytes(bytes[4..8].try_into().unwrap()) as usize;
    let size = std::mem::size_of::<PARTITION_INFORMATION_EX>();
    if count > (bytes.len() - offset) / size {
        return Err(format!(
            "invalid layout partition_count={count} bytes={}",
            bytes.len()
        ));
    }
    let partitions = (0..count)
        .map(|i| partition_snapshot(&bytes[offset + i * size..offset + (i + 1) * size]))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(serde_json::json!({"style": style, "partition_count": count, "partitions": partitions}))
}

fn filesystem_info(volume: HANDLE) -> io::Result<(String, u32)> {
    let mut name = [0_u16; 261];
    let mut flags = 0;
    let ok = unsafe {
        windows_sys::Win32::Storage::FileSystem::GetVolumeInformationByHandleW(
            volume,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut flags,
            name.as_mut_ptr(),
            name.len() as u32,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let end = name.iter().position(|&c| c == 0).unwrap_or(name.len());
    Ok((String::from_utf16_lossy(&name[..end]), flags))
}

fn filesystem_snapshot(volume: HANDLE) -> serde_json::Value {
    crate::diagnostics::step("diagnostic GetVolumeInformationByHandleW", || {
        let started = std::time::Instant::now();
        let (name, flags) = filesystem_info(volume).map_err(|error| {
            serde_json::json!({"status": "unavailable", "win32_error": error.raw_os_error(),
                "error": error.to_string(), "elapsed_ms": started.elapsed().as_millis()})
        })?;
        Ok(serde_json::json!({"status": "observed", "name": name,
            "flags": flags, "elapsed_ms": started.elapsed().as_millis()}))
    }).unwrap_or_else(|error: serde_json::Value| error)
}

// Summarize an existing successful read only. Never issue another device read
// or expose the sector contents; a plausible BPB alone does not prove a valid FS.
fn boot_sector_summary(data: &[u8]) -> Option<serde_json::Value> {
    let sector = data.get(..512)?;
    let bytes_per_sector = u16::from_le_bytes([sector[11], sector[12]]);
    let sectors_per_cluster = sector[13];
    let reserved = u16::from_le_bytes([sector[14], sector[15]]);
    Some(serde_json::json!({
        "sample_bytes": 512, "all_ff": sector.iter().all(|&b| b == 0xff),
        "all_zero": sector.iter().all(|&b| b == 0),
        "signature_55aa": sector[510..512] == [0x55, 0xaa],
        "fat_bpb_plausible": matches!(bytes_per_sector, 512 | 1024 | 2048 | 4096)
            && sectors_per_cluster.is_power_of_two() && sectors_per_cluster <= 128
            && reserved > 0 && matches!(sector[16], 1 | 2),
        "caveat": "sector_observation_only_not_filesystem_validation"
    }))
}

fn volume_snapshot(volume: HANDLE, phase: &str) -> String {
    use windows_sys::Win32::System::Ioctl::{
        FSCTL_IS_VOLUME_MOUNTED, IOCTL_DISK_GET_PARTITION_INFO_EX, PARTITION_INFORMATION_EX,
    };
    let partition = diagnostic_ioctl(
        volume,
        IOCTL_DISK_GET_PARTITION_INFO_EX,
        std::mem::size_of::<PARTITION_INFORMATION_EX>(),
    )
    .and_then(|bytes| {
        partition_snapshot(&bytes).map_err(|error| serde_json::json!({"error": error}))
    });
    let mounted = diagnostic_ioctl(volume, FSCTL_IS_VOLUME_MOUNTED, 0).map(|_| true);
    let filesystem = filesystem_snapshot(volume);
    let partition_detail = match &partition {
        Ok(value) => format!(
            "パーティション: 開始 {} バイト / サイズ {} バイト",
            value["offset"], value["size"]
        ),
        Err(value) => diagnostic_unavailable("パーティション情報", value),
    };
    let mount_detail = match &mounted {
        Ok(_) => "マウント確認: マウント中との応答".to_string(),
        Err(value) => diagnostic_unavailable("マウント状態", value),
    };
    crate::diagnostics::record(
        "INFO",
        &format!(
            "volume snapshot phase={phase} handle={volume:#X} partition={} mounted={} filesystem={filesystem}",
            match partition {
                Ok(value) | Err(value) => value,
            },
            match mounted {
                Ok(value) => serde_json::json!({"mounted": value}),
                Err(value) => value,
            }
        ),
    );
    format!("{partition_detail}\n{mount_detail}")
}

fn diagnostic_unavailable(label: &str, value: &serde_json::Value) -> String {
    match value["win32_error"].as_i64() {
        Some(code) => format!("{label}: 確認できません（Windowsエラー {code}）"),
        None => format!("{label}: 確認できません"),
    }
}

fn disk_snapshot(physical: HANDLE, volume: HANDLE, phase: &str) -> String {
    use windows_sys::Win32::System::Ioctl::{
        IOCTL_DISK_GET_DRIVE_LAYOUT_EX, IOCTL_DISK_IS_WRITABLE,
    };
    crate::diagnostics::step(
        format!("storage snapshot phase={phase}"),
        || -> Result<String, String> {
            let volume_detail = volume_snapshot(volume, phase);
            // Bounded diagnostic buffer; unusual/unsupported layouts are logged, never fatal.
            let layout = diagnostic_ioctl(physical, IOCTL_DISK_GET_DRIVE_LAYOUT_EX, 64 * 1024)
                .and_then(|bytes| {
                    layout_snapshot(&bytes).map_err(|error| serde_json::json!({"error": error}))
                });
            let writable = diagnostic_ioctl(physical, IOCTL_DISK_IS_WRITABLE, 0).map(|_| true);
            let write_detail = match &writable {
                Ok(_) => "デバイスの書込可否確認: 書込可能との応答".to_string(),
                Err(value) => diagnostic_unavailable("デバイスの書込可否", value),
            };
            crate::diagnostics::record(
                "INFO",
                &format!(
                    "disk snapshot phase={phase} handle={physical:#X} layout={} writable={}",
                    match layout {
                        Ok(value) | Err(value) => value,
                    },
                    match writable {
                        Ok(value) => serde_json::json!({"writable": value}),
                        Err(value) => value,
                    }
                ),
            );
            Ok(format!("{volume_detail}\n{write_detail}"))
        },
    )
    .unwrap_or_else(|_| "診断情報を取得できませんでした".into())
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;
    use windows_sys::Win32::System::Ioctl::{
        DRIVE_LAYOUT_INFORMATION_EX, PARTITION_INFORMATION_EX,
    };

    #[test]
    fn boot_evidence_distinguishes_ff_from_plausible_bpb_without_declaring_a_filesystem() {
        let erased = boot_sector_summary(&[0xff; 512]).unwrap();
        assert_eq!(erased["all_ff"], true);
        assert_eq!(erased["signature_55aa"], false);
        assert_eq!(erased["fat_bpb_plausible"], false);
        assert_eq!(boot_sector_summary(&[0; 512]).unwrap()["all_zero"], true);
        assert!(boot_sector_summary(&[0xff; 511]).is_none());
        let mut sector = [0_u8; 512];
        sector[11..13].copy_from_slice(&512_u16.to_le_bytes());
        sector[13] = 8;
        sector[14] = 32;
        sector[16] = 2;
        sector[510..512].copy_from_slice(&[0x55, 0xaa]);
        let bpb = boot_sector_summary(&sector).unwrap();
        assert_eq!(bpb["all_ff"], false);
        assert_eq!(bpb["signature_55aa"], true);
        assert_eq!(bpb["fat_bpb_plausible"], true);
        // A signature with invalid BPB fields must not imply FAT recognition.
        sector[11] = 0xff;
        assert_eq!(boot_sector_summary(&sector).unwrap()["fat_bpb_plausible"], false);
    }

    #[test]
    fn filesystem_query_failure_preserves_numeric_error_without_claiming_raw() {
        let result = filesystem_snapshot(INVALID_HANDLE_VALUE);
        assert_eq!(result["status"], "unavailable");
        assert!(result["win32_error"].as_i64().is_some());
        assert!(result.get("name").is_none());
    }

    #[test]
    fn only_explicit_raw_skips_extended_dasd_and_other_filesystems_keep_their_checks() {
        use windows_sys::Win32::System::Ioctl::{FSCTL_ALLOW_EXTENDED_DASD_IO, FSCTL_DISMOUNT_VOLUME};
        for (use_volume_io, filesystem, expected_extended) in [
            (true, Some("RAW"), false), (true, Some("raw"), false),
            (true, Some("FAT32"), true), (true, Some("NTFS"), true),
            (true, Some("exFAT"), true), (true, Some(""), true),
            (true, None, true), (false, Some("FAT32"), false),
            (false, Some("RAW"), false), (false, None, false),
        ] {
            let observed = filesystem.map(|name| (name.to_string(), 0))
                .ok_or_else(|| io::Error::from_raw_os_error(87));
            let extended = extended_dasd_required(use_volume_io, &observed);
            assert_eq!(extended, expected_extended);
            let mut calls = Vec::new();
            prepare_write_volume(extended, |_, code| { calls.push(code); Ok(()) }).unwrap();
            let mut expected = vec![FSCTL_LOCK_VOLUME];
            if expected_extended { expected.push(FSCTL_ALLOW_EXTENDED_DASD_IO); }
            expected.push(FSCTL_DISMOUNT_VOLUME);
            assert_eq!(calls, expected);
        }
        // Even confirmed RAW must stop on a failed lock/dismount, including error 87.
        let raw = Ok(("RAW".to_string(), 0));
        for failure_at in [0, 1] {
            for error_code in [5, 87] {
                let mut calls = 0;
                let result = prepare_write_volume(extended_dasd_required(true, &raw), |_, _| {
                    let fail = calls == failure_at;
                    calls += 1;
                    if fail { Err(io::Error::from_raw_os_error(error_code)) } else { Ok(()) }
                });
                assert!(result.unwrap_err().contains(&format!("os error {error_code}")));
                assert_eq!(calls, failure_at + 1);
            }
        }
    }

    #[test]
    fn completion_policy_uses_write_through_and_preserves_physical_flush_errors() {
        assert_ne!(VOLUME_WRITE_FLAGS & FILE_FLAG_NO_BUFFERING, 0);
        assert_ne!(VOLUME_WRITE_FLAGS & FILE_FLAG_WRITE_THROUGH, 0);
        complete_batch_writes(true, || {
            panic!("write-through volume must not flush a dismounted filesystem")
        })
        .unwrap();
        let mut called = 0;
        let error = complete_batch_writes(false, || {
            called += 1;
            Err(io::Error::from_raw_os_error(21))
        })
        .unwrap_err();
        assert_eq!(called, 1);
        assert!(error.contains("SD書込確定失敗"));
        assert!(error.contains("os error 21"));
        complete_batch_writes(false, || Ok(())).unwrap();
    }

    #[test]
    fn whole_device_route_requires_exact_disk_and_full_card_extent() {
        let capacity = 8 * 1024 * 1024 * 1024_u64;
        assert_eq!(
            whole_device_volume(
                &VolumeExtent {
                    disk: 1,
                    offset: 0,
                    length: capacity
                },
                1,
                capacity
            ),
            Ok(true)
        );
        assert_eq!(
            whole_device_volume(
                &VolumeExtent {
                    disk: 1,
                    offset: 1048576,
                    length: capacity - 1048576
                },
                1,
                capacity
            ),
            Ok(false)
        );
        assert_eq!(
            whole_device_volume(
                &VolumeExtent {
                    disk: 1,
                    offset: 0,
                    length: capacity - 512
                },
                1,
                capacity
            ),
            Ok(false)
        );
        for extent in [
            VolumeExtent {
                disk: 2,
                offset: 0,
                length: capacity,
            },
            VolumeExtent {
                disk: 1,
                offset: 512,
                length: capacity,
            },
            VolumeExtent {
                disk: 1,
                offset: u64::MAX,
                length: 512,
            },
        ] {
            assert!(whole_device_volume(&extent, 1, capacity).is_err());
        }
        assert!(validate_io_range(capacity - 512, 512, capacity).is_ok());
        for (address, length) in [
            (capacity, 512),
            (capacity - 512, 1024),
            (u64::MAX, 512),
            (0, 0),
        ] {
            assert!(validate_io_range(address, length, capacity).is_err());
        }
    }

    #[test]
    fn volume_extent_diagnostics_identify_each_rejection_and_keep_numeric_fields() {
        use windows_sys::Win32::System::Ioctl::DISK_EXTENT;
        let entry = std::mem::offset_of!(VOLUME_DISK_EXTENTS, Extents);
        let offset = entry + std::mem::offset_of!(DISK_EXTENT, StartingOffset);
        let length = entry + std::mem::offset_of!(DISK_EXTENT, ExtentLength);
        for (count, start, size, short, reason) in [
            (1_u32, 0_i64, 4096_i64, true, "response_shorter_than_struct"),
            (0, 0, 0, false, "extent_count_not_one"),
            (2, 0, 4096, false, "extent_count_not_one"),
            (1, -1, 4096, false, "negative_starting_offset"),
            (1, 0, -1, false, "negative_extent_length"),
        ] {
            let mut bytes = vec![0_u8; std::mem::size_of::<VOLUME_DISK_EXTENTS>()];
            bytes[..4].copy_from_slice(&count.to_ne_bytes());
            bytes[entry..entry + 4].copy_from_slice(&7_u32.to_ne_bytes());
            bytes[offset..offset + 8].copy_from_slice(&start.to_ne_bytes());
            bytes[length..length + 8].copy_from_slice(&size.to_ne_bytes());
            if short {
                bytes.truncate(4);
            }
            let (result, notes) = crate::diagnostics::quiet(|| parse_volume_mapping(&bytes));
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
            assert!(notes.iter().any(|n| n.contains(&format!("reason={reason}"))));
            assert!(notes[0].contains(&format!("extent_count=Some({count})")));
            if !short {
                assert!(notes.iter().any(|n| n.contains(&format!(
                    "disk_number=7 starting_offset={start} extent_length={size}"
                ))));
            }
        }
        let (result, notes) = crate::diagnostics::quiet(|| parse_volume_mapping(&[]));
        assert!(result.is_err());
        assert!(notes[0].contains("returned_bytes=0"));
        assert!(notes[0].contains("extent_count=None"));
    }

    #[test]
    fn zero_length_volume_maps_to_disk_and_uses_locked_physical_io_with_capacity_checks() {
        use windows_sys::Win32::System::Ioctl::{DISK_EXTENT, FSCTL_DISMOUNT_VOLUME};
        // Reproduce G: from the diagnostic log: one disk, offset=0, length=0.
        let mut bytes = vec![0_u8; std::mem::size_of::<VOLUME_DISK_EXTENTS>()];
        let entry = std::mem::offset_of!(VOLUME_DISK_EXTENTS, Extents);
        bytes[..4].copy_from_slice(&1_u32.to_ne_bytes());
        bytes[entry..entry + 4].copy_from_slice(&1_u32.to_ne_bytes());
        let (mapping, notes) = crate::diagnostics::quiet(|| parse_volume_mapping(&bytes));
        let mapping = mapping.unwrap();
        assert_eq!(mapping, VolumeExtent { disk: 1, offset: 0, length: 0 });
        assert!(notes.iter().any(|n| n.contains("disk identity only")));
        let capacity = 7_969_177_600;
        let use_volume_io = whole_device_volume(&mapping, 1, capacity).unwrap();
        assert!(!use_volume_io);
        assert!(whole_device_volume(&mapping, 2, capacity).is_err());
        assert!(whole_device_volume(&mapping, 1, 0).is_err());
        assert!(whole_device_volume(&VolumeExtent { offset: 512, ..mapping }, 1, capacity).is_err());
        assert!(validate_io_range(capacity - 512, 512, capacity).is_ok());
        assert!(validate_io_range(capacity, 512, capacity).is_err());
        let mut calls = Vec::new();
        let unknown_filesystem = Err(io::Error::from_raw_os_error(87));
        prepare_write_volume(extended_dasd_required(use_volume_io, &unknown_filesystem), |_, code| {
            calls.push(code);
            Ok(())
        }).unwrap();
        assert_eq!(calls, vec![FSCTL_LOCK_VOLUME, FSCTL_DISMOUNT_VOLUME]);
        // A non-empty ordinary partition still takes the physical route.
        let offset = entry + std::mem::offset_of!(DISK_EXTENT, StartingOffset);
        let length = entry + std::mem::offset_of!(DISK_EXTENT, ExtentLength);
        bytes[offset..offset + 8].copy_from_slice(&512_i64.to_ne_bytes());
        bytes[length..length + 8].copy_from_slice(&4096_i64.to_ne_bytes());
        assert!(!whole_device_volume(&parse_volume_mapping(&bytes).unwrap(), 1, capacity).unwrap());
    }

    #[test]
    fn volume_extent_rejects_short_multiple_and_negative_responses() {
        use windows_sys::Win32::System::Ioctl::DISK_EXTENT;
        let mut bytes = vec![0_u8; std::mem::size_of::<VOLUME_DISK_EXTENTS>()];
        let entry = std::mem::offset_of!(VOLUME_DISK_EXTENTS, Extents);
        let length = entry + std::mem::offset_of!(DISK_EXTENT, ExtentLength);
        bytes[0..4].copy_from_slice(&1_u32.to_ne_bytes());
        bytes[entry..entry + 4].copy_from_slice(&7_u32.to_ne_bytes());
        bytes[length..length + 8].copy_from_slice(&4096_i64.to_ne_bytes());
        assert_eq!(
            parse_volume_mapping(&bytes).unwrap(),
            VolumeExtent {
                disk: 7,
                offset: 0,
                length: 4096
            }
        );
        assert!(parse_volume_mapping(&bytes[..bytes.len() - 1]).is_err());
        bytes[0..4].copy_from_slice(&2_u32.to_ne_bytes());
        assert!(parse_volume_mapping(&bytes).is_err());
        bytes[0..4].copy_from_slice(&1_u32.to_ne_bytes());
        bytes[length..length + 8].copy_from_slice(&(-1_i64).to_ne_bytes());
        assert!(parse_volume_mapping(&bytes).is_err());
    }

    #[test]
    fn whole_device_preparation_enables_full_range_only_after_lock() {
        use windows_sys::Win32::System::Ioctl::{
            FSCTL_ALLOW_EXTENDED_DASD_IO, FSCTL_DISMOUNT_VOLUME,
        };
        for failure_at in [None, Some(0), Some(1), Some(2)] {
            let mut calls = Vec::new();
            let result = prepare_write_volume(true, |_, code| {
                let index = calls.len();
                calls.push(code);
                if failure_at == Some(index) {
                    Err(io::Error::from_raw_os_error(5))
                } else {
                    Ok(())
                }
            });
            let expected = [
                FSCTL_LOCK_VOLUME,
                FSCTL_ALLOW_EXTENDED_DASD_IO,
                FSCTL_DISMOUNT_VOLUME,
            ];
            assert_eq!(calls, expected[..failure_at.map_or(3, |i| i + 1)]);
            assert_eq!(result.is_ok(), failure_at.is_none());
        }
    }

    #[test]
    fn write_preparation_locks_before_dismount_and_stops_on_either_failure() {
        use windows_sys::Win32::System::Ioctl::FSCTL_DISMOUNT_VOLUME;
        for failure_at in [None, Some(0), Some(1)] {
            let mut calls = Vec::new();
            let result = prepare_write_volume(false, |name, code| {
                let index = calls.len();
                calls.push((name, code));
                if failure_at == Some(index) {
                    Err(io::Error::from_raw_os_error(5))
                } else {
                    Ok(())
                }
            });
            assert_eq!(calls[0], ("FSCTL_LOCK_VOLUME", FSCTL_LOCK_VOLUME));
            if failure_at == Some(0) {
                assert_eq!(calls.len(), 1, "never dismount after a failed lock");
                assert!(result.unwrap_err().contains("SDをロックできません"));
            } else {
                assert_eq!(calls[1], ("FSCTL_DISMOUNT_VOLUME", FSCTL_DISMOUNT_VOLUME));
                assert_eq!(calls.len(), 2);
                if failure_at == Some(1) {
                    let error = result.unwrap_err();
                    assert!(error.contains("書き込みを開始しませんでした"));
                    assert!(error.contains("os error 5"));
                } else {
                    assert!(result.is_ok());
                }
            }
        }
    }

    #[test]
    fn layout_diagnostics_preserve_ranges_and_reject_truncated_responses() {
        let offset = std::mem::offset_of!(DRIVE_LAYOUT_INFORMATION_EX, PartitionEntry);
        let size = std::mem::size_of::<PARTITION_INFORMATION_EX>();
        let mut bytes = vec![0_u8; offset + size];
        bytes[4..8].copy_from_slice(&1_u32.to_ne_bytes());
        let start = offset + std::mem::offset_of!(PARTITION_INFORMATION_EX, StartingOffset);
        let length = offset + std::mem::offset_of!(PARTITION_INFORMATION_EX, PartitionLength);
        bytes[start..start + 8].copy_from_slice(&1048576_i64.to_ne_bytes());
        bytes[length..length + 8].copy_from_slice(&7770000000_i64.to_ne_bytes());
        let parsed = layout_snapshot(&bytes).unwrap();
        assert_eq!(parsed["partition_count"], 1);
        assert_eq!(parsed["partitions"][0]["offset"], 1048576);
        assert_eq!(parsed["partitions"][0]["size"], 7770000000_i64);
        assert!(layout_snapshot(&bytes[..bytes.len() - 1]).is_err());
        assert!(layout_snapshot(&bytes[..4]).is_err());
        assert!(partition_snapshot(&bytes[..size - 1]).is_err());
        bytes[4..8].copy_from_slice(&u32::MAX.to_ne_bytes());
        assert!(layout_snapshot(&bytes).is_err());
    }

    #[test]
    fn unsupported_diagnostic_query_retains_numeric_windows_error() {
        use windows_sys::Win32::System::Ioctl::FSCTL_IS_VOLUME_MOUNTED;
        let error = diagnostic_ioctl(INVALID_HANDLE_VALUE, FSCTL_IS_VOLUME_MOUNTED, 0).unwrap_err();
        assert_eq!(error["win32_error"], 6); // ERROR_INVALID_HANDLE
        assert!(error["error"].is_string());
        let summary = disk_snapshot(
            INVALID_HANDLE_VALUE,
            INVALID_HANDLE_VALUE,
            "test_invalid_handle",
        );
        assert!(summary.contains("確認できません（Windowsエラー 6）"));
        assert!(!summary.contains("マウント中"));
        assert!(!summary.contains("書込可能"));
    }
}

fn disk_length(handle: HANDLE) -> io::Result<u64> {
    crate::diagnostics::step("IOCTL_DISK_GET_LENGTH_INFO", || {
        let mut length: i64 = 0;
        let mut count = 0;
        let ok = unsafe {
            DeviceIoControl(
                handle,
                0x0007405c,
                std::ptr::null(),
                0,
                &mut length as *mut _ as *mut _,
                8,
                &mut count,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        if count != 8 || length <= 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "SD容量を取得できませんでした",
            ));
        }
        Ok(length as u64)
    })
}

fn extended_dasd_required(use_volume_io: bool, filesystem: &io::Result<(String, u32)>) -> bool {
    // RAW has no filesystem boundaries to relax. An unknown/failed query is
    // never treated as RAW, and must retain the normal preparation checks.
    use_volume_io && !filesystem.as_ref().is_ok_and(|(name, _)| name.eq_ignore_ascii_case("RAW"))
}

// Keep the lock for the whole batch; never dismount a volume we could not lock.
fn prepare_write_volume(
    require_extended_dasd: bool,
    mut control: impl FnMut(&'static str, u32) -> io::Result<()>,
) -> Result<(), String> {
    use windows_sys::Win32::System::Ioctl::{FSCTL_ALLOW_EXTENDED_DASD_IO, FSCTL_DISMOUNT_VOLUME};
    control("FSCTL_LOCK_VOLUME", FSCTL_LOCK_VOLUME).map_err(|error| {
        format!("SDをロックできません。SDを使用中のファイルを閉じてください: {error}")
    })?;
    if require_extended_dasd {
        // Permit the same full-card range as physical I/O, including the final sectors.
        control("FSCTL_ALLOW_EXTENDED_DASD_IO", FSCTL_ALLOW_EXTENDED_DASD_IO).map_err(|error| {
            format!("SD全体のボリュームアクセスを許可できないため、書き込みを開始しませんでした: {error}")
        })?;
    }
    control("FSCTL_DISMOUNT_VOLUME", FSCTL_DISMOUNT_VOLUME).map_err(|error| {
        format!("SDのマウントを解除できないため、書き込みを開始しませんでした: {error}")
    })
}

fn complete_batch_writes(
    use_volume_io: bool,
    flush: impl FnOnce() -> io::Result<()>,
) -> Result<(), String> {
    if use_volume_io {
        // NO_BUFFERING + WRITE_THROUGH was requested when opening this handle.
        // Do not call the filesystem's FlushFileBuffers after dismounting it.
        crate::diagnostics::record("INFO", "SD write completion policy=unbuffered_write_through; FlushFileBuffers not used on dismounted volume");
        return Ok(());
    }
    crate::diagnostics::step("FlushFileBuffers", || {
        flush().map_err(|error| format!("SD書込確定失敗: {error}"))
    })
}

pub struct BatchDisk {
    physical: DiskHandle,
    _volume: DiskHandle,
    use_volume_io: bool,
    sector: usize,
    pub capacity: u64,
}
impl BatchDisk {
    pub fn open(drive: &str, expected_disk: u32, expected_capacity: u64) -> Result<Self, String> {
        Self::open_mode(drive, expected_disk, expected_capacity, false)
    }

    pub fn open_read(
        drive: &str,
        expected_disk: u32,
        expected_capacity: u64,
    ) -> Result<Self, String> {
        Self::open_mode(drive, expected_disk, expected_capacity, true)
    }

    fn open_mode(
        drive: &str,
        expected_disk: u32,
        expected_capacity: u64,
        read_only: bool,
    ) -> Result<Self, String> {
        crate::diagnostics::step(
            format!(
                    "BatchDisk::open drive={drive} disk={expected_disk} capacity={expected_capacity} read_only={read_only}"
            ),
            || {
                use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};
                let root: Vec<u16> = format!("{drive}\\\0").encode_utf16().collect();
                if drive.len() != 2
                    || !drive.as_bytes()[0].is_ascii_uppercase()
                    || !drive.ends_with(':')
                    || unsafe { GetDriveTypeW(root.as_ptr()) } != 2
                {
                    return Err("リムーバブルSDドライブではありません".into());
                }
                let volume_path = format!("\\\\.\\{drive}");
                let disk = get_disk_extents(&volume_path).map_err(|e| e.to_string())?;
                if disk != expected_disk {
                    return Err("確認後にSDドライブが変更されました".into());
                }
                let drives = unsafe { GetLogicalDrives() };
                for i in 0..26 {
                    let other = format!("{}:", (b'A' + i) as char);
                    if !read_only
                        && drives & (1u32 << i) != 0
                        && other != drive
                        && get_disk_extents(&format!("\\\\.\\{other}")).ok() == Some(disk)
                    {
                        return Err(
                            "複数ボリュームのあるSDカードへの書込には対応していません".into()
                        );
                    }
                }
                let volume = DiskHandle::open(
                    &volume_path,
                    if read_only {
                        GENERIC_READ
                    } else {
                        GENERIC_READ | GENERIC_WRITE
                    },
                    if read_only { FILE_ATTRIBUTE_NORMAL } else { VOLUME_WRITE_FLAGS },
                )
                .map_err(|e| format!("SDボリュームを開けません: {e}"))?;
                let use_volume_io = if read_only { false } else {
                    let extent = volume_mapping(volume.0).map_err(|e| e.to_string())?;
                    let whole_device = whole_device_volume(&extent, disk, expected_capacity)?;
                    crate::diagnostics::record("INFO", &format!("SD write extent disk={} offset={} length={} io_route={}", extent.disk, extent.offset, extent.length,
                        if whole_device { "locked_whole_device_volume" } else { "physical_disk" }));
                    whole_device
                };
                let mut count = 0;
                if !read_only {
                    crate::diagnostics::record("INFO", concat!("SD write preparation policy=raw_aware_write_through_v4 version=", env!("CARGO_PKG_VERSION")));
                    volume_snapshot(volume.0, "before_lock");
                    let filesystem = filesystem_info(volume.0);
                    let require_extended_dasd = extended_dasd_required(use_volume_io, &filesystem);
                    crate::diagnostics::record("INFO", &format!(
                        "SD write filesystem={filesystem:?} use_volume_io={use_volume_io} require_extended_dasd={require_extended_dasd}; explicit_RAW_only_skips_filesystem_boundary_control"));
                    if let Err(error) = prepare_write_volume(require_extended_dasd, |name, code| {
                        let result = crate::diagnostics::device_control(name, || unsafe {
                            DeviceIoControl(
                                volume.0,
                                code,
                                std::ptr::null(),
                                0,
                                std::ptr::null_mut(),
                                0,
                                &mut count,
                                std::ptr::null_mut(),
                            )
                        });
                        if result.is_ok() {
                            volume_snapshot(volume.0, match name { "FSCTL_LOCK_VOLUME" => "after_lock", "FSCTL_DISMOUNT_VOLUME" => "after_dismount", _ => "after_extended_dasd" });
                        }
                        result
                    }) {
                        let detail = volume_snapshot(volume.0, "write_preparation_failed");
                        return Err(format!("{error}\n{detail}"));
                    }
                }
                let physical = DiskHandle::open(
                    &format!("\\\\.\\PhysicalDrive{disk}"),
                    if read_only || use_volume_io {
                        GENERIC_READ
                    } else {
                        GENERIC_READ | GENERIC_WRITE
                    },
                    FILE_ATTRIBUTE_NORMAL | FILE_FLAG_NO_BUFFERING,
                )
                .map_err(|e| e.to_string())?;
                if !read_only {
                    disk_snapshot(physical.0, volume.0, "write_opened_locked_dismounted");
                    crate::diagnostics::record("INFO", &format!("SD write io_handle={:#X} physical_handle={:#X} volume_handle={:#X} route={}",
                        if use_volume_io { volume.0 } else { physical.0 }, physical.0, volume.0,
                        if use_volume_io { "locked_whole_device_volume" } else { "physical_disk" }));
                }
                let capacity = disk_length(physical.0).map_err(|e| e.to_string())?;
                if capacity != expected_capacity {
                    return Err("確認後にSD容量が変更されました".into());
                }
                let mut geometry: DISK_GEOMETRY = unsafe { std::mem::zeroed() };
                if let Err(error) =
                    crate::diagnostics::device_control("IOCTL_DISK_GET_DRIVE_GEOMETRY", || unsafe {
                        DeviceIoControl(
                            physical.0,
                            IOCTL_DISK_GET_DRIVE_GEOMETRY,
                            std::ptr::null(),
                            0,
                            &mut geometry as *mut _ as *mut _,
                            std::mem::size_of::<DISK_GEOMETRY>() as u32,
                            &mut count,
                            std::ptr::null_mut(),
                        )
                    })
                {
                    return Err(error.to_string());
                }
                let sector = geometry.BytesPerSector as usize;
                if !sector.is_power_of_two() || sector > 65536 || capacity % sector as u64 != 0 {
                    return Err("対応していないSDセクターサイズです".into());
                }
                Ok(Self {
                    physical,
                    _volume: volume,
                    use_volume_io,
                    sector,
                    capacity,
                })
            },
        )
    }
    fn io_handle(&self) -> HANDLE {
        if self.use_volume_io {
            self._volume.0
        } else {
            self.physical.0
        }
    }

    fn seek(&self, address: u64, length: usize) -> Result<(), String> {
        validate_io_range(address, length, self.capacity)?;
        crate::diagnostics::step(format!("SetFilePointerEx address={address:#X}"), || {
            let mut position = 0;
            if address > i64::MAX as u64
                || unsafe {
                    SetFilePointerEx(self.io_handle(), address as i64, &mut position, FILE_BEGIN)
                } == 0
            {
                return Err(io::Error::last_os_error().to_string());
            }
            Ok(())
        })
    }
}
impl crate::sd_batch::Disk for BatchDisk {
    fn sector(&self) -> usize {
        self.sector
    }
    fn read(&mut self, address: u64, data: &mut [u8]) -> Result<(), String> {
        crate::diagnostics::step(
            format!("ReadFile address={address:#X} bytes={}", data.len()),
            || {
                self.seek(address, data.len())?;
                let mut count = 0;
                if unsafe {
                    ReadFile(
                        self.io_handle(),
                        data.as_mut_ptr() as *mut _,
                        data.len() as u32,
                        &mut count,
                        std::ptr::null_mut(),
                    )
                } == 0
                {
                    let error = io::Error::last_os_error();
                    let detail = disk_snapshot(self.physical.0, self._volume.0, "read_failed");
                    return Err(format!(
                        "SD読込失敗: {error}\n読込位置: {address:#X} / サイズ: {} バイト\n{detail}",
                        data.len()
                    ));
                }
                if count as usize != data.len() {
                    return Err("SD読込サイズが不足しています".into());
                }
                if address == 0 {
                    if let Some(summary) = boot_sector_summary(data) {
                        crate::diagnostics::record("INFO", &format!(
                            "boot sector observation source=existing_read handle={:#X} address=0x0 {summary}", self.io_handle()));
                    }
                }
                Ok(())
            },
        )
    }
    fn write(&mut self, address: u64, data: &[u8]) -> Result<(), String> {
        crate::diagnostics::step(
            format!("WriteFile address={address:#X} bytes={}", data.len()),
            || {
                self.seek(address, data.len())?;
                let mut count = 0;
                if unsafe {
                    WriteFile(
                        self.io_handle(),
                        data.as_ptr() as *const _,
                        data.len() as u32,
                        &mut count,
                        std::ptr::null_mut(),
                    )
                } == 0
                {
                    // Capture immediately: diagnostic IOCTLs must not replace GetLastError.
                    let error = io::Error::last_os_error();
                    crate::diagnostics::record("ERROR", &format!(
                        "write failed handle={:#X} address={address:#X} bytes={} transferred={count} sector={} address_aligned={} length_aligned={} buffer_aligned={} win32_error={:?}",
                        self.io_handle(), data.len(), self.sector,
                        address % self.sector as u64 == 0, data.len() % self.sector == 0,
                        data.as_ptr() as usize % self.sector == 0, error.raw_os_error()));
                    let detail = disk_snapshot(self.physical.0, self._volume.0, "write_failed");
                    return Err(format!("SD書込失敗: {error}\nSDのロック: 取得済み（処理中に保持）\nマウント解除: 成功\n書込経路: {}\n書込位置: {address:#X} / サイズ: {} バイト / 転送済み: {count} バイト\n{detail}", if self.use_volume_io { "ロックしたボリューム" } else { "物理ディスク" }, data.len()));
                }
                if count as usize != data.len() {
                    return Err("SD書込サイズが不足しています".into());
                }
                if address == 0 {
                    disk_snapshot(self.physical.0, self._volume.0, "after_sector_zero_write");
                }
                Ok(())
            },
        )
    }
    fn flush(&mut self) -> Result<(), String> {
        complete_batch_writes(self.use_volume_io, || {
            if unsafe {
                windows_sys::Win32::Storage::FileSystem::FlushFileBuffers(self.io_handle())
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        })
    }
}

pub fn sd_read(physical_drive_num: u32, address: u64, size: usize) -> Result<Vec<u8>, io::Error> {
    let physical_path = format!("\\\\.\\PhysicalDrive{}", physical_drive_num);
    let wide: Vec<u16> = OsStr::new(&physical_path)
        .encode_wide()
        .chain(once(0))
        .collect();

    unsafe {
        let h_physical: HANDLE = CreateFileW(
            wide.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_NO_BUFFERING,
            0,
        );

        if h_physical == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }

        const SECTOR_SIZE: u64 = 512;

        let aligned_start = (address / SECTOR_SIZE) * SECTOR_SIZE;
        let offset_in_sector = (address - aligned_start) as usize;
        let aligned_end = ((address + size as u64 + SECTOR_SIZE - 1) / SECTOR_SIZE) * SECTOR_SIZE;
        let aligned_size = (aligned_end - aligned_start) as usize;

        let mut li_new_file_pointer = 0i64;
        if SetFilePointerEx(
            h_physical,
            aligned_start as i64,
            &mut li_new_file_pointer,
            FILE_BEGIN,
        ) == 0
        {
            let err = io::Error::last_os_error();
            CloseHandle(h_physical);
            return Err(err);
        }

        let mut aligned_buffer = vec![0u8; aligned_size];
        let mut bytes_read: u32 = 0;

        let ok_read = ReadFile(
            h_physical,
            aligned_buffer.as_mut_ptr() as *mut _,
            aligned_size as u32,
            &mut bytes_read,
            std::ptr::null_mut(),
        );

        let last_err = if ok_read == 0 {
            Some(io::Error::last_os_error())
        } else {
            None
        };
        CloseHandle(h_physical);

        if let Some(e) = last_err {
            return Err(e);
        }

        let mut result = vec![0u8; size];
        let copy_len = std::cmp::min(size, (bytes_read as usize).saturating_sub(offset_in_sector));
        if copy_len > 0 {
            result[..copy_len]
                .copy_from_slice(&aligned_buffer[offset_in_sector..offset_in_sector + copy_len]);
        }

        Ok(result)
    }
}

pub fn sd_write(
    drive_input: &str,
    physical_drive_num: u32,
    address: u64,
    data: &[u8],
) -> Result<u32, io::Error> {
    let physical_path = format!("\\\\.\\PhysicalDrive{}", physical_drive_num);
    let wide_physical: Vec<u16> = OsStr::new(&physical_path)
        .encode_wide()
        .chain(once(0))
        .collect();

    unsafe {
        let (h_volume, volume_locked) = if !drive_input.starts_with("PhysicalDrive") {
            let drive_letter = drive_input.trim_end_matches(':');
            let volume_path = format!("\\\\.\\{}:", drive_letter);
            let wide_volume: Vec<u16> = OsStr::new(&volume_path)
                .encode_wide()
                .chain(once(0))
                .collect();

            let h = CreateFileW(
                wide_volume.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                0,
            );

            if h != INVALID_HANDLE_VALUE {
                let mut bytes_returned: u32 = 0;
                let locked = DeviceIoControl(
                    h,
                    FSCTL_LOCK_VOLUME,
                    std::ptr::null(),
                    0,
                    std::ptr::null_mut(),
                    0,
                    &mut bytes_returned,
                    std::ptr::null_mut(),
                ) != 0;
                (h, locked)
            } else {
                (INVALID_HANDLE_VALUE, false)
            }
        } else {
            (INVALID_HANDLE_VALUE, false)
        };

        let h_physical: HANDLE = CreateFileW(
            wide_physical.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_NO_BUFFERING | FILE_FLAG_WRITE_THROUGH,
            0,
        );

        if h_physical == INVALID_HANDLE_VALUE {
            let err = io::Error::last_os_error();
            if volume_locked {
                DeviceIoControl(
                    h_volume,
                    FSCTL_UNLOCK_VOLUME,
                    std::ptr::null(),
                    0,
                    std::ptr::null_mut(),
                    0,
                    &mut 0,
                    std::ptr::null_mut(),
                );
            }
            if h_volume != INVALID_HANDLE_VALUE {
                CloseHandle(h_volume);
            }
            return Err(err);
        }

        let mut li_new_file_pointer = 0i64;
        if SetFilePointerEx(
            h_physical,
            address as i64,
            &mut li_new_file_pointer,
            FILE_BEGIN,
        ) == 0
        {
            let err = io::Error::last_os_error();
            CloseHandle(h_physical);
            if volume_locked {
                DeviceIoControl(
                    h_volume,
                    FSCTL_UNLOCK_VOLUME,
                    std::ptr::null(),
                    0,
                    std::ptr::null_mut(),
                    0,
                    &mut 0,
                    std::ptr::null_mut(),
                );
            }
            if h_volume != INVALID_HANDLE_VALUE {
                CloseHandle(h_volume);
            }
            return Err(err);
        }

        const SECTOR_SIZE: usize = 512;
        let data_len = data.len();

        let mut total_bytes_written: u32 = 0;

        const CHUNK_SIZE: usize = 1024 * 1024;
        let mut current_pos = 0;

        while current_pos < data_len {
            let remaining = data_len - current_pos;
            let to_write_raw = std::cmp::min(CHUNK_SIZE, remaining);
            let _write_address = address + current_pos as u64;

            let is_last_chunk = (current_pos + to_write_raw) >= data_len;
            let (chunk_data_to_write, chunk_write_size) = if is_last_chunk {
                let aligned = ((to_write_raw + SECTOR_SIZE - 1) / SECTOR_SIZE) * SECTOR_SIZE;
                if aligned > to_write_raw {
                    let mut buf = vec![0xFFu8; aligned];
                    buf[..to_write_raw]
                        .copy_from_slice(&data[current_pos..current_pos + to_write_raw]);
                    (None, buf)
                } else {
                    (Some(&data[current_pos..current_pos + to_write_raw]), vec![])
                }
            } else {
                (Some(&data[current_pos..current_pos + to_write_raw]), vec![])
            };

            let final_buffer = if chunk_data_to_write.is_some() {
                chunk_data_to_write.unwrap()
            } else {
                &chunk_write_size
            };

            let final_size = final_buffer.len();

            let mut bytes_written_in_chunk: u32 = 0;
            let ok_write = WriteFile(
                h_physical,
                final_buffer.as_ptr() as *const _,
                final_size as u32,
                &mut bytes_written_in_chunk,
                std::ptr::null_mut(),
            );

            if ok_write == 0 {
                let err = io::Error::last_os_error();
                CloseHandle(h_physical);
                if volume_locked {
                    DeviceIoControl(
                        h_volume,
                        FSCTL_UNLOCK_VOLUME,
                        std::ptr::null(),
                        0,
                        std::ptr::null_mut(),
                        0,
                        &mut 0,
                        std::ptr::null_mut(),
                    );
                }
                if h_volume != INVALID_HANDLE_VALUE {
                    CloseHandle(h_volume);
                }
                return Err(err);
            }

            current_pos += to_write_raw;
            total_bytes_written += bytes_written_in_chunk;
        }

        CloseHandle(h_physical);
        if volume_locked {
            DeviceIoControl(
                h_volume,
                FSCTL_UNLOCK_VOLUME,
                std::ptr::null(),
                0,
                std::ptr::null_mut(),
                0,
                &mut 0,
                std::ptr::null_mut(),
            );
        }
        if h_volume != INVALID_HANDLE_VALUE {
            CloseHandle(h_volume);
        }

        Ok(total_bytes_written)
    }
}

pub fn sd_fill(
    drive_input: &str,
    physical_drive_num: u32,
    address: u64,
    size: usize,
    fill_byte: u8,
) -> Result<usize, io::Error> {
    let physical_path = format!("\\\\.\\PhysicalDrive{}", physical_drive_num);
    let wide_physical: Vec<u16> = OsStr::new(&physical_path)
        .encode_wide()
        .chain(once(0))
        .collect();

    unsafe {
        let (h_volume, volume_locked) = if !drive_input.starts_with("PhysicalDrive") {
            let drive_letter = drive_input.trim_end_matches(':');
            let volume_path = format!("\\\\.\\{}:", drive_letter);
            let wide_volume: Vec<u16> = OsStr::new(&volume_path)
                .encode_wide()
                .chain(once(0))
                .collect();

            let h = CreateFileW(
                wide_volume.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                0,
            );

            if h != INVALID_HANDLE_VALUE {
                let mut bytes_returned: u32 = 0;
                let locked = DeviceIoControl(
                    h,
                    FSCTL_LOCK_VOLUME,
                    std::ptr::null(),
                    0,
                    std::ptr::null_mut(),
                    0,
                    &mut bytes_returned,
                    std::ptr::null_mut(),
                ) != 0;
                (h, locked)
            } else {
                (INVALID_HANDLE_VALUE, false)
            }
        } else {
            (INVALID_HANDLE_VALUE, false)
        };

        let h_physical: HANDLE = CreateFileW(
            wide_physical.as_ptr(),
            GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_NO_BUFFERING | FILE_FLAG_WRITE_THROUGH,
            0,
        );

        if h_physical == INVALID_HANDLE_VALUE {
            let err = io::Error::last_os_error();
            if volume_locked {
                DeviceIoControl(
                    h_volume,
                    FSCTL_UNLOCK_VOLUME,
                    std::ptr::null(),
                    0,
                    std::ptr::null_mut(),
                    0,
                    &mut 0,
                    std::ptr::null_mut(),
                );
            }
            if h_volume != INVALID_HANDLE_VALUE {
                CloseHandle(h_volume);
            }
            return Err(err);
        }

        let mut li_new_file_pointer = 0i64;
        if SetFilePointerEx(
            h_physical,
            address as i64,
            &mut li_new_file_pointer,
            FILE_BEGIN,
        ) == 0
        {
            let err = io::Error::last_os_error();
            CloseHandle(h_physical);
            if volume_locked {
                DeviceIoControl(
                    h_volume,
                    FSCTL_UNLOCK_VOLUME,
                    std::ptr::null(),
                    0,
                    std::ptr::null_mut(),
                    0,
                    &mut 0,
                    std::ptr::null_mut(),
                );
            }
            if h_volume != INVALID_HANDLE_VALUE {
                CloseHandle(h_volume);
            }
            return Err(err);
        }

        const SECTOR_SIZE: usize = 512;
        let chunk_size = 1024 * 1024;
        let buffer = vec![fill_byte; chunk_size];
        let mut total_written = 0;

        let aligned_total_size = ((size + SECTOR_SIZE - 1) / SECTOR_SIZE) * SECTOR_SIZE;

        while total_written < aligned_total_size {
            let to_write = std::cmp::min(chunk_size, aligned_total_size - total_written);
            let mut bytes_written: u32 = 0;
            let ok = WriteFile(
                h_physical,
                buffer.as_ptr() as *const _,
                to_write as u32,
                &mut bytes_written,
                std::ptr::null_mut(),
            );

            if ok == 0 {
                let err = io::Error::last_os_error();
                CloseHandle(h_physical);
                if volume_locked {
                    DeviceIoControl(
                        h_volume,
                        FSCTL_UNLOCK_VOLUME,
                        std::ptr::null(),
                        0,
                        std::ptr::null_mut(),
                        0,
                        &mut 0,
                        std::ptr::null_mut(),
                    );
                }
                if h_volume != INVALID_HANDLE_VALUE {
                    CloseHandle(h_volume);
                }
                return Err(err);
            }
            total_written += bytes_written as usize;
            if bytes_written == 0 {
                break;
            }
        }

        CloseHandle(h_physical);
        if volume_locked {
            DeviceIoControl(
                h_volume,
                FSCTL_UNLOCK_VOLUME,
                std::ptr::null(),
                0,
                std::ptr::null_mut(),
                0,
                &mut 0,
                std::ptr::null_mut(),
            );
        }
        if h_volume != INVALID_HANDLE_VALUE {
            CloseHandle(h_volume);
        }

        Ok(std::cmp::min(total_written, size))
    }
}

pub fn sd_verify_fill(
    physical_drive_num: u32,
    address: u64,
    size: usize,
    fill_byte: u8,
) -> Result<usize, io::Error> {
    let physical_path = format!("\\\\.\\PhysicalDrive{}", physical_drive_num);
    let wide: Vec<u16> = OsStr::new(&physical_path)
        .encode_wide()
        .chain(once(0))
        .collect();

    unsafe {
        let h_physical: HANDLE = CreateFileW(
            wide.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            0,
        );

        if h_physical == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }

        let mut li_new_file_pointer = 0i64;
        if SetFilePointerEx(
            h_physical,
            address as i64,
            &mut li_new_file_pointer,
            FILE_BEGIN,
        ) == 0
        {
            let err = io::Error::last_os_error();
            CloseHandle(h_physical);
            return Err(err);
        }

        const SECTOR_SIZE: usize = 512;
        let aligned_size = ((size + SECTOR_SIZE - 1) / SECTOR_SIZE) * SECTOR_SIZE;

        let chunk_size = 1024 * 1024;
        let mut buffer = vec![0u8; chunk_size];
        let mut mismatch_count = 0;
        let mut total_read = 0;

        while total_read < aligned_size {
            let to_read = std::cmp::min(chunk_size, aligned_size - total_read);
            let mut bytes_read: u32 = 0;
            let ok = ReadFile(
                h_physical,
                buffer.as_mut_ptr() as *mut _,
                to_read as u32,
                &mut bytes_read,
                std::ptr::null_mut(),
            );

            if ok == 0 {
                let err = io::Error::last_os_error();
                CloseHandle(h_physical);
                return Err(err);
            }

            let current_chunk_actual_size =
                std::cmp::min(bytes_read as usize, size.saturating_sub(total_read));
            for i in 0..current_chunk_actual_size {
                if buffer[i] != fill_byte {
                    mismatch_count += 1;
                }
            }

            total_read += bytes_read as usize;
            if bytes_read == 0 {
                break;
            }
        }

        CloseHandle(h_physical);
        Ok(mismatch_count)
    }
}
