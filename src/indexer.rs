//! The indexer: runs the embedded Guile script through `guix repl`, streams
//! progress from stderr, collects the JSON document, and emits typed events
//! to the UI thread. Fully cancellable; the child is killed on cancel/drop.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use crate::cache::{Cache, CacheStatus};
use crate::error::IndexerError;
use crate::index::Index;

/// Embed the Guile index generator; written to a temp file at runtime.
const INDEX_SCRIPT: &str = concat!(
    include_str!("../data/guix-index-core.scm"),
    "\n",
    include_str!("../data/guix-index.scm")
);

/// Hard timeout for a full cold index build (Guile module compilation can
/// take minutes on a fresh cache).
const TIMEOUT: Duration = Duration::from_secs(900);
/// Upper bound on the JSON document the script may emit (~20 MB observed).
const MAX_OUTPUT: usize = 256 * 1024 * 1024;

pub enum IndexEvent {
    /// `PROGRESS <done> <total>` line seen on the script's stderr.
    Progress { done: u64, total: u64 },
    /// A valid index is available (from cache or a finished build).
    Ready {
        index: std::sync::Arc<crate::index::Index>,
        fresh: bool,
        unkeyed: bool,
    },
    /// The load/build failed; `msg` is safe to show in the status bar.
    Failed { msg: String },
}

/// Cancellation token shared with the loader/indexer thread.
pub type Cancel = Arc<AtomicBool>;

/// Load the index from cache or build it via `guix repl`; runs on its own
/// thread and reports through `tx`. Shared by the TUI and the web server.
pub fn start_loader(tx: Sender<IndexEvent>, cancel: Cancel, force: bool) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name("guixvis-loader".into())
        .spawn(move || {
            let now_ms = || -> u64 {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0)
            };
            let (guix, origin) = match crate::guix_env::current_guix().and_then(|guix| {
                crate::guix_env::probe_origin(&guix, &cancel).map(|origin| (guix, origin))
            }) {
                Ok(pair) => pair,
                Err(msg) => {
                    let _ = tx.send(IndexEvent::Failed { msg });
                    return;
                }
            };

            if !force {
                if let Ok(cache) = Cache::new() {
                    match cache.load(Some(&origin), now_ms()) {
                        Ok(CacheStatus::Fresh(index)) => {
                            let _ = tx.send(IndexEvent::Ready {
                                index: Arc::new(index),
                                fresh: true,
                                unkeyed: false,
                            });
                            return;
                        }
                        Ok(CacheStatus::Unverified(index)) => {
                            let _ = tx.send(IndexEvent::Ready {
                                index: Arc::new(index),
                                fresh: false,
                                unkeyed: true,
                            });
                            return;
                        }
                        Ok(CacheStatus::Stale { reason }) => {
                            let _ = tx.send(IndexEvent::Failed {
                                msg: format!("cache stale ({reason}); rebuilding"),
                            });
                        }
                        Ok(CacheStatus::Absent) => {
                            let _ = tx.send(IndexEvent::Progress { done: 0, total: 0 });
                        }
                        Err(e) => {
                            let _ = tx.send(IndexEvent::Failed {
                                msg: format!("cache unreadable ({e}); rebuilding"),
                            });
                            cache.quarantine();
                        }
                    }
                }
            }

            // Rebuild path: run the indexer, then save the cache.
            match build(&guix, &origin, &cancel, &tx) {
                Ok((doc, _raw, _)) => {
                    match Index::from_doc(doc, now_ms()) {
                        Ok(index) => {
                            // The snapshot is what the next start reads; the
                            // JSON from the indexer stays in memory.
                            if let Ok(cache) = Cache::new() {
                                if let Err(e) = cache.save(&index) {
                                    let _ = tx.send(IndexEvent::Failed {
                                        msg: format!("cache save failed: {e}"),
                                    });
                                }
                            }
                            let _ = tx.send(IndexEvent::Ready {
                                index: Arc::new(index),
                                fresh: false,
                                unkeyed: !origin.is_verified(),
                            });
                        }
                        Err(e) => {
                            let _ = tx.send(IndexEvent::Failed {
                                msg: format!("indexer output invalid: {e}"),
                            });
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(IndexEvent::Failed {
                        msg: format!("index build failed: {e}"),
                    });
                }
            }
        })
        .expect("failed to spawn loader thread")
}

