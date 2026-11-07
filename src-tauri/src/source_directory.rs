use std::{fs, io, path::PathBuf};

pub fn directory(path: Option<String>) -> Result<PathBuf, String> {
    let path = match path {
        Some(path) => PathBuf::from(path),
        None => std::env::current_exe()
            .map_err(|e| e.to_string())?
            .parent()
            .ok_or("exeの保存先を取得できませんでした")?
            .to_path_buf(),
    };
    if !path.is_absolute() {
        return Err("絶対パスでフォルダーを指定してください".into());
    }
    let path = fs::canonicalize(path).map_err(|e| e.to_string())?;
    if !path.is_dir() {
        return Err("フォルダーではありません".into());
    }
    Ok(path)
}

pub fn dropped_directory(paths: Vec<String>) -> Result<String, String> {
    let mut selected = None;
    for path in paths {
        let path = PathBuf::from(path);
        if !path.is_absolute() {
            return Err("絶対パスで指定してください".into());
        }
        let path = fs::canonicalize(path).map_err(|e| e.to_string())?;
        let metadata = fs::metadata(&path).map_err(|e| e.to_string())?;
        let folder = if metadata.is_dir() {
            path
        } else if metadata.is_file() {
            path.parent()
                .ok_or("ファイルの親フォルダーがありません")?
                .to_path_buf()
        } else {
            return Err("ファイルまたはフォルダーを指定してください".into());
        };
        if selected
            .as_ref()
            .is_some_and(|previous| previous != &folder)
        {
            return Err(
                "異なるフォルダーが含まれています。対象フォルダーを1つ指定してください".into(),
            );
        }
        selected = Some(folder);
    }
    selected
        .ok_or("ファイルまたはフォルダーをドロップしてください")?
        .into_os_string()
        .into_string()
        .map_err(|_| "フォルダー名を変換できませんでした".into())
}

pub fn input_exists(directory: &std::path::Path) -> Result<bool, String> {
    match fs::metadata(directory.join("input")) {
        Ok(metadata) => Ok(metadata.is_dir()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.to_string()),
    }
}

/// Fall back only for missing/non-directory paths. Access failures must not imply deletion.
pub fn saved_directory(path: String) -> Result<Option<String>, String> {
    let mut candidate = PathBuf::from(path);
    if !candidate.is_absolute() {
        return Err("保存されたフォルダーは絶対パスではありません".into());
    }
    // An unreachable share cannot prove its descendants were deleted.
    #[cfg(windows)]
    if matches!(candidate.components().next(), Some(std::path::Component::Prefix(prefix))
        if matches!(prefix.kind(), std::path::Prefix::UNC(..) | std::path::Prefix::VerbatimUNC(..)))
    {
        let root = candidate.ancestors().last().ok_or("共有フォルダーのルートを取得できませんでした")?;
        fs::metadata(root).map_err(|error| error.to_string())?;
    }
    loop {
        match fs::metadata(&candidate) {
            Ok(metadata) if metadata.is_dir() => return Ok(Some(candidate.to_string_lossy().into_owned())),
            Ok(_) => {},
            Err(error) if matches!(error.kind(), io::ErrorKind::NotFound | io::ErrorKind::NotADirectory)
                && !matches!(error.raw_os_error(), Some(53 | 64 | 67 | 121 | 1231 | 1232)) => {},
            Err(error) => return Err(error.to_string()),
        }
        if !candidate.pop() { return Ok(None); }
    }
}

/// Resolve a single file in the displayed directory, never a URL or another directory.
pub fn file_to_open(folder: String, filename: &str) -> Result<PathBuf, String> {
    if filename.is_empty() || filename.contains(['/', '\\', ':']) || filename == "." || filename == ".." {
        return Err("ファイル名が不正です".into());
    }
    let folder = directory(Some(folder))?;
    let path = fs::canonicalize(folder.join(filename))
        .map_err(|e| format!("ファイルが存在しないか、アクセスできません: {e}"))?;
    if path.parent() != Some(folder.as_path()) || !path.is_file() {
        return Err("表示中のフォルダーの通常ファイルではありません".into());
    }
    Ok(path)
}

/// Only enumerate immediate directories. Do not follow descendants or read file contents.
pub fn child_folders(path: String) -> Result<Vec<String>, String> {
    let directory = directory(Some(path))?;
    let mut names = Vec::new();
    for entry in fs::read_dir(directory).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            names.push(
                entry
                    .file_name()
                    .into_string()
                    .map_err(|_| "フォルダー名を変換できませんでした")?,
            );
        }
    }
    names.sort_by_cached_key(|name| name.to_lowercase());
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_selection_uses_nearest_existing_directory_only() {
        let root = tempfile::tempdir().unwrap();
        let existing = root.path().join("existing");
        fs::create_dir(&existing).unwrap();
        let resolve = |path: PathBuf| saved_directory(path.to_str().unwrap().to_owned()).unwrap();
        assert_eq!(resolve(existing.clone()), Some(existing.to_string_lossy().into_owned()));
        assert_eq!(resolve(existing.join("deleted").join("nested")), Some(existing.to_string_lossy().into_owned()));
        fs::write(existing.join("file"), b"data").unwrap();
        assert_eq!(resolve(existing.join("file")), Some(existing.to_string_lossy().into_owned()));
        assert_eq!(resolve(existing.join("file").join("nested")), Some(existing.to_string_lossy().into_owned()));
        assert!(saved_directory("relative".into()).is_err());
    }

    #[test]
    fn open_file_resolves_only_files_in_the_displayed_folder() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("input")).unwrap();
        fs::write(root.path().join("日本語 file.001"), b"root").unwrap();
        fs::write(root.path().join("input").join("日本語 file.001"), b"input").unwrap();
        for folder in [root.path().to_path_buf(), root.path().join("input")] {
            let resolve = |name| file_to_open(folder.to_str().unwrap().to_owned(), name);
            assert_eq!(fs::read(resolve("日本語 file.001").unwrap()).unwrap(),
                if folder == root.path() { b"root".to_vec() } else { b"input".to_vec() });
            for invalid in ["", ".", "..", "../file.001", "input\\file.001", "https:example", "missing", "input"] {
                assert!(resolve(invalid).is_err(), "{invalid}");
            }
        }
        assert!(file_to_open("relative".into(), "file.001").is_err());
    }

    #[test]
    fn lists_only_immediate_folders_and_reports_invalid_paths() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("B")).unwrap();
        fs::create_dir_all(root.path().join("a").join("nested")).unwrap();
        fs::write(root.path().join("file.txt"), b"data").unwrap();
        let list = |path: PathBuf| child_folders(path.to_str().unwrap().to_owned());
        assert_eq!(list(root.path().to_path_buf()).unwrap(), vec!["a", "B"]);
        assert!(list(root.path().join("B")).unwrap().is_empty());
        assert!(list(root.path().join("missing")).is_err());
        assert!(list(root.path().join("file.txt")).is_err());
        assert!(child_folders("relative".into()).is_err());
    }
}
