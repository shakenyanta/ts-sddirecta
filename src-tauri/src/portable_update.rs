//! Signed, opt-in updates for the portable exe. Never extract an archive into user data.
use base64::{engine::general_purpose::STANDARD, Engine};
use minisign_verify::{PublicKey, Signature};
use reqwest::blocking::Client;
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::Command,
    sync::Mutex,
    time::{Duration, Instant},
};

const RELEASE_ROOT: &str = "https://github.com/shakenyanta/ts-sddirecta-releases/releases/";
const MANIFEST_URL: &str =
    "https://github.com/shakenyanta/ts-sddirecta-releases/releases/latest/download/latest.json";
const PUBLIC_KEY: &str = include_str!("../update-public-key.txt");
const MAX_MANIFEST: u64 = 64 * 1024;
const MAX_EXE: u64 = 256 * 1024 * 1024;
const CREATE_NO_WINDOW: u32 = 0x08000000;
static PENDING: Mutex<Option<VerifiedManifest>> = Mutex::new(None);

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: String,
    target: String,
    url: String,
    size: u64,
    sha256: String,
    notes: String,
}

#[derive(Clone)]
struct VerifiedManifest {
    data: Vec<u8>,
    signature: String,
    manifest: Manifest,
}

#[derive(Serialize)]
pub struct UpdateCheck {
    pub status: &'static str,
    pub current_version: String,
    pub version: Option<String>,
    pub notes: String,
}

fn verify_signature(data: &[u8], signature: &str, key: &str) -> Result<(), String> {
    // Tauri CLI stores the complete minisign text, base64-encoded, in both files.
    let decode = |value: &str| -> Result<String, String> {
        String::from_utf8(STANDARD.decode(value.trim()).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())
    };
    let key = PublicKey::decode(&decode(key)?).map_err(|e| e.to_string())?;
    let signature = Signature::decode(&decode(signature)?).map_err(|e| e.to_string())?;
    key.verify(data, &signature, false)
        .map_err(|_| "更新情報の署名が正しくありません。更新を中止しました".into())
}

fn target() -> String {
    format!("windows-{}", std::env::consts::ARCH)
}

fn validate_manifest(data: &[u8]) -> Result<Manifest, String> {
    let manifest: Manifest =
        serde_json::from_slice(data).map_err(|e| format!("更新情報の形式が不正です: {e}"))?;
    Version::parse(&manifest.version).map_err(|_| "更新バージョンの形式が不正です")?;
    let url = reqwest::Url::parse(&manifest.url).map_err(|_| "更新URLが不正です")?;
    if manifest.target != target()
        || !manifest
            .url
            .starts_with(&format!("{RELEASE_ROOT}download/v{}/", manifest.version))
        || url.scheme() != "https"
        || url.host_str() != Some("github.com")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.path().ends_with(".exe")
        || manifest.size == 0
        || manifest.size > MAX_EXE
        || manifest.sha256.len() != 64
        || !manifest.sha256.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err("更新ファイルの配布先・対象環境・サイズが不正です".into());
    }
    Ok(manifest)
}

