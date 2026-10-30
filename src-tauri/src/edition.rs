use serde::{Deserialize, Serialize};
use std::{fs, io, path::Path, sync::OnceLock};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Edition {
    Lite,
    Pro,
}

#[derive(Clone, Debug, Serialize)]
pub struct EditionInfo {
    pub edition: Edition,
    pub development: bool,
    pub warning: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    edition: Edition,
}

fn resolve(
    development: bool,
    dev_edition: Option<&str>,
    read_settings: impl FnOnce() -> io::Result<String>,
) -> EditionInfo {
    let mut info = EditionInfo {
        edition: Edition::Lite,
        development,
        warning: None,
    };
    if development {
        info.edition = if dev_edition == Some("lite") {
            Edition::Lite
        } else {
            Edition::Pro
        };
        return info;
    }
    match read_settings() {
        Ok(contents) => {
            match serde_json::from_str::<Settings>(contents.trim_start_matches('\u{feff}')) {
                Ok(settings) => info.edition = settings.edition,
                Err(error) => {
                    info.warning = Some(format!(
                        "版の設定が不正です。ライト版で起動しました: {error}"
                    ))
                }
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            info.warning = Some(format!(
                "版の設定を読み取れません。ライト版で起動しました: {error}"
            ))
        }
    }
    info
}

fn read_settings(executable: &Path) -> io::Result<String> {
    let directory = executable
        .parent()
        .ok_or_else(|| io::Error::other("exeの保存先がありません"))?;
    fs::read_to_string(directory.join(".config").join("edition.json"))
}

pub fn current() -> &'static EditionInfo {
    static INFO: OnceLock<EditionInfo> = OnceLock::new();
    INFO.get_or_init(|| {
        let development = cfg!(debug_assertions);
        let dev_edition = if development {
            std::env::var("SDDIRECTA_DEV_EDITION").ok()
        } else {
            None
        };
        resolve(development, dev_edition.as_deref(), || {
            read_settings(&std::env::current_exe()?)
        })
    })
}

impl EditionInfo {
    pub fn label(&self) -> &'static str {
        match self.edition {
            Edition::Lite => "lite",
            Edition::Pro => "pro",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn development_ignores_files_and_defaults_to_pro() {
        for (value, edition) in [
            (None, Edition::Pro),
            (Some("lite"), Edition::Lite),
            (Some("pro"), Edition::Pro),
        ] {
            let info = resolve(true, value, || panic!("開発時は設定を読みません"));
            assert_eq!(info.edition, edition);
            assert!(info.development);
        }
    }

    #[test]
    fn release_ignores_development_override() {
        for dev in [None, Some("lite"), Some("pro")] {
            assert_eq!(
                resolve(false, dev, || Ok(r#"{"edition":"pro"}"#.into())).edition,
                Edition::Pro
            );
            assert_eq!(
                resolve(false, dev, || Err(io::ErrorKind::NotFound.into())).edition,
                Edition::Lite
            );
            assert_eq!(
                resolve(false, dev, || Ok(r#"{"edition":"lite"}"#.into())).edition,
                Edition::Lite
            );
        }
    }

    #[test]
    fn invalid_or_unreadable_settings_warn_and_fall_back() {
        for contents in [
            "",
            "{}",
            r#"{"edition":"other"}"#,
            r#"{"edition":"pro","typo":true}"#,
        ] {
            let info = resolve(false, None, || Ok(contents.into()));
            assert_eq!(info.edition, Edition::Lite);
            assert!(info.warning.is_some());
        }
        let info = resolve(false, None, || Err(io::ErrorKind::PermissionDenied.into()));
        assert_eq!(info.edition, Edition::Lite);
        assert!(info.warning.is_some());
        assert!(resolve(false, None, || Err(io::ErrorKind::NotFound.into()))
            .warning
            .is_none());
    }

    #[test]
    fn reads_only_beside_executable_and_accepts_bom() {
        let folder = tempfile::tempdir().unwrap();
        let executable = folder.path().join("app.exe");
        fs::create_dir(folder.path().join(".config")).unwrap();
        fs::write(
            folder.path().join(".config/edition.json"),
            "\u{feff}{\"edition\":\"pro\"}",
        )
        .unwrap();
        let info = resolve(false, None, || read_settings(&executable));
        assert_eq!(info.edition, Edition::Pro);
        assert_eq!(info.label(), "pro");
        fs::write(
            folder.path().join(".config/edition.json"),
            r#"{"edition":"lite"}"#,
        )
        .unwrap();
        assert_eq!(info.edition, Edition::Pro); // 起動時の判定結果は変更しない
    }
}
