//! Individual-info formats ported from ts-sddirect's fileDataUtils.ts/getUniqueInfo.
use std::{fs, path::Path};

pub fn for_files(
    directory: &Path,
    files: &[String],
    edition: crate::edition::Edition,
) -> std::collections::HashMap<String, String> {
    if edition != crate::edition::Edition::Pro {
        return std::collections::HashMap::new();
    }
    files
        .iter()
        .map(|name| (name.clone(), read(&directory.join(name))))
        .collect()
}

fn supported(stem: &str) -> bool {
    matches!(
        stem,
        "FILE-BM"
            | "IM"
            | "FILE-BA"
            | "TS-MAIN"
            | "TS-MAIN2"
            | "TS-SUB21"
            | "TS-SUB22"
            | "TS-SUB23"
            | "TS-SUB24"
            | "TS-SUB25"
            | "TS-DISP"
    )
}

fn read(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_uppercase();
    if !supported(&stem) {
        return String::new();
    }
    if stem == "TS-MAIN2"
        && !path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("HEX"))
    {
        return String::new();
    }
    match fs::read(path) {
        Ok(bytes) => extract(&stem, &bytes),
        Err(error) => {
            crate::diagnostics::record(
                "WARN",
                &format!(
                    "individual info unavailable path={} error={error}",
                    path.display()
                ),
            );
            String::new()
        }
    }
}

fn hex(bytes: &[u8], separator: &str) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(separator)
}

fn version(value: &str) -> String {
    format!("{}_{}", &value[..6], &value[6..8])
}

fn find_version(content: &str, pattern: &str) -> String {
    for line in content.lines() {
        if let Some(index) = line.find(pattern) {
            if let Some(value) = line.get(index + pattern.len()..index + pattern.len() + 8) {
                if value.is_ascii() {
                    return version(value);
                }
            }
        }
    }
    String::new()
}

fn extract(stem: &str, bytes: &[u8]) -> String {
    match stem {
        "FILE-BM" => bytes.get(..8).map(|b| hex(b, " ")).unwrap_or_default(),
        "IM" => bytes
            .get(0xD7F0..0xD7FE)
            .map(|b| b.iter().map(|&b| char::from(b)).collect())
            .unwrap_or_else(|| "登録番号なし".into()),
        "FILE-BA" => match bytes.get(0x5B..0x63) {
            Some(b">ABATUF<") => "営業ｶｰﾄﾞ",
            Some(b"]abatuf[") => "設定ｶｰﾄﾞ",
            Some(b"!EAKIKAK") => "ﾛｰﾃﾞｨﾝｸﾞｶｰﾄﾞ",
            Some(b"]RAELCD[") => "ｸﾘｱｶｰﾄﾞ",
            Some(b"]retsam[") => "ﾏｽﾀｰｶｰﾄﾞ",
            Some(b"!DNIKRAC") => "車種設定ｶｰﾄﾞ",
            Some(b"]MARPST[") => "ﾊﾟﾗﾑ復旧ｶｰﾄﾞ",
            Some(b"]TAMROF[") => "ﾌｫｰﾏｯﾄｶｰﾄﾞ",
            _ => "",
        }
        .into(),
        "TS-MAIN2" => bytes
            .get(0x1002..0x1006)
            .map(|b| {
                format!(
                    "{} {}",
                    version(&hex(b, "")),
                    hex(&bytes[bytes.len() - 2..], "")
                )
            })
            .unwrap_or_default(),
        "TS-MAIN" => {
            let content = String::from_utf8_lossy(bytes);
            let version = find_version(&content, "000010004655");
            let checksum = from_checksum(&content).unwrap_or_default();
            [version, checksum]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        }
        name if supported(name) => {
            let content = String::from_utf8_lossy(bytes);
            find_version(&content, &hex(name.as_bytes(), ""))
        }
        _ => String::new(),
    }
}

