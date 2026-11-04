//! Update numbers and 32-byte FILE-BC records follow ts-sddirect.
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::Path,
};

#[derive(Clone, Default, Debug, Serialize)]
pub struct UpdateInfo {
    pub update_no: String,
    pub update_ver: String,
    pub condition: String,
    pub car_type: String,
}

#[derive(Default, Serialize)]
pub struct FileUpdates {
    pub files: HashMap<String, Vec<UpdateInfo>>,
    pub warnings: Vec<String>,
}

fn stem(name: &str) -> String {
    Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_uppercase()
}

fn update_numbers(name: &str) -> &'static [&'static str] {
    let name = stem(name);
    // Numeric suffixes (e.g. TS-MAIN2) are supported by the reference app.
    for (base, numbers) in [
        ("FILE-BM", &["01", "02"][..]),
        ("FILE-BO", &["03"][..]),
        ("FILE-BP", &["04"][..]),
        ("FILE-BQ", &["05"][..]),
        ("FILE-BR", &["06"][..]),
        ("FILE-BS", &["07"][..]),
        ("FILE-BU", &["08"][..]),
        ("IM", &["09"][..]),
        ("FILE-BW", &["10"][..]),
        ("TS-MAIN", &["11"][..]),
        ("TS-SUB21", &["12"][..]),
        ("TS-SUB22", &["13"][..]),
        ("TS-SUB23", &["14"][..]),
        ("TS-SUB24", &["15"][..]),
        ("TS-SUB25", &["16"][..]),
        ("TS-DISP", &["17"][..]),
        ("TS-PRN", &["18"][..]),
        ("TS-PRNSW", &["19"][..]),
        ("FILE-BI", &["31"][..]),
        ("FILE-BH", &["32"][..]),
        ("FILE-BV", &["43"][..]),
    ] {
        if name
            .strip_prefix(base)
            .is_some_and(|suffix| suffix.bytes().all(|b| b.is_ascii_digit()))
        {
            return numbers;
        }
    }
    &[]
}

fn parse(bytes: &[u8], warnings: &mut Vec<String>) -> HashMap<String, UpdateInfo> {
    let mut records = HashMap::new();
    let mut seen = HashSet::new();
    if bytes.is_empty() {
        warnings.push("FILE-BCが空です".into());
    }
    if bytes
        .chunks_exact(32)
        .remainder()
        .iter()
        .any(|b| *b != 0xFF)
    {
        warnings.push("FILE-BCの末尾に32バイト未満の不完全なレコードがあります".into());
    }
    for (index, chunk) in bytes.chunks_exact(32).enumerate() {
        if chunk.iter().all(|b| *b == 0xFF) {
            continue;
        }
        if !chunk[..2].iter().all(u8::is_ascii_digit) {
            warnings.push(format!(
                "FILE-BCの{}番目のレコードの更新Noが不正です",
                index + 1
            ));
            continue;
        }
        let no = String::from_utf8_lossy(&chunk[..2]).into_owned();
        if !seen.insert(no.clone()) {
            records.remove(&no);
            warnings.push(format!(
                "FILE-BCの更新No={no}が重複しています。該当番号の値は表示しません"
            ));
            continue;
        }
        let mut info = UpdateInfo {
            update_no: no.clone(),
            ..Default::default()
        };
        if chunk[4..12].iter().all(u8::is_ascii_digit) {
            let ver = String::from_utf8_lossy(&chunk[4..12]);
            info.update_ver = format!("{}_{}", &ver[..6], &ver[6..]);
        } else {
            warnings.push(format!("FILE-BCの更新No={no}の更新Verが不正です"));
        }
        for (label, bytes, target) in [
            ("条件", &chunk[12..14], &mut info.condition),
            ("車種", &chunk[24..26], &mut info.car_type),
        ] {
            if bytes.iter().all(|b| (0x20..=0x7E).contains(b)) {
                *target = String::from_utf8_lossy(bytes).into_owned();
            } else {
                warnings.push(format!("FILE-BCの更新No={no}の{label}が不正です"));
            }
        }
        records.insert(no, info);
    }
    records
}