fn client() -> Result<Client, String> {
    Client::builder()
        .https_only(true)
        .user_agent(concat!("ts-sddirecta/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(300))
        .build()
        .map_err(|e| e.to_string())
}

fn fetch(client: &Client, url: &str, limit: u64) -> Result<Option<Vec<u8>>, String> {
    let response = client
        .get(url)
        .send()
        .map_err(|e| format!("更新サーバーに接続できません: {e}"))?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let response = response
        .error_for_status()
        .map_err(|e| format!("更新情報を取得できません: {e}"))?;
    if response.content_length().is_some_and(|n| n > limit) {
        return Err("更新ファイルのサイズが上限を超えています".into());
    }
    let mut data = Vec::new();
    response
        .take(limit + 1)
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    if data.len() as u64 > limit {
        return Err("更新ファイルのサイズが上限を超えています".into());
    }
    Ok(Some(data))
}

pub fn check(current: &str) -> Result<UpdateCheck, String> {
    *PENDING.lock().map_err(|e| e.to_string())? = None;
    let current_version = Version::parse(current).map_err(|e| e.to_string())?;
    let client = client()?;
    let Some(data) = fetch(&client, MANIFEST_URL, MAX_MANIFEST)? else {
        return Ok(UpdateCheck {
            status: "unpublished",
            current_version: current.into(),
            version: None,
            notes: "更新情報はまだ公開されていません。".into(),
        });
    };
    let signature = fetch(&client, &format!("{MANIFEST_URL}.sig"), 8192)?
        .ok_or("更新情報の署名が公開されていません")?;
    let signature = String::from_utf8(signature).map_err(|e| e.to_string())?;
    verify_signature(&data, &signature, PUBLIC_KEY)?;
    let manifest = validate_manifest(&data)?;
    let available = Version::parse(&manifest.version).map_err(|e| e.to_string())? > current_version;
    let result = UpdateCheck {
        status: if available { "available" } else { "latest" },
        current_version: current.into(),
        version: Some(manifest.version.clone()),
        notes: manifest.notes.clone(),
    };
    if available {
        *PENDING.lock().map_err(|e| e.to_string())? = Some(VerifiedManifest {
            data,
            signature,
            manifest,
        });
    }
    Ok(result)
}

fn verify_exe(data: &[u8], manifest: &Manifest) -> Result<(), String> {
    if data.len() as u64 != manifest.size
        || !data.starts_with(b"MZ")
        || format!("{:x}", Sha256::digest(data)) != manifest.sha256.to_ascii_lowercase()
    {
        return Err("更新exeのサイズまたはハッシュが一致しません。更新を中止しました".into());
    }
    Ok(())
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Request {
    parent_pid: u32,
    target: PathBuf,
}

pub struct PreparedUpdate {
    directory: PathBuf,
}

pub fn prepare(version: &str) -> Result<PreparedUpdate, String> {
    if cfg!(debug_assertions) {
        return Err("開発ビルドではexeの更新を実行できません。配布用exeで確認してください".into());
    }
    let pending = PENDING
        .lock()
        .map_err(|e| e.to_string())?
        .clone()
        .filter(|p| p.manifest.version == version)
        .ok_or("もう一度アップデート確認を行ってください")?;
    verify_signature(&pending.data, &pending.signature, PUBLIC_KEY)?;
    let data = fetch(&client()?, &pending.manifest.url, pending.manifest.size)?
        .ok_or("更新exeが公開されていません")?;
    verify_exe(&data, &pending.manifest)?;
    let target = std::env::current_exe()
        .and_then(fs::canonicalize)
        .map_err(|e| e.to_string())?;
    let parent = target.parent().ok_or("exeの保存先を取得できません")?;
    // Same-volume staging allows renames and never changes CSV/input/output.
    let directory = tempfile::Builder::new()
        .prefix(".ts-sddirecta-update-")
        .tempdir_in(parent)
        .map_err(|e| format!("exeの保存先に更新ファイルを作成できません: {e}"))?;
    let write = |name: &str, content: &[u8]| -> Result<(), String> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.path().join(name))
            .map_err(|e| e.to_string())?;
        file.write_all(content)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())
    };
    write("next.exe", &data)?;
    write("latest.json", &pending.data)?;
    write("latest.json.sig", pending.signature.as_bytes())?;
    fs::copy(&target, directory.path().join("helper.exe")).map_err(|e| e.to_string())?;
    write(
        "request.json",
        &serde_json::to_vec(&Request {
            parent_pid: std::process::id(),
            target,
        })
        .map_err(|e| e.to_string())?,
    )?;
    Ok(PreparedUpdate {
        directory: directory.keep(),
    })
}

