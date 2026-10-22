use std::{
    fs, io,
    path::{Path, PathBuf},
};

const FILE_NAME: &str = "sdDirectaxs.csv";
const DEFAULT_CSV: &str = include_str!("../../config/sdDirectaxs.csv");

pub enum CsvSource {
    File(PathBuf),
    Embedded,
}

impl CsvSource {
    pub fn folder_label(&self, executable_directory: &Path) -> &'static str {
        let executable_directory = fs::canonicalize(executable_directory)
            .unwrap_or_else(|_| executable_directory.to_path_buf());
        match self {
            Self::File(path) => {
                let parent = path.parent().unwrap_or(Path::new(""));
                let folder = fs::canonicalize(parent).unwrap_or_else(|_| parent.to_path_buf());
                let config = executable_directory.join("config");
                let config = fs::canonicalize(&config).unwrap_or(config);
                if folder == executable_directory {
                    "EXEフォルダ"
                } else if folder == config {
                    "configフォルダ"
                } else {
                    "対象フォルダ"
                }
            }
            Self::Embedded => "内蔵CSV",
        }
    }

    pub fn read(self) -> io::Result<String> {
        match self {
            Self::File(path) => fs::read_to_string(path),
            Self::Embedded => Ok(DEFAULT_CSV.to_owned()),
        }
    }
}

pub fn resolve_for_source(source: &Path, executable_directory: &Path) -> io::Result<CsvSource> {
    let is_input = source
        .file_name()
        .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("input"));
    let csv_directory = if is_input {
        source.parent().unwrap_or(source)
    } else {
        source
    };
    if let Some(path) = existing_csv(csv_directory)? {
        return Ok(CsvSource::File(path));
    }
    // An input drop uses its parent's CSV, then app config, then the embedded CSV.
    if !is_input {
        if let Some(path) = existing_csv(executable_directory)? {
            return Ok(CsvSource::File(path));
        }
    }
    if let Some(path) = existing_csv(&executable_directory.join("config"))? {
        return Ok(CsvSource::File(path));
    }
    Ok(CsvSource::Embedded)
}

fn existing_csv(directory: &Path) -> io::Result<Option<PathBuf>> {
    let selected = directory.join(FILE_NAME);
    match fs::metadata(&selected) {
        Ok(metadata) if metadata.is_file() => Ok(Some(selected)),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "CSVのパスがファイルではありません",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TestDirectory(PathBuf);
    impl TestDirectory {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "sddirect-csv-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn input_folder_and_file_drops_use_parent_then_config_then_embedded() {
        let exe = TestDirectory::new();
        let dropped = TestDirectory::new();
        let input = dropped.0.join("input");
        let config = exe.0.join("config");
        fs::create_dir(&input).unwrap();
        fs::create_dir(&config).unwrap();
        fs::write(input.join("FILE-AA.001"), b"data").unwrap();
        fs::write(input.join(FILE_NAME), b"ignored input CSV").unwrap();
        fs::write(dropped.0.join(FILE_NAME), b"parent CSV").unwrap();
        fs::write(exe.0.join(FILE_NAME), b"ignored exe CSV").unwrap();
        fs::write(config.join(FILE_NAME), b"edited config").unwrap();

        for path in [&input, &input.join("FILE-AA.001")] {
            let source =
                crate::source_directory::dropped_directory(vec![path.to_str().unwrap().to_owned()])
                    .unwrap();
            assert_eq!(
                resolve_for_source(Path::new(&source), &exe.0)
                    .unwrap()
                    .read()
                    .unwrap(),
                "parent CSV"
            );
        }
        fs::remove_file(dropped.0.join(FILE_NAME)).unwrap();
        assert_eq!(
            resolve_for_source(&input, &exe.0).unwrap().read().unwrap(),
            "edited config"
        );
        fs::remove_file(config.join(FILE_NAME)).unwrap();
        assert_eq!(
            resolve_for_source(&input, &exe.0).unwrap().read().unwrap(),
            DEFAULT_CSV
        );
        assert!(!config.join(FILE_NAME).exists());
    }

    #[test]
    fn normal_drop_uses_source_then_exe_then_config() {
        let exe = TestDirectory::new();
        let dropped = TestDirectory::new();
        let config = exe.0.join("config");
        fs::create_dir(&config).unwrap();
        fs::write(config.join(FILE_NAME), b"edited config").unwrap();
        fs::write(exe.0.join(FILE_NAME), b"exe CSV").unwrap();
        fs::write(dropped.0.join(FILE_NAME), b"dropped CSV").unwrap();
        let read = |expected_folder| {
            let source = resolve_for_source(&dropped.0, &exe.0).unwrap();
            assert_eq!(source.folder_label(&exe.0), expected_folder);
            assert_eq!(
                source.folder_label(&fs::canonicalize(&exe.0).unwrap()),
                expected_folder
            );
            source.read().unwrap()
        };
        assert_eq!(read("対象フォルダ"), "dropped CSV");
        fs::remove_file(dropped.0.join(FILE_NAME)).unwrap();
        assert_eq!(read("EXEフォルダ"), "exe CSV");
        fs::remove_file(exe.0.join(FILE_NAME)).unwrap();
        assert_eq!(read("configフォルダ"), "edited config");
        fs::remove_file(config.join(FILE_NAME)).unwrap();
        assert_eq!(read("内蔵CSV"), DEFAULT_CSV);
    }

    #[test]
    fn embedded_fallback_does_not_create_config_and_input_name_is_case_insensitive() {
        let exe = TestDirectory::new();
        let dropped = TestDirectory::new();
        let input = dropped.0.join("INPUT");
        fs::create_dir(&input).unwrap();
        fs::write(exe.0.join(FILE_NAME), b"ignored exe CSV").unwrap();
        assert_eq!(
            resolve_for_source(&input, &exe.0).unwrap().read().unwrap(),
            DEFAULT_CSV
        );
        assert!(!exe.0.join("config").exists());
        fs::write(dropped.0.join(FILE_NAME), b"parent CSV").unwrap();
        assert_eq!(
            resolve_for_source(&input, &exe.0).unwrap().read().unwrap(),
            "parent CSV"
        );
    }

    #[test]
    fn invalid_csv_path_reports_error() {
        let temp = TestDirectory::new();
        fs::create_dir(temp.0.join(FILE_NAME)).unwrap();
        assert!(resolve_for_source(&temp.0, &temp.0).is_err());
        assert!(!temp.0.join("config").exists());
    }
}
