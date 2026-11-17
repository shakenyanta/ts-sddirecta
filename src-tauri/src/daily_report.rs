//! Read-only snapshots of daily report files from exe/output.
use std::{io::Read, path::Path};

const MAX_BYTES: u64 = 16 * 1024 * 1024;

pub fn require_pro(edition: crate::edition::Edition) -> Result<(), String> {
    if edition != crate::edition::Edition::Pro {
        return Err("日報解析はプロ版専用です".into());
    }
    Ok(())
}

pub fn read(directory: &Path, filename: &str) -> Result<Vec<u8>, String> {
    let path = crate::source_directory::file_to_open(
        directory.to_string_lossy().into_owned(), filename,
    )?;
    let file = std::fs::File::open(path).map_err(|e| format!("日報ファイルを開けません: {e}"))?;
    read_snapshot(file)
}

fn read_snapshot(reader: impl Read) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader.take(MAX_BYTES + 1).read_to_end(&mut bytes)
        .map_err(|e| format!("日報ファイルを読み込めません: {e}"))?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("日報ファイルが読込上限の16MiBを超えています".into());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restricts_to_pro_and_direct_files_without_modifying_bytes() {
        assert!(require_pro(crate::edition::Edition::Lite).is_err());
        assert!(require_pro(crate::edition::Edition::Pro).is_ok());
        let dir = tempfile::tempdir().unwrap();
        let original = [0x0b, 0x12, 0x07, 0xce, 0xfc, 0xe6, 0xff];
        let path = dir.path().join("FILE-AT.001");
        std::fs::write(&path, original).unwrap();
        assert_eq!(read(dir.path(), "FILE-AT.001").unwrap(), original);
        assert_eq!(std::fs::read(path).unwrap(), original);
        for name in ["../FILE-AT.001", "sub/FILE-AT.001", "missing", "", "."] {
            assert!(read(dir.path(), name).is_err(), "{name}");
        }
    }

    #[test]
    fn limits_reads_even_if_the_file_grows() {
        assert!(read_snapshot(std::io::repeat(0).take(MAX_BYTES + 1)).is_err());
        assert!(read_snapshot(&b""[..]).unwrap().is_empty());
    }
}
