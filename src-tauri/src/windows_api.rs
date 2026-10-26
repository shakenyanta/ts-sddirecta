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
        let mut disk_number: u32 = 0xFF;
        let wide: Vec<u16> = OsStr::new(drive_path)
            .encode_wide()
            .chain(once(0))
            .collect();

        unsafe {
            let h_drive: HANDLE = CreateFileW(
                wide.as_ptr(),
                0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                0,
            );

            if h_drive == INVALID_HANDLE_VALUE {
                return Err(io::Error::last_os_error());
            }

            let buf_size =
                std::mem::size_of::<VOLUME_DISK_EXTENTS>() + 16 * std::mem::size_of::<u64>();
            let mut buffer = vec![0u8; buf_size];
            let mut bytes_returned: u32 = 0;

            let ok = DeviceIoControl(
                h_drive,
                IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS,
                std::ptr::null(),
                0,
                buffer.as_mut_ptr() as *mut _,
                buf_size as u32,
                &mut bytes_returned,
                std::ptr::null_mut(),
            );

            let last_err = if ok == 0 {
                Some(io::Error::last_os_error())
            } else {
                None
            };
            CloseHandle(h_drive);

            if let Some(e) = last_err {
                return Err(e);
            }

            if (bytes_returned as usize) >= std::mem::size_of::<VOLUME_DISK_EXTENTS>() {
                let p = buffer.as_ptr() as *const VOLUME_DISK_EXTENTS;
                let vde = std::ptr::read_unaligned(p);
                if vde.NumberOfDiskExtents != 1 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "単一ディスクのドライブのみ対応しています",
                    ));
                }

                if vde.NumberOfDiskExtents > 0 {
                    #[repr(C)]
                    struct DISK_EXTENT {
                        pub DiskNumber: u32,
                        pub _start: i64,
                        pub _length: i64,
                    }
                    let de_ptr = buffer.as_ptr().add(8) as *const DISK_EXTENT;
                    let de = std::ptr::read_unaligned(de_ptr);
                    disk_number = de.DiskNumber;
                }
            }
        }
        Ok(disk_number)
    })
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

// One physical handle and one mandatory volume lock per batch. Large, aligned,
// unbuffered transfers avoid stale read-back data and repeated open/close costs.
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
                    Ok(Self(handle))
                }
            },
        )
    }
}
impl Drop for DiskHandle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
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

pub struct BatchDisk {
    physical: DiskHandle,
    _volume: DiskHandle,
    sector: usize,
    pub capacity: u64,
}
impl BatchDisk {
    pub fn open(drive: &str, expected_disk: u32, expected_capacity: u64) -> Result<Self, String> {
        crate::diagnostics::step(
            format!(
                "BatchDisk::open drive={drive} disk={expected_disk} capacity={expected_capacity}"
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
                    if drives & (1u32 << i) != 0
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
                    GENERIC_READ | GENERIC_WRITE,
                    FILE_ATTRIBUTE_NORMAL,
                )
                .map_err(|e| format!("SDボリュームを開けません: {e}"))?;
                let mut count = 0;
                if let Err(error) =
                    crate::diagnostics::device_control("FSCTL_LOCK_VOLUME", || unsafe {
                        DeviceIoControl(
                            volume.0,
                            FSCTL_LOCK_VOLUME,
                            std::ptr::null(),
                            0,
                            std::ptr::null_mut(),
                            0,
                            &mut count,
                            std::ptr::null_mut(),
                        )
                    })
                {
                    return Err(format!(
                        "SDをロックできません。SDを使用中のファイルを閉じてください: {}",
                        error
                    ));
                }
                let physical = DiskHandle::open(
                    &format!("\\\\.\\PhysicalDrive{disk}"),
                    GENERIC_READ | GENERIC_WRITE,
                    FILE_ATTRIBUTE_NORMAL | FILE_FLAG_NO_BUFFERING,
                )
                .map_err(|e| e.to_string())?;
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
                    sector,
                    capacity,
                })
            },
        )
    }
    fn seek(&self, address: u64) -> Result<(), String> {
        crate::diagnostics::step(format!("SetFilePointerEx address={address:#X}"), || {
            let mut position = 0;
            if address > i64::MAX as u64
                || unsafe {
                    SetFilePointerEx(self.physical.0, address as i64, &mut position, FILE_BEGIN)
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
                self.seek(address)?;
                let mut count = 0;
                if unsafe {
                    ReadFile(
                        self.physical.0,
                        data.as_mut_ptr() as *mut _,
                        data.len() as u32,
                        &mut count,
                        std::ptr::null_mut(),
                    )
                } == 0
                {
                    return Err(format!("SD読込失敗: {}", io::Error::last_os_error()));
                }
                if count as usize != data.len() {
                    return Err("SD読込サイズが不足しています".into());
                }
                Ok(())
            },
        )
    }
    fn write(&mut self, address: u64, data: &[u8]) -> Result<(), String> {
        crate::diagnostics::step(
            format!("WriteFile address={address:#X} bytes={}", data.len()),
            || {
                self.seek(address)?;
                let mut count = 0;
                if unsafe {
                    WriteFile(
                        self.physical.0,
                        data.as_ptr() as *const _,
                        data.len() as u32,
                        &mut count,
                        std::ptr::null_mut(),
                    )
                } == 0
                {
                    return Err(format!("SD書込失敗: {}", io::Error::last_os_error()));
                }
                if count as usize != data.len() {
                    return Err("SD書込サイズが不足しています".into());
                }
                Ok(())
            },
        )
    }
    fn flush(&mut self) -> Result<(), String> {
        crate::diagnostics::step("FlushFileBuffers", || {
            if unsafe { windows_sys::Win32::Storage::FileSystem::FlushFileBuffers(self.physical.0) }
                == 0
            {
                return Err(format!("SD書込確定失敗: {}", io::Error::last_os_error()));
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
