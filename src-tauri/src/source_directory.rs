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
