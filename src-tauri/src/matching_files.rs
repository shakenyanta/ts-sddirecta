use std::{collections::HashSet, fs, io, path::Path};

fn matching_names(directory: &Path, keys: &HashSet<String>) -> io::Result<Vec<String>> {
    let mut matches = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        if let Some(name) = entry.file_name().to_str() {
            if keys.contains(name) {
                matches.push(name.to_owned());
            }
        }
    }
    matches.sort();
    Ok(matches)
}

/// Search only immediate files; input is a fallback when the exe folder has no matches.
pub fn find_matches(directory: &Path, filenames: Vec<String>) -> io::Result<(Vec<String>, bool)> {
    let keys: HashSet<String> = filenames.into_iter().collect();
    let matches = matching_names(directory, &keys)?;
    if !matches.is_empty() {
        return Ok((matches, false));
    }
    let input = directory.join("input");
    match fs::metadata(&input) {
        Ok(metadata) if metadata.is_dir() => {
            let matches = matching_names(&input, &keys)?;
            let from_input = !matches.is_empty();
            Ok((matches, from_input))
        }
        Ok(_) => Ok((Vec::new(), false)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok((Vec::new(), false)),
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
            (vec!["FILE-AA.001".into()], false)
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
            (vec!["FILE-AB.001".into()], true)
        );
    }

    #[test]
    fn missing_input_and_unmatched_files_return_no_matches() {
        let dir = TestDirectory::new();
        assert_eq!(find_matches(&dir.0, keys()).unwrap(), (Vec::new(), false));
        fs::create_dir_all(dir.0.join("input/nested")).unwrap();
        fs::write(dir.0.join("input/unrelated.txt"), b"").unwrap();
        fs::write(dir.0.join("input/nested/FILE-AA.001"), b"").unwrap();
        assert_eq!(find_matches(&dir.0, keys()).unwrap(), (Vec::new(), false));
    }

    #[test]
    fn returns_all_matches_in_filename_order() {
        let dir = TestDirectory::new();
        fs::write(dir.0.join("FILE-AB.001"), b"").unwrap();
        fs::write(dir.0.join("FILE-AA.001"), b"").unwrap();
        assert_eq!(find_matches(&dir.0, keys()).unwrap(), (keys(), false));
    }
}