impl PreparedUpdate {
    pub fn launch(&self) -> Result<(), String> {
        let mut child = Command::new(self.directory.join("helper.exe"))
            .arg("--portable-update-helper")
            .arg(self.directory.join("request.json"))
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| format!("更新用プロセスを起動できません: {e}"))?;
        let result = (|| {
            let start = Instant::now();
            while start.elapsed() < Duration::from_secs(10) {
                if self.directory.join("ready").is_file() {
                    // A helper left behind after a failed launch must never update
                    // the exe when the user later closes the application normally.
                    fs::write(self.directory.join("commit.tmp"), b"commit")
                        .map_err(|e| e.to_string())?;
                    fs::rename(
                        self.directory.join("commit.tmp"),
                        self.directory.join("commit"),
                    )
                    .map_err(|e| e.to_string())?;
                    return Ok(());
                }
                if child.try_wait().map_err(|e| e.to_string())?.is_some() {
                    return Err(
                        "更新用プロセスの準備に失敗しました。旧exeは変更していません".into(),
                    );
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err("更新用プロセスの応答がありません。旧exeは変更していません".into())
        })();
        if result.is_err() {
            let _ = child.kill();
            let _ = child.wait();
        }
        result
    }
}

fn replace_and_restart(
    target: &Path,
    next: &Path,
    backup: &Path,
    restart: impl Fn(&Path) -> Result<(), String>,
) -> Result<(), String> {
    // Each rename is same-volume. Keep the original until replacement succeeds.
    fs::rename(target, backup)
        .map_err(|e| format!("旧exeを退避できません（他の起動中アプリも終了してください）: {e}"))?;
    if let Err(error) = fs::rename(next, target) {
        fs::rename(backup, target).map_err(|e| {
            format!(
                "更新失敗: {error}。復元失敗: {e}。旧exe: {}",
                backup.display()
            )
        })?;
        let _ = restart(target);
        return Err(format!(
            "exeの差し替えに失敗したため旧版に戻しました: {error}"
        ));
    }
    if let Err(error) = restart(target) {
        fs::rename(target, next)
            .and_then(|_| fs::rename(backup, target))
            .map_err(|e| {
                format!(
                    "起動失敗: {error}。復元失敗: {e}。旧exe: {}",
                    backup.display()
                )
            })?;
        let _ = restart(target);
        return Err(format!(
            "新しいexeを起動できないため旧版に戻しました: {error}"
        ));
    }
    Ok(())
}

fn helper(request_path: &Path) -> Result<(), String> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, WAIT_OBJECT_0},
        System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE},
    };
    let helper = std::env::current_exe()
        .and_then(fs::canonicalize)
        .map_err(|e| e.to_string())?;
    let directory = helper.parent().ok_or("更新フォルダーが不正です")?;
    if fs::canonicalize(request_path).map_err(|e| e.to_string())? != directory.join("request.json")
    {
        return Err("更新要求の保存先が不正です".into());
    }
    let request: Request =
        serde_json::from_slice(&fs::read(request_path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let target = fs::canonicalize(&request.target).map_err(|e| e.to_string())?;
    if target.parent() != directory.parent()
        || target
            .extension()
            .and_then(|e| e.to_str())
            .is_none_or(|e| !e.eq_ignore_ascii_case("exe"))
    {
        return Err("更新対象のexeが不正です".into());
    }
    let data = fs::read(directory.join("latest.json")).map_err(|e| e.to_string())?;
    let signature =
        fs::read_to_string(directory.join("latest.json.sig")).map_err(|e| e.to_string())?;
    verify_signature(&data, &signature, PUBLIC_KEY)?;
    let manifest = validate_manifest(&data)?;
    if Version::parse(&manifest.version).map_err(|e| e.to_string())?
        <= Version::parse(env!("CARGO_PKG_VERSION")).map_err(|e| e.to_string())?
    {
        return Err("更新対象は現在のバージョンより新しくありません".into());
    }
    let next = directory.join("next.exe");
    let next_data = fs::read(&next).map_err(|e| e.to_string())?;
    verify_exe(&next_data, &manifest)?;
    let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, request.parent_pid) };
    if process == 0 {
        return Err("終了待ちのアプリを確認できません".into());
    }
    fs::write(directory.join("ready"), b"ready").map_err(|e| {
        unsafe {
            CloseHandle(process);
        }
        e.to_string()
    })?;
    let result = unsafe { WaitForSingleObject(process, 120_000) };
    unsafe {
        CloseHandle(process);
    }
    if result != WAIT_OBJECT_0 {
        return Err("アプリの終了を確認できないため、exeを変更しませんでした".into());
    }
    if fs::read(directory.join("commit")).ok().as_deref() != Some(b"commit") {
        return Err("更新の実行が確定されていないため、exeを変更しませんでした".into());
    }
    // Recheck just before replacing to detect a damaged/modified staged file.
    verify_exe(&fs::read(&next).map_err(|e| e.to_string())?, &manifest)?;
    replace_and_restart(&target, &next, &directory.join("previous.exe"), |exe| {
        Command::new(exe)
            .current_dir(exe.parent().ok_or("保存先が不正です")?)
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    })
}

