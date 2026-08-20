#![allow(non_snake_case)]
use std::ffi::OsStr;
use std::iter::once;
use std::os::windows::prelude::*;
use std::io;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, GENERIC_READ, GENERIC_WRITE};
use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, SetFilePointerEx, ReadFile, WriteFile, FILE_BEGIN, FILE_FLAG_NO_BUFFERING, FILE_FLAG_WRITE_THROUGH};
use windows_sys::Win32::System::Ioctl::{DISK_GEOMETRY, IOCTL_DISK_GET_DRIVE_GEOMETRY, VOLUME_DISK_EXTENTS};
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
    let mut disk_number: u32 = 0xFF;
    let wide: Vec<u16> = OsStr::new(drive_path).encode_wide().chain(once(0)).collect();

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

        let buf_size = std::mem::size_of::<VOLUME_DISK_EXTENTS>() + 16 * std::mem::size_of::<u64>();
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

        let last_err = if ok == 0 { Some(io::Error::last_os_error()) } else { None };
        CloseHandle(h_drive);

        if let Some(e) = last_err {
            return Err(e);
        }

        if (bytes_returned as usize) >= std::mem::size_of::<VOLUME_DISK_EXTENTS>() {
            let p = buffer.as_ptr() as *const VOLUME_DISK_EXTENTS;
            let vde = std::ptr::read_unaligned(p);
            
            if vde.NumberOfDiskExtents > 0 {
                #[repr(C)]
                struct DISK_EXTENT { pub DiskNumber: u32, pub _start: i64, pub _length: i64 }
                let de_ptr = buffer.as_ptr().add(8) as *const DISK_EXTENT;
                let de = std::ptr::read_unaligned(de_ptr);
                disk_number = de.DiskNumber;
            }
        }
    }
    Ok(disk_number)
}

pub fn get_physical_size(physical_path: &str) -> Result<u64, io::Error> {
    let wide: Vec<u16> = OsStr::new(physical_path).encode_wide().chain(once(0)).collect();

    unsafe {
        let h_physical: HANDLE = CreateFileW(
            wide.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            0,
        );

        if h_physical == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }

        let mut geometry: DISK_GEOMETRY = std::mem::zeroed();
        let mut bytes_returned: u32 = 0;

        let ok = DeviceIoControl(
            h_physical,
            IOCTL_DISK_GET_DRIVE_GEOMETRY,
            std::ptr::null(),
            0,
            &mut geometry as *mut _ as *mut _,
            std::mem::size_of::<DISK_GEOMETRY>() as u32,
            &mut bytes_returned,
            std::ptr::null_mut(),
        );

        let last_err = if ok == 0 { Some(io::Error::last_os_error()) } else { None };
        CloseHandle(h_physical);

        if let Some(e) = last_err {
            return Err(e);
        }

        let total_size = geometry.Cylinders as u64 
            * geometry.TracksPerCylinder as u64 
            * geometry.SectorsPerTrack as u64 
            * geometry.BytesPerSector as u64;
        
        Ok(total_size)
    }
}

pub fn sd_read(physical_drive_num: u32, address: u64, size: usize) -> Result<Vec<u8>, io::Error> {
    let physical_path = format!("\\\\.\\PhysicalDrive{}", physical_drive_num);
    let wide: Vec<u16> = OsStr::new(&physical_path).encode_wide().chain(once(0)).collect();

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
        if SetFilePointerEx(h_physical, aligned_start as i64, &mut li_new_file_pointer, FILE_BEGIN) == 0 {
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

        let last_err = if ok_read == 0 { Some(io::Error::last_os_error()) } else { None };
        CloseHandle(h_physical);

        if let Some(e) = last_err {
            return Err(e);
        }

        let mut result = vec![0u8; size];
        let copy_len = std::cmp::min(size, (bytes_read as usize).saturating_sub(offset_in_sector));
        if copy_len > 0 {
            result[..copy_len].copy_from_slice(&aligned_buffer[offset_in_sector..offset_in_sector + copy_len]);
        }
        
        Ok(result)
    }
}

