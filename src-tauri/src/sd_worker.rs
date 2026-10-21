//! The GUI stays unelevated. One elevated, headless copy handles one SD operation.
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    io::{self, Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    os::windows::ffi::OsStrExt,
    sync::Mutex,
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

static SD_OPERATION: Mutex<()> = Mutex::new(());
// Leave room for the result envelope around a worst-case 64 MiB byte array.
const MAX_FRAME: usize = 256 * 1024 * 1024 + 1024;
const MAX_READ: usize = 64 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
pub enum Operation {
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
}

fn invoke_blocking(operation: Operation) -> Result<serde_json::Value, String> {
    let _guard = SD_OPERATION.lock().map_err(|e| e.to_string())?;
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
                send(&mut stream, &operation)?;
                return receive::<Result<serde_json::Value, String>>(&mut stream)?;
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                if process.0 != 0 && unsafe { WaitForSingleObject(process.0, 0) } == WAIT_OBJECT_0 {
                    return Err("SD操作プロセスが接続前に終了しました".into());
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    Err("SD操作プロセスへの接続がタイムアウトしました".into())
}

pub async fn invoke<T: DeserializeOwned>(operation: Operation) -> Result<T, String> {
    let result = tauri::async_runtime::spawn_blocking(move || invoke_blocking(operation))
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
        let operation = receive::<Operation>(&mut stream)?;
        send(&mut stream, &execute(operation))
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
        }).is_err());
    }
}