pub fn for_files(
    directory: &Path,
    files: &[String],
    edition: crate::edition::Edition,
) -> FileUpdates {
    let mut result = FileUpdates::default();
    if edition != crate::edition::Edition::Pro || files.is_empty() {
        return result;
    }
    let mut bc = Vec::new();
    let entries =
        match fs::read_dir(directory).and_then(|entries| entries.collect::<Result<Vec<_>, _>>()) {
            Ok(entries) => entries,
            Err(error) => {
                result
                    .warnings
                    .push(format!("フォルダーの確認に失敗しました: {error}"));
                Vec::new()
            }
        };
    for base in ["FILE-AA", "FILE-BA", "FILE-BC"] {
        let mut names = Vec::new();
        for entry in &entries {
            let name = entry.file_name().to_string_lossy().into_owned();
            if stem(&name) != base || name.to_ascii_uppercase().ends_with(".BAK") {
                continue;
            }
            match entry.file_type() {
                Ok(kind) if kind.is_file() => names.push(name),
                Ok(_) => {}
                Err(error) => result
                    .warnings
                    .push(format!("{name}の確認に失敗しました: {error}")),
            }
        }
        names.sort();
        match names.len() {
            0 => result.warnings.push(format!("{base}がありません")),
            1 => {}
            _ => result
                .warnings
                .push(format!("{base}が複数あります: {}", names.join(", "))),
        }
        if base == "FILE-BC" {
            bc = names;
        }
    }
    let mut records = HashMap::new();
    let mut parsed = false;
    if bc.len() == 1 {
        let name = &bc[0];
        if !files.contains(name) {
            result
                .warnings
                .push(format!("{name}がgridに表示されていないため解析しません"));
        } else {
            match fs::read(directory.join(name)) {
                Ok(bytes) => {
                    records = parse(&bytes, &mut result.warnings);
                    parsed = true;
                }
                Err(error) => result
                    .warnings
                    .push(format!("{name}の読込に失敗しました: {error}")),
            }
        }
    }
    let mut missing = HashSet::new();
    for name in files {
        let rows = update_numbers(name)
            .iter()
            .map(|no| {
                records.get(*no).cloned().unwrap_or_else(|| {
                    if parsed && missing.insert(*no) {
                        result
                            .warnings
                            .push(format!("FILE-BCに更新No={no}の有効なレコードがありません"));
                    }
                    UpdateInfo {
                        update_no: (*no).into(),
                        ..Default::default()
                    }
                })
            })
            .collect();
        result.files.insert(name.clone(), rows);
    }
    for warning in &result.warnings {
        crate::diagnostics::record(
            "WARN",
            &format!(
                "file update info: directory={} {warning}",
                directory.display()
            ),
        );
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edition::Edition::{Lite, Pro};

    fn record(no: &str, ver: &str, condition: &str, car: &str) -> Vec<u8> {
        let mut bytes = vec![b' '; 32];
        bytes[..2].copy_from_slice(no.as_bytes());
        bytes[4..12].copy_from_slice(ver.as_bytes());
        bytes[12..14].copy_from_slice(condition.as_bytes());
        bytes[24..26].copy_from_slice(car.as_bytes());
        bytes
    }

    fn setup() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for name in ["FILE-AA.001", "FILE-BA.001"] {
            fs::write(dir.path().join(name), []).unwrap();
        }
        dir
    }

    fn load(dir: &Path, names: &[&str]) -> FileUpdates {
        for_files(
            dir,
            &names.iter().map(|s| (*s).into()).collect::<Vec<_>>(),
            Pro,
        )
    }

    #[test]
    fn every_bm_file_has_two_distinct_records_without_pseudo_numbers() {
        let dir = setup();
        let mut bytes = record("01", "23061900", "00", "01");
        bytes.extend(record("02", "23011200", "01", "XX"));
        bytes.extend(record("11", "26110401", "02", "02"));
        bytes.extend([0xFF; 32]);
        fs::write(dir.path().join("FILE-BC.001"), bytes).unwrap();
        let result = load(
            dir.path(),
            &[
                "FILE-BM.001",
                "FILE-BM.002",
                "FILE-BM.003",
                "TS-MAIN2.HEX",
                "TS-MAIN3.HEX",
                "FILE-AA.001",
                "FILE-BA.001",
                "FILE-BC.001",
                "OTHER.001",
            ],
        );
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        for name in ["FILE-BM.001", "FILE-BM.002", "FILE-BM.003"] {
            let rows = &result.files[name];
            assert_eq!(rows.len(), 2);
            assert_eq!(
                (
                    &*rows[0].update_no,
                    &*rows[0].update_ver,
                    &*rows[0].condition,
                    &*rows[0].car_type
                ),
                ("01", "230619_00", "00", "01")
            );
            assert_eq!(
                (
                    &*rows[1].update_no,
                    &*rows[1].update_ver,
                    &*rows[1].condition,
                    &*rows[1].car_type
                ),
                ("02", "230112_00", "01", "XX")
            );
        }
        for name in ["TS-MAIN2.HEX", "TS-MAIN3.HEX"] {
            assert_eq!(result.files[name][0].update_no, "11");
        }
        for name in ["FILE-AA.001", "FILE-BA.001", "FILE-BC.001", "OTHER.001"] {
            assert!(result.files[name].is_empty());
        }
    }

    #[test]
    fn printer_update_numbers_use_bc_records_18_and_19() {
        let dir = setup();
        let mut bytes = record("18", "11060100", "00", "  ");
        bytes[2..4].copy_from_slice(b"FU");
        let mut switch = record("19", "10033000", "00", "  ");
        switch[2..4].copy_from_slice(b"FU");
        bytes.extend(switch);
        bytes.extend([0xFF; 32]);
        fs::write(dir.path().join("FILE-BC.001"), bytes).unwrap();
        let result = load(dir.path(), &["TS-PRN.HEX", "TS-PRNSW.HEX", "FILE-BC.001"]);
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        for (filename, no, ver) in [
            ("TS-PRN.HEX", "18", "110601_00"),
            ("TS-PRNSW.HEX", "19", "100330_00"),
        ] {
            let rows = &result.files[filename];
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].update_no, no);
            assert_eq!(rows[0].update_ver, ver);
            assert_eq!(rows[0].condition, "00");
            assert_eq!(rows[0].car_type, "  ");
        }
        assert_eq!(update_numbers("ts-prnsw.hex"), &["19"]);
        assert!(update_numbers("TS-PRNSWA.HEX").is_empty());
    }

    #[test]
    fn no_write_targets_skip_file_checks_and_warnings() {
        let dir = tempfile::tempdir().unwrap();
        // No files: missing AA/BA/BC must not produce warnings.
        let result = load(dir.path(), &[]);
        assert!(result.files.is_empty());
        assert!(result.warnings.is_empty());
        // Unmatched files: duplicates and malformed BC must not be checked either.
        for name in [
            "FILE-AA.002",
            "FILE-AA.003",
            "FILE-BA.002",
            "FILE-BA.003",
            "FILE-BC.002",
            "FILE-BC.003",
        ] {
            fs::write(dir.path().join(name), b"invalid").unwrap();
        }
        let result = load(dir.path(), &[]);
        assert!(result.files.is_empty());
        assert!(result.warnings.is_empty());
        assert!(load(&dir.path().join("missing"), &[]).warnings.is_empty());
    }

    #[test]
    fn missing_and_multiple_files_warn_independently_and_do_not_select_bc() {
        let dir = tempfile::tempdir().unwrap();
        let result = load(dir.path(), &["FILE-BO.001"]);
        assert_eq!(
            result.warnings,
            [
                "FILE-AAがありません",
                "FILE-BAがありません",
                "FILE-BCがありません"
            ]
        );
        for base in ["FILE-AA", "FILE-BA", "FILE-BC"] {
            assert!(result
                .warnings
                .iter()
                .any(|w| w.contains(&format!("{base}がありません"))));
        }
        assert_eq!(result.files["FILE-BO.001"][0].update_no, "03");
        for base in ["FILE-AA", "FILE-BA", "FILE-BC"] {
            for ext in ["001", "002"] {
                fs::write(
                    dir.path().join(format!("{base}.{ext}")),
                    record("03", "26010100", "00", "01"),
                )
                .unwrap();
            }
        }
        let result = load(dir.path(), &["FILE-BO.001", "FILE-BC.001", "FILE-BC.002"]);
        for base in ["FILE-AA", "FILE-BA", "FILE-BC"] {
            assert!(result
                .warnings
                .iter()
                .any(|w| w.contains(&format!("{base}が複数"))
                    && w.contains(&format!("{base}.001"))
                    && w.contains(&format!("{base}.002"))));
        }
        assert!(result.files["FILE-BO.001"][0].update_ver.is_empty());
    }

    #[test]
    fn aa_ba_warnings_do_not_prevent_parsing_but_hidden_bc_does() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("FILE-BC.001"),
            record("03", "26010100", "00", "01"),
        )
        .unwrap();
        let result = load(dir.path(), &["FILE-BO.001", "FILE-BC.001"]);
        assert_eq!(result.warnings.len(), 2);
        assert_eq!(result.files["FILE-BO.001"][0].update_ver, "260101_00");
        let result = load(dir.path(), &["FILE-BO.001"]);
        assert!(result
            .warnings
            .iter()
            .any(|w| w.contains("gridに表示されていない")));
        assert!(result.files["FILE-BO.001"][0].update_ver.is_empty());
    }

    #[test]
    fn malformed_fields_duplicate_numbers_and_incomplete_tail_are_reported() {
        let mut bytes = record("01", "26010100", "00", "01");
        bytes.extend(record("01", "26010200", "01", "02"));
        bytes.extend(record("01", "26010300", "02", "03"));
        bytes.extend(record("02", "invalid!", "00", "01"));
        let mut bad_fields = record("03", "26010100", "00", "01");
        bad_fields[12] = 0xFF;
        bad_fields[24] = 0;
        bytes.extend(bad_fields);
        bytes.extend(record("??", "26010100", "00", "01"));
        bytes.extend([0xFF; 32]);
        bytes.push(0x00);
        let mut warnings = Vec::new();
        let records = parse(&bytes, &mut warnings);
        assert!(!records.contains_key("01"));
        assert!(records["02"].update_ver.is_empty());
        assert_eq!(records["02"].condition, "00");
        assert_eq!(records["03"].update_ver, "260101_00");
        assert!(records["03"].condition.is_empty());
        assert!(records["03"].car_type.is_empty());
        for text in [
            "重複",
            "更新Verが不正",
            "条件が不正",
            "車種が不正",
            "更新Noが不正",
            "不完全",
        ] {
            assert!(warnings.iter().any(|w| w.contains(text)), "{text}");
        }
        let mut warnings = Vec::new();
        assert!(parse(&[0xFF; 64], &mut warnings).is_empty());
        assert!(warnings.is_empty());
        parse(&[], &mut warnings);
        assert!(warnings[0].contains("空です"));
    }

    #[test]
    fn incomplete_ff_padding_is_ignored_but_non_ff_tails_warn() {
        for length in 1..32 {
            let mut bytes = record("01", "26010100", "00", "01");
            bytes.extend(vec![0xFF; length]);
            let mut warnings = Vec::new();
            let records = parse(&bytes, &mut warnings);
            assert_eq!(records["01"].update_ver, "260101_00");
            assert!(warnings.is_empty(), "padding length={length}");
            bytes[32 + length - 1] = 0x00;
            let mut warnings = Vec::new();
            let records = parse(&bytes, &mut warnings);
            assert_eq!(records["01"].update_ver, "260101_00");
            assert_eq!(warnings.len(), 1);
            assert!(warnings[0].contains("不完全"));
        }
    }

    #[test]
    fn directories_and_backups_are_not_candidates_and_lite_skips_io() {
        let dir = setup();
        fs::create_dir(dir.path().join("FILE-BC.002")).unwrap();
        fs::write(dir.path().join("FILE-BC.BAK"), []).unwrap();
        fs::write(
            dir.path().join("FILE-BC.001"),
            record("03", "26010100", "00", "01"),
        )
        .unwrap();
        let result = load(dir.path(), &["FILE-BC.001", "FILE-BO.001", "FILE-BP.001"]);
        assert_eq!(result.files["FILE-BO.001"][0].update_ver, "260101_00");
        assert!(result
            .warnings
            .iter()
            .any(|w| w.contains("更新No=04の有効なレコードがありません")));
        assert!(!result.warnings.iter().any(|w| w.contains("複数")));
        let result = for_files(
            &dir.path().join("nonexistent"),
            &["FILE-BM.001".into()],
            Lite,
        );
        assert!(result.files.is_empty());
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn input_uses_only_its_own_bc_and_read_failure_warns_without_stale_values() {
        use std::{fs::OpenOptions, os::windows::fs::OpenOptionsExt};
        let dir = setup();
        fs::write(
            dir.path().join("FILE-BC.001"),
            record("03", "26010100", "00", "01"),
        )
        .unwrap();
        let input = dir.path().join("input");
        fs::create_dir(&input).unwrap();
        fs::write(input.join("FILE-BO.001"), []).unwrap();
        let filenames = vec!["FILE-BO.001".into(), "FILE-BC.001".into()];
        let (files, from_input, _) =
            crate::matching_files::find_matches(&input, filenames.clone()).unwrap();
        assert!(!from_input);
        let result = for_files(&input, &files, Pro);
        assert!(result
            .warnings
            .iter()
            .any(|w| w.contains("FILE-BCがありません")));
        assert!(result.files["FILE-BO.001"][0].update_ver.is_empty());

        let bc_path = input.join("FILE-BC.001");
        fs::write(&bc_path, record("03", "26110400", "01", "XX")).unwrap();
        let result = for_files(&input, &filenames, Pro);
        assert_eq!(result.files["FILE-BO.001"][0].update_ver, "261104_00");
        // A real Windows sharing violation exercises the read-error path.
        let _lock = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&bc_path)
            .unwrap();
        let result = for_files(&input, &filenames, Pro);
        assert!(result
            .warnings
            .iter()
            .any(|w| w.contains("FILE-BC.001の読込に失敗")));
        assert_eq!(result.files["FILE-BO.001"][0].update_no, "03");
        assert!(result.files["FILE-BO.001"][0].update_ver.is_empty());
    }
}