pub fn sd_write(drive_input: &str, physical_drive_num: u32, address: u64, data: &[u8]) -> Result<u32, io::Error> {
    let physical_path = format!("\\\\.\\PhysicalDrive{}", physical_drive_num);
    let wide_physical: Vec<u16> = OsStr::new(&physical_path).encode_wide().chain(once(0)).collect();

    unsafe {
        let (h_volume, volume_locked) = if !drive_input.starts_with("PhysicalDrive") {
            let drive_letter = drive_input.trim_end_matches(':');
            let volume_path = format!("\\\\.\\{}:", drive_letter);
            let wide_volume: Vec<u16> = OsStr::new(&volume_path).encode_wide().chain(once(0)).collect();

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
                DeviceIoControl(h_volume, FSCTL_UNLOCK_VOLUME, std::ptr::null(), 0, std::ptr::null_mut(), 0, &mut 0, std::ptr::null_mut());
            }
            if h_volume != INVALID_HANDLE_VALUE { CloseHandle(h_volume); }
            return Err(err);
        }

        let mut li_new_file_pointer = 0i64;
        if SetFilePointerEx(h_physical, address as i64, &mut li_new_file_pointer, FILE_BEGIN) == 0 {
            let err = io::Error::last_os_error();
            CloseHandle(h_physical);
            if volume_locked {
                DeviceIoControl(h_volume, FSCTL_UNLOCK_VOLUME, std::ptr::null(), 0, std::ptr::null_mut(), 0, &mut 0, std::ptr::null_mut());
            }
            if h_volume != INVALID_HANDLE_VALUE { CloseHandle(h_volume); }
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
                    buf[..to_write_raw].copy_from_slice(&data[current_pos..current_pos + to_write_raw]);
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
                    DeviceIoControl(h_volume, FSCTL_UNLOCK_VOLUME, std::ptr::null(), 0, std::ptr::null_mut(), 0, &mut 0, std::ptr::null_mut());
                }
                if h_volume != INVALID_HANDLE_VALUE { CloseHandle(h_volume); }
                return Err(err);
            }

            current_pos += to_write_raw;
            total_bytes_written += bytes_written_in_chunk;
        }

        CloseHandle(h_physical);
        if volume_locked {
            DeviceIoControl(h_volume, FSCTL_UNLOCK_VOLUME, std::ptr::null(), 0, std::ptr::null_mut(), 0, &mut 0, std::ptr::null_mut());
        }
        if h_volume != INVALID_HANDLE_VALUE {
            CloseHandle(h_volume);
        }
        
        Ok(total_bytes_written)
    }
}

pub fn sd_fill(drive_input: &str, physical_drive_num: u32, address: u64, size: usize, fill_byte: u8) -> Result<usize, io::Error> {
    let physical_path = format!("\\\\.\\PhysicalDrive{}", physical_drive_num);
    let wide_physical: Vec<u16> = OsStr::new(&physical_path).encode_wide().chain(once(0)).collect();

    unsafe {
        let (h_volume, volume_locked) = if !drive_input.starts_with("PhysicalDrive") {
            let drive_letter = drive_input.trim_end_matches(':');
            let volume_path = format!("\\\\.\\{}:", drive_letter);
            let wide_volume: Vec<u16> = OsStr::new(&volume_path).encode_wide().chain(once(0)).collect();

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
                DeviceIoControl(h_volume, FSCTL_UNLOCK_VOLUME, std::ptr::null(), 0, std::ptr::null_mut(), 0, &mut 0, std::ptr::null_mut());
            }
            if h_volume != INVALID_HANDLE_VALUE { CloseHandle(h_volume); }
            return Err(err);
        }

        let mut li_new_file_pointer = 0i64;
        if SetFilePointerEx(h_physical, address as i64, &mut li_new_file_pointer, FILE_BEGIN) == 0 {
            let err = io::Error::last_os_error();
            CloseHandle(h_physical);
            if volume_locked {
                DeviceIoControl(h_volume, FSCTL_UNLOCK_VOLUME, std::ptr::null(), 0, std::ptr::null_mut(), 0, &mut 0, std::ptr::null_mut());
            }
            if h_volume != INVALID_HANDLE_VALUE { CloseHandle(h_volume); }
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
                    DeviceIoControl(h_volume, FSCTL_UNLOCK_VOLUME, std::ptr::null(), 0, std::ptr::null_mut(), 0, &mut 0, std::ptr::null_mut());
                }
                if h_volume != INVALID_HANDLE_VALUE { CloseHandle(h_volume); }
                return Err(err);
            }
            total_written += bytes_written as usize;
            if bytes_written == 0 { break; }
        }

        CloseHandle(h_physical);
        if volume_locked {
            DeviceIoControl(h_volume, FSCTL_UNLOCK_VOLUME, std::ptr::null(), 0, std::ptr::null_mut(), 0, &mut 0, std::ptr::null_mut());
        }
        if h_volume != INVALID_HANDLE_VALUE {
            CloseHandle(h_volume);
        }
        
        Ok(std::cmp::min(total_written, size))
    }
}

pub fn sd_verify_fill(physical_drive_num: u32, address: u64, size: usize, fill_byte: u8) -> Result<usize, io::Error> {
    let physical_path = format!("\\\\.\\PhysicalDrive{}", physical_drive_num);
    let wide: Vec<u16> = OsStr::new(&physical_path).encode_wide().chain(once(0)).collect();

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
        if SetFilePointerEx(h_physical, address as i64, &mut li_new_file_pointer, FILE_BEGIN) == 0 {
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
            
            let current_chunk_actual_size = std::cmp::min(bytes_read as usize, size.saturating_sub(total_read));
            for i in 0..current_chunk_actual_size {
                if buffer[i] != fill_byte {
                    mismatch_count += 1;
                }
            }

            total_read += bytes_read as usize;
            if bytes_read == 0 { break; }
        }

        CloseHandle(h_physical);
        Ok(mismatch_count)
    }
}