pub fn run_if_helper() -> bool {
    let mut args = std::env::args_os().skip(1);
    if args
        .next()
        .is_none_or(|arg| arg != "--portable-update-helper")
    {
        return false;
    }
    let result = args
        .next()
        .ok_or_else(|| "更新要求がありません".to_string())
        .and_then(|path| helper(Path::new(&path)));
    if let Err(error) = result {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
        let wide = |s: &str| {
            std::ffi::OsStr::new(s)
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<_>>()
        };
        let text = wide(&format!("更新に失敗しました。\n{error}"));
        let title = wide("ts-sddirecta アップデート");
        unsafe {
            MessageBoxW(0, text.as_ptr(), title.as_ptr(), MB_OK | MB_ICONERROR);
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Manifest {
        Manifest {
            version: "0.2.0".into(),
            target: target(),
            url: format!("{RELEASE_ROOT}download/v0.2.0/ts-sddirecta.exe"),
            size: 7,
            sha256: format!("{:x}", Sha256::digest(b"MZhello")),
            notes: String::new(),
        }
    }
    #[test]
    fn accepts_only_expected_platform_and_release_exe() {
        let manifest = fixture();
        assert!(validate_manifest(&serde_json::to_vec(&manifest).unwrap()).is_ok());
        for changed in [
            Manifest {
                target: "windows-wrong".into(),
                ..manifest.clone()
            },
            Manifest {
                url: "https://example.com/evil.exe".into(),
                ..manifest.clone()
            },
            Manifest {
                size: MAX_EXE + 1,
                ..manifest.clone()
            },
            Manifest {
                version: "bad".into(),
                ..manifest.clone()
            },
        ] {
            assert!(validate_manifest(&serde_json::to_vec(&changed).unwrap()).is_err());
        }
    }
    #[test]
    fn rejects_corrupt_exe_and_invalid_signature() {
        assert!(verify_exe(b"MZhello", &fixture()).is_ok());
        assert!(verify_exe(b"MZhell!", &fixture()).is_err());
        assert!(verify_exe(b"MZ", &fixture()).is_err());
        assert!(verify_signature(b"data", "invalid", PUBLIC_KEY).is_err());
        let key = String::from_utf8(STANDARD.decode(PUBLIC_KEY.trim()).unwrap()).unwrap();
        assert!(PublicKey::decode(&key).is_ok());
    }
    #[test]
    fn verifies_tauri_cli_signatures_and_rejects_tampering_or_wrong_key() {
        // Generated using a throwaway key, not the production signing key.
        let data = include_bytes!("../../tests/fixtures/portable-update/manifest.json");
        let signature = include_str!("../../tests/fixtures/portable-update/manifest.json.sig");
        let key = include_str!("../../tests/fixtures/portable-update/public-key.txt");
        verify_signature(data, signature, key).unwrap();
        let mut changed = data.to_vec();
        changed[0] ^= 1;
        assert!(verify_signature(&changed, signature, key).is_err());
        assert!(verify_signature(data, signature, PUBLIC_KEY).is_err());
    }
    #[test]
    fn replacement_preserves_data_and_old_exe() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("app.exe");
        let next = dir.path().join("next.exe");
        let backup = dir.path().join("previous.exe");
        let data = dir.path().join("sdDirectaxs.csv");
        fs::write(&target, b"old").unwrap();
        fs::write(&next, b"new").unwrap();
        fs::write(&data, b"user data").unwrap();
        let settings = dir.path().join(".config/edition.json");
        fs::create_dir(dir.path().join(".config")).unwrap();
        fs::write(&settings, br#"{"edition":"pro"}"#).unwrap();
        replace_and_restart(&target, &next, &backup, |_| Ok(())).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new");
        assert_eq!(fs::read(&backup).unwrap(), b"old");
        assert_eq!(fs::read(&data).unwrap(), b"user data");
        assert_eq!(fs::read(&settings).unwrap(), br#"{"edition":"pro"}"#);
    }
    #[test]
    fn failed_replacement_or_restart_restores_old_exe() {
        for missing in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let target = dir.path().join("app.exe");
            let next = dir.path().join("next.exe");
            let backup = dir.path().join("previous.exe");
            fs::write(&target, b"old").unwrap();
            if !missing {
                fs::write(&next, b"new").unwrap();
            }
            assert!(
                replace_and_restart(&target, &next, &backup, |_| Err("launch failed".into()))
                    .is_err()
            );
            assert_eq!(fs::read(&target).unwrap(), b"old");
        }
    }
}