/// A private temp directory removed on drop.
pub(crate) struct TempScript(PathBuf);

impl Drop for TempScript {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Write the embedded script into a fresh directory that only this user can
/// read: the system temp directory is world-writable, so a predictable file
/// name there is an invitation to a symlink race.
pub(crate) fn write_private_script(content: &str) -> std::io::Result<(PathBuf, TempScript)> {
    use std::io::Write as _;
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!(
        "guixvis-{}-{}-{}",
        std::process::id(),
        nanos,
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));

    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        let mut builder = std::fs::DirBuilder::new();
        builder.mode(0o700);
        builder.create(&dir)?;
        let path = dir.join("index.scm");
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        Ok((path, TempScript(dir)))
    }

    #[cfg(not(unix))]
    {
        std::fs::create_dir(&dir)?;
        let path = dir.join("index.scm");
        std::fs::write(&path, content)?;
        Ok((path, TempScript(dir)))
    }
}

/// Run the embedded Guile script through `guix repl` and return the parsed
/// document plus raw bytes (for the cache). Blocking; call from a worker
/// thread. Progress lines are relayed through `progress`.
pub fn build(
    guix: &std::path::Path,
    origin: &crate::model::GuixOrigin,
    cancel: &Cancel,
    progress: &Sender<IndexEvent>,
) -> Result<(crate::model::IndexDoc, Vec<u8>, String), IndexerError> {
    let (script_path, _script_guard) = write_private_script(INDEX_SCRIPT)
        .map_err(|e| IndexerError::Exited(format!("cannot write temp script: {e}")))?;

    let commit = origin
        .channels
        .iter()
        .find(|c| c.name == "guix")
        .map(|c| c.commit.as_str())
        .unwrap_or("");
    let generated_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
        .to_string();

    let mut command = Command::new(guix);
    command
        .arg("repl")
        .arg("-q")
        .arg("--")
        .arg(&script_path)
        .arg(commit)
        .arg(&generated_ms);
    let mut last_progress = std::time::Instant::now() - Duration::from_secs(1);
    let raw = crate::process::capture(&mut command, cancel, TIMEOUT, MAX_OUTPUT, |line| {
        if last_progress.elapsed() < Duration::from_millis(50) {
            return;
        }
        if let Some(rest) = line.strip_prefix("PROGRESS ") {
            let mut parts = rest.split_whitespace();
            if let (Some(d), Some(t)) = (parts.next(), parts.next()) {
                if let (Ok(done), Ok(total)) = (d.parse::<u64>(), t.parse::<u64>()) {
                    let _ = progress.send(IndexEvent::Progress { done, total });
                    last_progress = std::time::Instant::now();
                }
            }
        }
    })
    .map_err(IndexerError::Exited)?;
    if raw.is_empty() {
        return Err(IndexerError::NoOutput);
    }

    let mut doc: crate::model::IndexDoc = serde_json::from_slice(&raw)
        .map_err(|e| IndexerError::Exited(format!("cannot parse indexer JSON: {e}")))?;
    doc.header.origin = origin.clone();
    Ok((doc, raw, commit.to_string()))
}

#[cfg(all(test, unix))]
mod cancellation_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::Ordering;
    use std::time::Instant;

    #[test]
    fn cancellation_does_not_join_inherited_pipes_forever() {
        let shell = std::env::split_paths(&std::env::var_os("PATH").unwrap())
            .map(|p| p.join("sh"))
            .find(|p| p.is_file())
            .unwrap();
        let (script, _guard) =
            write_private_script(&format!("#!{}\nsleep 2 &\nwait\n", shell.display())).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let cancel: Cancel = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&cancel);
        let stop = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            flag.store(true, Ordering::Relaxed);
        });
        let (tx, _rx) = std::sync::mpsc::channel();
        let started = Instant::now();
        assert!(build(&script, &Default::default(), &cancel, &tx).is_err());
        stop.join().unwrap();
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "cancel waited for descendant-held pipes"
        );
    }
}
