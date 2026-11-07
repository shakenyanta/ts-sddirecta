//! Individual-info formats ported from ts-sddirect's fileDataUtils.ts/getUniqueInfo.
use std::{fs, path::Path};

#[derive(Default, serde::Serialize)]
pub struct IndividualInfo {
    pub value: String,
    pub bm_parameter2: String,
}

pub fn for_files(
    directory: &Path,
    files: &[String],
    edition: crate::edition::Edition,
) -> std::collections::HashMap<String, IndividualInfo> {
    if edition != crate::edition::Edition::Pro {
        return std::collections::HashMap::new();
    }
    files
        .iter()
        .map(|name| (name.clone(), read(&directory.join(name))))
        .collect()
}

fn numbered_ts_main(stem: &str) -> bool {
    stem.strip_prefix("TS-MAIN")
        .is_some_and(|suffix| !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit()))
}

fn supported(stem: &str) -> bool {
    numbered_ts_main(stem)
        || matches!(
            stem,
            "FILE-BM"
                | "IM"
                | "FILE-AA"
                | "FILE-BA"
                | "FILE-BC"
                | "TS-MAIN"
                | "TS-SUB21"
                | "TS-SUB22"
                | "TS-SUB23"
                | "TS-SUB24"
                | "TS-SUB25"
                | "TS-DISP"
                | "TS-PRN"
                | "TS-PRNSW"
        )
}

fn read(path: &Path) -> IndividualInfo {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_uppercase();
    if !supported(&stem) {
        return IndividualInfo::default();
    }
    if numbered_ts_main(&stem)
        && !path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("HEX"))
    {
        return IndividualInfo::default();
    }
    match fs::read(path) {
        Ok(bytes) => IndividualInfo {
            value: extract(&stem, &bytes),
            bm_parameter2: if stem == "FILE-BM" {
                bm_parameter2(&bytes)
            } else {
                String::new()
            },
        },
        Err(error) => {
            crate::diagnostics::record(
                "WARN",
                &format!(
                    "individual info unavailable path={} error={error}",
                    path.display()
                ),
            );
            IndividualInfo::default()
        }
    }
}