// Reconstruct the 0xFF000-byte FROM image from S3 records, leaving holes erased.
// The reference excludes three reserved 0xFFFF words from its little-endian sum.
fn from_checksum(content: &str) -> Option<String> {
    let mut image = vec![0xFF; 0xFF000];
    let (mut saw_s3, mut saw_s7) = (false, false);
    for line in content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if !line.is_ascii() || !line.starts_with('S') || line.len() < 4 {
            return None;
        }
        match line.as_bytes()[1] {
            b'0' => continue,
            b'3' => {
                let count = u8::from_str_radix(&line[2..4], 16).ok()? as usize;
                if count < 5 || line.len() < 4 + count * 2 {
                    return None;
                }
                let record: Vec<_> = (4..4 + count * 2)
                    .step_by(2)
                    .map(|i| u8::from_str_radix(&line[i..i + 2], 16).ok())
                    .collect::<Option<_>>()?;
                if record
                    .iter()
                    .fold(count as u8, |sum, b| sum.wrapping_add(*b))
                    != 0xFF
                {
                    return None;
                }
                let address = u32::from_be_bytes(record[..4].try_into().ok()?) as usize;
                for (i, byte) in record[4..count - 1].iter().enumerate() {
                    if let Some(target) = image.get_mut(address + i) {
                        *target = *byte;
                    }
                }
                saw_s3 = true;
            }
            b'7' => {
                saw_s7 = true;
                break;
            }
            _ => return None,
        }
    }
    if !saw_s3 || !saw_s7 {
        return None;
    }
    let sum = image.chunks_exact(2).fold(0u16, |sum, b| {
        sum.wrapping_add(u16::from_le_bytes([b[0], b[1]]))
    });
    Some(format!(
        "{:04X}",
        0u16.wrapping_sub(sum.wrapping_sub(3u16.wrapping_mul(0xFFFF)))
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_formats_and_short_files_match_reference() {
        assert_eq!(
            extract("FILE-BM", &[0, 1, 0xAB, 0xFF, 4, 5, 6, 7, 8]),
            "00 01 AB FF 04 05 06 07"
        );
        assert_eq!(extract("FILE-BM", &[0; 7]), "");
        let mut im = vec![0; 0xD7FE];
        im[0xD7F0..].copy_from_slice(b"12345678901234");
        assert_eq!(extract("IM", &im), "12345678901234");
        assert_eq!(extract("IM", &[]), "登録番号なし");
        let mut main = vec![0; 0x1008];
        main[0x1002..0x1006].copy_from_slice(&[0x26, 1, 0x22, 0]);
        main[0x1006..].copy_from_slice(&[0xAB, 0xCD]);
        assert_eq!(extract("TS-MAIN2", &main), "260122_00 ABCD");
        assert_eq!(extract("TS-MAIN2", &main[..0x1005]), "");
        assert_eq!(extract("TS-MAIN3", &main), ""); // Not dispatched in the reference.
    }

    #[test]
    fn card_signatures_match_reference() {
        for (signature, expected) in [
            (">ABATUF<", "営業ｶｰﾄﾞ"),
            ("]abatuf[", "設定ｶｰﾄﾞ"),
            ("!EAKIKAK", "ﾛｰﾃﾞｨﾝｸﾞｶｰﾄﾞ"),
            ("]RAELCD[", "ｸﾘｱｶｰﾄﾞ"),
            ("]retsam[", "ﾏｽﾀｰｶｰﾄﾞ"),
            ("!DNIKRAC", "車種設定ｶｰﾄﾞ"),
            ("]MARPST[", "ﾊﾟﾗﾑ復旧ｶｰﾄﾞ"),
            ("]TAMROF[", "ﾌｫｰﾏｯﾄｶｰﾄﾞ"),
            ("unknown!", ""),
        ] {
            let mut bytes = vec![0; 0x63];
            bytes[0x5B..].copy_from_slice(signature.as_bytes());
            assert_eq!(extract("FILE-BA", &bytes), expected);
        }
        assert_eq!(extract("FILE-BA", &[]), "");
    }

    #[test]
    fn text_versions_do_not_cross_lines_and_invalid_checksums_still_allow_version() {
        for name in [
            "TS-SUB21", "TS-SUB22", "TS-SUB23", "TS-SUB24", "TS-SUB25", "TS-DISP",
        ] {
            let content = format!("prefix{}26021800suffix\r\n", hex(name.as_bytes(), ""));
            assert_eq!(extract(name, content.as_bytes()), "260218_00");
            assert_eq!(extract(name, b"not a version"), "");
        }
        assert_eq!(extract("TS-MAIN", b"00001000465526012200"), "260122_00");
        assert_eq!(extract("TS-MAIN", b"0000100046552601\n2200"), "");
        assert_eq!(
            from_checksum("S3060000000001F8\nS70500000000FA"),
            Some("F8FB".into())
        );
        assert_eq!(from_checksum("S3060000000001F9\nS70500000000FA"), None);
        assert_eq!(from_checksum("S3060000000001F8"), None);
        // Golden values checked with the original getUniqueInfo implementation.
        assert_eq!(
            extract("TS-MAIN", b"S30B0000100046552601220000\nS70500000000FA"),
            "260122_00 A16C"
        );
        assert_eq!(
            extract(
                "TS-MAIN",
                b"S30B0000100046552601220000\nS307000FF0000000F9\nS70500000000FA"
            ),
            "260122_00 A16C"
        );
    }

    #[test]
    fn reads_selected_folder_and_enforces_edition_and_failure_fallback() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("input")).unwrap();
        fs::write(dir.path().join("FILE-BM.001"), [0; 8]).unwrap();
        fs::write(dir.path().join("input/FILE-BM.001"), [0xFF; 8]).unwrap();
        let files = vec!["FILE-BM.001".into()];
        use crate::edition::Edition::{Lite, Pro};
        assert!(for_files(dir.path(), &files, Lite).is_empty());
        assert_eq!(
            for_files(dir.path(), &files, Pro)["FILE-BM.001"],
            "00 00 00 00 00 00 00 00"
        );
        assert_eq!(
            for_files(&dir.path().join("input"), &files, Pro)["FILE-BM.001"],
            "FF FF FF FF FF FF FF FF"
        );
        assert_eq!(read(&dir.path().join("missing/FILE-BM.001")), "");
        assert_eq!(read(&dir.path().join("unsupported.001")), "");
        fs::write(dir.path().join("TS-MAIN2.bin"), vec![0xFF; 0x1008]).unwrap();
        assert_eq!(read(&dir.path().join("TS-MAIN2.bin")), "");
    }
}
