use std::{collections::HashSet, fs, io, path::Path};

fn matching_names(
    directory: &Path,
    keys: &HashSet<String>,
    stems: &HashSet<&str>,
) -> io::Result<(Vec<String>, Vec<String>)> {
    let mut matches = Vec::new();
    let mut warnings = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        if let Some(name) = entry.file_name().to_str() {
            if keys.contains(name) {
                matches.push(name.to_owned());
            } else if Path::new(name)
                .file_stem()
                .and_then(|stem| stem.to_str())
                .is_some_and(|stem| stems.contains(stem))
            {
                warnings.push(name.to_owned());
            }
        }
    }
    matches.sort();
    warnings.sort();
    Ok((matches, warnings))
}

/// Search only immediate files; input is a fallback when the selected folder has no matches.
pub fn find_matches(
    directory: &Path,
    filenames: Vec<String>,
) -> io::Result<(Vec<String>, bool, Vec<String>)> {
    let keys: HashSet<String> = filenames.into_iter().collect();
    let stems = keys
        .iter()
        .filter_map(|name| Path::new(name).file_stem()?.to_str())
        .collect();
    let (matches, mut warnings) = matching_names(directory, &keys, &stems)?;
    if !matches.is_empty() {
        return Ok((matches, false, warnings));
    }
    let input = directory.join("input");
    match fs::metadata(&input) {
        Ok(metadata) if metadata.is_dir() => {
            let (matches, input_warnings) = matching_names(&input, &keys, &stems)?;
            warnings.extend(
                input_warnings
                    .into_iter()
                    .map(|name| format!("input\\{name}")),
            );
            let from_input = !matches.is_empty();
            Ok((matches, from_input, warnings))
        }
        Ok(_) => Ok((Vec::new(), false, warnings)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok((Vec::new(), false, warnings)),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
    };

    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "ts-sddirecta-matching-{}-{}",
                std::process::id(),
                NEXT_ID.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let resolved = self.0.canonicalize().unwrap();
            let temp = std::env::temp_dir().canonicalize().unwrap();
            assert_eq!(resolved.parent(), Some(temp.as_path()));
            assert!(resolved
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("ts-sddirecta-matching-"));
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn keys() -> Vec<String> {
        vec!["FILE-AA.001".into(), "FILE-AB.001".into()]
    }

    #[test]
    fn exe_folder_matches_take_priority_over_input() {
        let dir = TestDirectory::new();
        fs::create_dir(dir.0.join("input")).unwrap();
        fs::write(dir.0.join("FILE-AA.001"), b"").unwrap();
        fs::write(dir.0.join("input/FILE-AB.001"), b"").unwrap();
        assert_eq!(
            find_matches(&dir.0, keys()).unwrap(),
            (vec!["FILE-AA.001".into()], false, vec![])
        );
    }

    #[test]
    fn input_is_used_only_when_no_exe_folder_files_match() {
        let dir = TestDirectory::new();
        fs::create_dir(dir.0.join("input")).unwrap();
        fs::create_dir(dir.0.join("FILE-AA.001")).unwrap();
        fs::write(dir.0.join("unrelated.txt"), b"").unwrap();
        fs::write(dir.0.join("input/FILE-AB.001"), b"").unwrap();
        assert_eq!(
            find_matches(&dir.0, keys()).unwrap(),
            (vec!["FILE-AB.001".into()], true, vec![])
        );
    }

    #[test]
    fn missing_input_and_unmatched_files_return_no_matches() {
        let dir = TestDirectory::new();
        assert_eq!(
            find_matches(&dir.0, keys()).unwrap(),
            (Vec::new(), false, vec![])
        );
        fs::create_dir_all(dir.0.join("input/nested")).unwrap();
        fs::write(dir.0.join("input/unrelated.txt"), b"").unwrap();
        fs::write(dir.0.join("input/nested/FILE-AA.001"), b"").unwrap();
        assert_eq!(
            find_matches(&dir.0, keys()).unwrap(),
            (Vec::new(), false, vec![])
        );
    }

    #[test]
    fn returns_all_matches_in_filename_order() {
        let dir = TestDirectory::new();
        fs::write(dir.0.join("FILE-AB.001"), b"").unwrap();
        fs::write(dir.0.join("FILE-AA.001"), b"").unwrap();
        assert_eq!(
            find_matches(&dir.0, keys()).unwrap(),
            (keys(), false, vec![])
        );
    }

    #[test]
    fn extension_warnings_preserve_exact_matches_and_ignore_directories() {
        let dir = TestDirectory::new();
        for name in ["FILE-AA.001", "FILE-AA.bin", "FILE-AA", "unrelated.bin"] {
            fs::write(dir.0.join(name), b"").unwrap();
        }
        fs::create_dir(dir.0.join("FILE-AB.bin")).unwrap();
        assert_eq!(
            find_matches(&dir.0, keys()).unwrap(),
            (
                vec!["FILE-AA.001".into()],
                false,
                vec!["FILE-AA".into(), "FILE-AA.bin".into()]
            )
        );
    }

    #[test]
    fn warnings_do_not_prevent_input_fallback_and_keep_full_filenames() {
        let dir = TestDirectory::new();
        fs::create_dir(dir.0.join("input")).unwrap();
        fs::write(dir.0.join("FILE-AA.txt"), b"").unwrap();
        fs::write(dir.0.join("input/FILE-AB.001"), b"").unwrap();
        fs::write(dir.0.join("input/FILE-AA.002"), b"").unwrap();
        assert_eq!(
            find_matches(&dir.0, keys()).unwrap(),
            (
                vec!["FILE-AB.001".into()],
                true,
                vec!["FILE-AA.txt".into(), "input\\FILE-AA.002".into()]
            )
        );
    }

    #[test]
    fn warning_only_results_do_not_become_write_matches() {
        let dir = TestDirectory::new();
        fs::write(dir.0.join("日本語.data.bin"), b"").unwrap();
        fs::write(dir.0.join("日本語.other.bin"), b"").unwrap();
        assert_eq!(
            find_matches(&dir.0, vec!["日本語.data.001".into()]).unwrap(),
            (vec![], false, vec!["日本語.data.bin".into()])
        );
    }

    #[test]
    fn dropped_folder_and_files_resolve_to_the_same_source() {
        let dir = TestDirectory::new();
        let folder = dir.0.join("日本語 folder");
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("one.txt"), b"").unwrap();
        fs::write(folder.join("two.txt"), b"").unwrap();
        let resolve = crate::source_directory::dropped_directory;
        let folder_source = resolve(vec![folder.to_str().unwrap().into()]).unwrap();
        assert_eq!(
            resolve(vec![
                folder.join("one.txt").to_str().unwrap().into(),
                folder.join("two.txt").to_str().unwrap().into(),
            ])
            .unwrap(),
            folder_source
        );
        assert!(resolve(vec![folder_source, dir.0.to_str().unwrap().into()]).is_err());
        assert!(resolve(vec![]).is_err());
        assert!(resolve(vec![folder.join("missing.txt").to_str().unwrap().into()]).is_err());
    }

    #[test]
    fn selected_source_controls_csv_and_input_matching() {
        let dir = TestDirectory::new();
        let source = Some(dir.0.to_str().unwrap().to_owned());
        let csv = "A,FILE-AA.001,00000000,000001FF";
        fs::write(dir.0.join("sdDirectaxs.csv"), csv).unwrap();
        assert!(crate::check_startup_csv(source.clone()).unwrap());
        let loaded = crate::read_startup_csv(source.clone()).unwrap();
        assert_eq!(loaded.contents, csv);
        assert_eq!(loaded.source_folder, "対象フォルダ");
        fs::create_dir(dir.0.join("input")).unwrap();
        fs::write(dir.0.join("input/FILE-AA.001"), b"test").unwrap();
        let matched = crate::find_dictionary_files(keys(), source).unwrap();
        assert_eq!(matched.files, vec!["FILE-AA.001"]);
        assert!(matched.from_input);
        assert!(matched.input_exists);
        let other = TestDirectory::new();
        let other_source = Some(other.0.to_str().unwrap().to_owned());
        let matched = crate::find_dictionary_files(keys(), other_source).unwrap();
        assert!(matched.files.is_empty());
        assert!(!matched.from_input);
        assert!(!matched.input_exists);
    }
}
