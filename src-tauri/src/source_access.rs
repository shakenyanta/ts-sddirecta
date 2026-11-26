use std::{
    collections::HashSet,
    sync::{mpsc, Arc, LazyLock, Mutex},
    time::Duration,
};

const ACCESS_TIMEOUT: Duration = Duration::from_secs(2);
// Timed-out filesystem calls may still be waiting in Windows. Bound their number.
const MAX_PENDING: usize = 4;
static CHECKS: LazyLock<AccessChecks> = LazyLock::new(AccessChecks::default);

#[derive(Default)]
struct AccessChecks {
    pending: Arc<Mutex<HashSet<Vec<String>>>>,
}

struct PendingCheck {
    pending: Arc<Mutex<HashSet<Vec<String>>>>,
    paths: Vec<String>,
}

impl Drop for PendingCheck {
    fn drop(&mut self) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.remove(&self.paths);
        }
    }
}

impl AccessChecks {
    fn run(
        &self,
        paths: Vec<String>,
        timeout: Duration,
        check: impl FnOnce(Vec<String>) -> Result<String, String> + Send + 'static,
    ) -> Result<String, String> {
        let mut pending = self.pending.lock().map_err(|e| e.to_string())?;
        if pending.contains(&paths) {
            return Err("このパスは前回のアクセス確認が応答待ちです。しばらく待ってから、もう一度お試しください".into());
        }
        if pending.len() >= MAX_PENDING {
            return Err("応答待ちのアクセス確認が残っています。接続状態を確認し、しばらく待ってから、もう一度お試しください".into());
        }
        pending.insert(paths.clone());
        drop(pending);
        let guard = PendingCheck {
            pending: self.pending.clone(),
            paths: paths.clone(),
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::Builder::new()
            .name("source-access".into())
            .spawn(move || {
                let result = check(paths);
                drop(guard);
                // Once the receiver times out, late results are discarded here.
                let _ = sender.send(result);
            })
            .map_err(|e| format!("アクセス確認を開始できませんでした: {e}"))?;
        match receiver.recv_timeout(timeout) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => Err("2秒以内にフォルダーへのアクセスを確認できませんでした（タイムアウト）。ネットワーク接続や共有先の状態を確認して、もう一度お試しください".into()),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err("アクセス確認処理が終了しました。もう一度お試しください".into()),
        }
    }
}

pub fn resolve(paths: Vec<String>) -> Result<String, String> {
    let started = std::time::Instant::now();
    let result = CHECKS.run(paths.clone(), ACCESS_TIMEOUT, |paths| {
        let directory = crate::source_directory::dropped_directory(paths)?;
        // Metadata may be cached: also verify that the directory can be listed.
        let mut entries = std::fs::read_dir(&directory).map_err(access_error)?;
        if let Some(entry) = entries.next() {
            entry.map_err(access_error)?;
        }
        Ok(directory)
    });
    if let Err(error) = &result {
        crate::diagnostics::record(
            "WARN",
            &format!(
                "source access failed paths={paths:?} elapsed_ms={} error={error}",
                started.elapsed().as_millis()
            ),
        );
    }
    result
}

pub fn access_error(error: std::io::Error) -> String {
    let reason = match error.raw_os_error() {
        Some(53 | 64 | 67 | 121 | 1231 | 1232) => "ネットワークまたは共有先にアクセスできません",
        _ => match error.kind() {
            std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory => {
                "指定したファイルまたはフォルダーが見つかりません"
            }
            std::io::ErrorKind::PermissionDenied => "アクセス権限がありません",
            _ => "ファイルまたはフォルダーにアクセスできません",
        },
    };
    format!("{reason}: {error}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_success_and_immediate_errors_and_releases_slots() {
        let checks = AccessChecks::default();
        let paths = vec!["C:\\folder".into()];
        assert_eq!(
            checks
                .run(paths.clone(), ACCESS_TIMEOUT, |_| Ok("resolved".into()))
                .unwrap(),
            "resolved"
        );
        assert_eq!(
            checks
                .run(paths, ACCESS_TIMEOUT, |_| Err("denied".into()))
                .unwrap_err(),
            "denied"
        );
        assert!(checks.pending.lock().unwrap().is_empty());
    }

    #[test]
    fn timeout_discards_late_results_and_bounds_pending_checks() {
        let checks = AccessChecks::default();
        let mut releases = Vec::new();
        let mut completed = Vec::new();
        for index in 0..MAX_PENDING {
            let (release, wait) = mpsc::channel();
            let (done, finish) = mpsc::channel();
            let paths = vec![format!("C:\\folder{index}")];
            let started = std::time::Instant::now();
            let error = checks
                .run(paths.clone(), Duration::from_millis(20), move |_| {
                    wait.recv().unwrap();
                    let _ = done.send(());
                    Ok("late result".into())
                })
                .unwrap_err();
            assert!(error.contains("タイムアウト"));
            assert!(started.elapsed() < Duration::from_secs(1));
            assert!(checks
                .run(paths, ACCESS_TIMEOUT, |_| panic!("duplicate check started"))
                .unwrap_err()
                .contains("前回"));
            releases.push(release);
            completed.push(finish);
        }
        assert!(checks
            .run(vec!["C:\\another".into()], ACCESS_TIMEOUT, |_| panic!(
                "too many workers"
            ))
            .unwrap_err()
            .contains("応答待ち"));
        for release in releases {
            release.send(()).unwrap();
        }
        for finish in completed {
            finish.recv_timeout(ACCESS_TIMEOUT).unwrap();
        }
        let started = std::time::Instant::now();
        while !checks.pending.lock().unwrap().is_empty() {
            assert!(started.elapsed() < ACCESS_TIMEOUT);
            std::thread::yield_now();
        }
        assert_eq!(
            checks
                .run(vec!["C:\\folder0".into()], ACCESS_TIMEOUT, |_| Ok(
                    "retry".into()
                ))
                .unwrap(),
            "retry"
        );
    }

    #[test]
    fn real_paths_and_error_reasons() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().to_str().unwrap().to_owned();
        assert!(resolve(vec![path.clone()]).is_ok());
        let file = root.path().join("file.txt");
        std::fs::write(&file, b"test").unwrap();
        assert_eq!(
            resolve(vec![file.to_str().unwrap().into()]).unwrap(),
            resolve(vec![path]).unwrap()
        );
        assert!(
            resolve(vec![root.path().join("missing").to_str().unwrap().into()])
                .unwrap_err()
                .contains("見つかりません")
        );
        assert!(access_error(std::io::Error::from_raw_os_error(5)).contains("権限"));
        assert!(access_error(std::io::Error::from_raw_os_error(53)).contains("ネットワーク"));
        assert_eq!(ACCESS_TIMEOUT, Duration::from_secs(2));
    }
}