fn bm_parameter2(bytes: &[u8]) -> String {
    const FORMATS: [&str; 11] = [
        "Nフォーマット",
        "Fフォーマット",
        "マルチ",
        "日本交通(CC)",
        "扇橋(CD)",
        "京王自動車",
        "名鉄",
        "両備",
        "TS(自動日報2)",
        "高速",
        "CBフォーマット",
    ];
    bytes
        .get(0x007F)
        .and_then(|&value| {
            FORMATS
                .get(value as usize)
                .map(|name| format!("{value:02X}[{name}]"))
        })
        .unwrap_or_default()
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
        // Two bytes contain four decimal BCD digits; omit leading zeros.
        "FILE-AA" => bytes
            .get(0xCE..0xD0)
            .map(|b| {
                if b == [0xFF, 0xFF] {
                    "無制限".into()
                } else {
                    hex(b, "")
                        .parse::<u16>()
                        .map(|count| count.to_string())
                        .unwrap_or_default()
                }
            })
            .unwrap_or_default(),
        "FILE-BM" => bytes.get(..8).map(|b| hex(b, " ")).unwrap_or_default(),
        "FILE-BC" => bytes
            .chunks_exact(32)
            .filter(|block| block[..2] != [0xFF, 0xFF] && block[2..4] == *b"FU")
            .map(|block| String::from_utf8_lossy(&block[..2]).into_owned())
            .collect::<Vec<_>>()
            .join(" "),
        "IM" => bytes
            .get(0xD7F0..0xD7FE)
            .map(|b| b.iter().map(|&b| char::from(b)).collect())
            .unwrap_or_else(|| "登録番号なし(容量不足)".into()),
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
        name if numbered_ts_main(name) => bytes
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
            let search_name = match name {
                "TS-DISP" => "TS-SUB20",
                "TS-PRN" => "TS-PRN  ",
                "TS-PRNSW" => "TS-SUB26",
                _ => name,
            };
            find_version(&content, &hex(search_name.as_bytes(), ""))
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
    fn file_bc_lists_block_numbers_in_order_only_with_fu_signature() {
        let block = |header: [u8; 4]| {
            let mut bytes = vec![0xFF; 32];
            bytes[..4].copy_from_slice(&header);
            bytes
        };
        let mut bytes = block(*b"01FU");
        bytes.extend(block(*b"99XX"));
        bytes.extend(block([0xFF, 0xFF, b'F', b'U']));
        bytes.extend(block(*b"02FU"));
        bytes.extend(block(*b"01FU"));
        bytes.extend(block(*b"03Fu"));
        bytes.extend([0xFF; 32]);
        bytes.extend(b"04FU"); // Ignore an incomplete final block.
        let dir = tempfile::tempdir().unwrap();
        for filename in ["FILE-BC.001", "FILE-BC.002", "file-bc.003"] {
            let path = dir.path().join(filename);
            fs::write(&path, &bytes).unwrap();
            assert_eq!(read(&path).value, "01 02 01", "{filename}");
        }
        for bytes in [vec![], vec![0xFF; 64], block(*b"01XX"), b"01FU".to_vec()] {
            assert_eq!(extract("FILE-BC", &bytes), "");
        }
    }

    #[test]
    fn loading_limit_reads_bcd_at_ce_cf_and_rejects_invalid_or_short_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("FILE-AA.001");
        let mut bytes = vec![0xFF; 0xD0];
        for (value, expected) in [
            ([0x01, 0x00], "100"),
            ([0x00, 0x99], "99"),
            ([0x00, 0x00], "0"),
            ([0x00, 0x01], "1"),
            ([0x12, 0x34], "1234"),
            ([0x99, 0x99], "9999"),
            ([0xFF, 0xFF], "無制限"),
            ([0x0A, 0x01], ""),
            ([0x01, 0x9F], ""),
        ] {
            bytes[0xCE..0xD0].copy_from_slice(&value);
            fs::write(&path, &bytes).unwrap();
            assert_eq!(read(&path).value, expected, "{value:02X?}");
        }
        for size in [0, 0xCE, 0xCF] {
            fs::write(&path, &bytes[..size]).unwrap();
            assert_eq!(read(&path).value, "");
        }
    }

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
        assert_eq!(extract("IM", &[]), "登録番号なし(容量不足)");
        let mut main = vec![0; 0x1008];
        main[0x1002..0x1006].copy_from_slice(&[0x26, 1, 0x22, 0]);
        main[0x1006..].copy_from_slice(&[0xAB, 0xCD]);
        assert_eq!(extract("TS-MAIN2", &main), "260122_00 ABCD");
        assert_eq!(extract("TS-MAIN2", &main[..0x1005]), "");
        assert_eq!(extract("TS-MAIN3", &main), "260122_00 ABCD");
    }

    #[test]
    fn ts_main_numeric_suffixes_share_binary_format_and_reject_mixed_suffixes() {
        let dir = tempfile::tempdir().unwrap();
        let mut bytes = vec![0; 0x1008];
        bytes[0x1002..0x1006].copy_from_slice(&[0x26, 0x11, 0x04, 0x01]);
        bytes[0x1006..].copy_from_slice(&[0xAB, 0xCD]);
        for stem in [
            "TS-MAIN0",
            "TS-MAIN1",
            "TS-MAIN2",
            "TS-MAIN3",
            "TS-MAIN15",
            "TS-MAIN003",
            "TS-MAIN99999999999999999999",
        ] {
            assert!(supported(stem));
            let path = dir
                .path()
                .join(format!("{}.hEx", stem.to_ascii_lowercase()));
            fs::write(&path, &bytes).unwrap();
            assert_eq!(read(&path).value, "261104_01 ABCD", "{stem}");
            assert_eq!(extract(stem, &bytes[..0x1005]), "");
        }
        for stem in [
            "TS-MAINA",
            "TS-MAIN3A",
            "TS-MAINA3",
            "TS-MAIN3-1",
            "TS-MAIN3_1",
            "TS-MAIN 3",
            "TS-MAIN３",
            "OTHER-TS-MAIN3",
        ] {
            assert!(!supported(stem), "{stem}");
            let path = dir.path().join(format!("{stem}.HEX"));
            fs::write(&path, &bytes).unwrap();
            assert_eq!(read(&path).value, "", "{stem}");
        }
        let path = dir.path().join("TS-MAIN3.bin");
        fs::write(&path, &bytes).unwrap();
        assert_eq!(read(&path).value, "");
        let path = dir.path().join("TS-MAIN.HEX");
        fs::write(&path, b"00001000465526110401").unwrap();
        assert_eq!(read(&path).value, "261104_01");
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
        for name in ["TS-SUB21", "TS-SUB22", "TS-SUB23", "TS-SUB24", "TS-SUB25"] {
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
    fn ts_disp_reads_ts_sub20_version_instead_of_filename_marker() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("TS-DISP.HEX");
        // Include both markers to ensure TS-DISP does not take precedence.
        fs::write(
            &path,
            b"54532D4449535026010100\r\n54532D535542323026021800\r\n",
        )
        .unwrap();
        assert_eq!(read(&path).value, "260218_00");
        assert_eq!(extract("TS-DISP", b"54532D4449535026010100"), "");
        assert_eq!(extract("TS-DISP", b"54532D53554232302602\n1800"), "");
        assert_eq!(extract("TS-DISP", b"not a version"), "");
        assert_eq!(
            extract("TS-SUB21", b"54532D535542323126021800"),
            "260218_00"
        );
    }

    #[test]
    fn printer_versions_use_padded_prn_and_sub26_markers() {
        let dir = tempfile::tempdir().unwrap();
        for (filename, content, expected) in [
            ("TS-PRN.HEX", "54532D50524E202026110400", "261104_00"),
            ("TS-PRNSW.HEX", "54532D535542323610033000", "100330_00"),
            ("ts-prnsw.hex", "54532D535542323610033001", "100330_01"),
        ] {
            let path = dir.path().join(filename);
            fs::write(&path, content).unwrap();
            assert_eq!(read(&path).value, expected, "{filename}");
        }
        // The two padding spaces are part of the TS-PRN signature.
        assert_eq!(extract("TS-PRN", b"54532D50524E26110400"), "");
        assert_eq!(extract("TS-PRN", b"54532D50524E2026110400"), "");
        assert_eq!(extract("TS-PRNSW", b"54532D50524E535710033000"), "");
        for (name, content) in [
            ("TS-PRN", &b"54532D50524E20202611\n0400"[..]),
            ("TS-PRNSW", &b"54532D53554232361003\n3000"[..]),
        ] {
            assert_eq!(extract(name, content), "");
            assert_eq!(extract(name, b"not a version"), "");
        }
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
            for_files(dir.path(), &files, Pro)["FILE-BM.001"].value,
            "00 00 00 00 00 00 00 00"
        );
        assert_eq!(
            for_files(&dir.path().join("input"), &files, Pro)["FILE-BM.001"].value,
            "FF FF FF FF FF FF FF FF"
        );
        assert_eq!(read(&dir.path().join("missing/FILE-BM.001")).value, "");
        assert_eq!(read(&dir.path().join("unsupported.001")).value, "");
        fs::write(dir.path().join("TS-MAIN2.bin"), vec![0xFF; 0x1008]).unwrap();
        assert_eq!(read(&dir.path().join("TS-MAIN2.bin")).value, "");
    }

    #[test]
    fn bm_parameter2_reads_the_binary_byte_at_007f() {
        let names = [
            "Nフォーマット",
            "Fフォーマット",
            "マルチ",
            "日本交通(CC)",
            "扇橋(CD)",
            "京王自動車",
            "名鉄",
            "両備",
            "TS(自動日報2)",
            "高速",
            "CBフォーマット",
        ];
        let mut bytes = vec![0xFF; 0x81];
        for (value, name) in names.iter().enumerate() {
            bytes[0x7F] = value as u8;
            assert_eq!(bm_parameter2(&bytes), format!("{value:02X}[{name}]"));
        }
        for value in [0x0B, 0x30, 0xFF] {
            bytes[0x7F] = value;
            assert_eq!(bm_parameter2(&bytes), "");
        }
        assert_eq!(bm_parameter2(&bytes[..0x7F]), "");
        assert_eq!(bm_parameter2(&[]), "");
    }

    #[test]
    fn bm_info_is_read_once_per_file_and_refreshed_in_each_folder() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("input")).unwrap();
        let mut bytes = vec![0; 0x80];
        bytes[0x7F] = 0x0A;
        fs::write(dir.path().join("file-bm.002"), &bytes).unwrap();
        bytes[0x7F] = 1;
        fs::write(dir.path().join("input/file-bm.002"), &bytes).unwrap();
        let files = vec!["file-bm.002".into()];
        use crate::edition::Edition::{Lite, Pro};
        assert!(for_files(dir.path(), &files, Lite).is_empty());
        assert_eq!(
            for_files(dir.path(), &files, Pro)["file-bm.002"].bm_parameter2,
            "0A[CBフォーマット]"
        );
        assert_eq!(
            for_files(&dir.path().join("input"), &files, Pro)["file-bm.002"].bm_parameter2,
            "01[Fフォーマット]"
        );
        fs::write(dir.path().join("file-bm.002"), [0; 8]).unwrap();
        let refreshed = for_files(dir.path(), &files, Pro);
        assert_eq!(refreshed["file-bm.002"].bm_parameter2, "");
        assert_eq!(refreshed["file-bm.002"].value, "00 00 00 00 00 00 00 00");
        fs::remove_file(dir.path().join("file-bm.002")).unwrap();
        assert_eq!(
            for_files(dir.path(), &files, Pro)["file-bm.002"].bm_parameter2,
            ""
        );
    }
}
