//! The indexer: runs the embedded Guile script through `guix repl`, streams
//! progress from stderr, collects the JSON document, and emits typed events
//! to the UI thread. Fully cancellable; the child is killed on cancel/drop.

use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::cache::{describe_commit, find_guix, Cache, CacheStatus};
use crate::error::IndexerError;
use crate::index::Index;

/// Embed the Guile index generator; written to a temp file at runtime.
const INDEX_SCRIPT: &str = include_str!("../data/guix-index.scm");

/// Hard timeout for a full cold index build (Guile module compilation can
/// take minutes on a fresh cache).
const TIMEOUT: Duration = Duration::from_secs(900);
/// Upper bound on the JSON document the script may emit (~20 MB observed).
const MAX_OUTPUT: u64 = 256 * 1024 * 1024;

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
            let guix = find_guix();
            let commit = guix.as_deref().and_then(describe_commit);

            if !force {
                if let Ok(cache) = Cache::new() {
                    match cache.load(commit.as_deref()) {
                        Ok(CacheStatus::Fresh(doc)) => match Index::from_doc(doc, now_ms()) {
                            Ok(index) => {
                                let _ = tx.send(IndexEvent::Ready {
                                    index: Arc::new(index),
                                    fresh: true,
                                    unkeyed: commit.is_none(),
                                });
                                return;
                            }
                            Err(e) => {
                                let _ = tx.send(IndexEvent::Failed {
                                    msg: format!("cached index rejected ({e}); rebuilding"),
                                });
                                cache.quarantine();
                            }
                        },
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
            match build(commit.as_deref(), &cancel, &tx) {
                Ok((doc, raw, _)) => {
                    if let Ok(cache) = Cache::new() {
                        if let Err(e) = cache.save(&raw) {
                            let _ = tx.send(IndexEvent::Failed {
                                msg: format!("cache save failed: {e}"),
                            });
                        }
                    }
                    match Index::from_doc(doc, now_ms()) {
                        Ok(index) => {
                            let _ = tx.send(IndexEvent::Ready {
                                index: Arc::new(index),
                                fresh: false,
                                unkeyed: commit.is_none(),
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

/// A temp file removed on drop.
struct TempScript(PathBuf);

impl Drop for TempScript {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Run the embedded Guile script through `guix repl` and return the parsed
/// document plus raw bytes (for the cache). Blocking; call from a worker
/// thread. Progress lines are relayed through `progress`.
pub fn build(
    commit: Option<&str>,
    cancel: &Cancel,
    progress: &Sender<IndexEvent>,
) -> Result<(crate::model::IndexDoc, Vec<u8>, String), IndexerError> {
    let started = Instant::now();
    let guix = find_guix().ok_or_else(|| IndexerError::NotFound(guix_search_paths()))?;

    // Write the embedded script next to the system temp dir.
    let script_path =
        std::env::temp_dir().join(format!("guixvis-index-{}.scm", std::process::id()));
    std::fs::write(&script_path, INDEX_SCRIPT)
        .map_err(|e| IndexerError::Exited(format!("cannot write temp script: {e}")))?;
    let _script_guard = TempScript(script_path.clone());

    let commit = commit.unwrap_or("");
    let generated_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
        .to_string();

    let mut child = Command::new(&guix)
        .arg("repl")
        .arg("--")
        .arg(&script_path)
        .arg(commit)
        .arg(&generated_ms)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| IndexerError::Exited(format!("cannot spawn guix: {e}")))?;

    // Drain stdout on its own thread; bounded by MAX_OUTPUT.
    let mut stdout = child.stdout.take().expect("stdout piped");
    let stdout_reader: JoinHandle<Result<Vec<u8>, String>> = std::thread::Builder::new()
        .name("guixvis-indexer-stdout".into())
        .spawn(move || {
            let mut buf = Vec::with_capacity(32 * 1024 * 1024);
            let mut chunk = [0u8; 64 * 1024];
            loop {
                let n = stdout.read(&mut chunk).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                if buf.len() as u64 + n as u64 > MAX_OUTPUT {
                    return Err("indexer output exceeded 256 MiB".into());
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            Ok(buf)
        })
        .expect("failed to spawn stdout reader");

    // Drain stderr, relaying PROGRESS lines.
    let mut stderr = child.stderr.take().expect("stderr piped");
    let tx_prog = progress.clone();
    let stderr_reader: JoinHandle<()> = std::thread::Builder::new()
        .name("guixvis-indexer-stderr".into())
        .spawn(move || {
            let reader = BufReader::new(&mut stderr);
            for line in reader.lines().map_while(Result::ok) {
                if let Some(rest) = line.strip_prefix("PROGRESS ") {
                    let mut parts = rest.split_whitespace();
                    if let (Some(d), Some(t)) = (parts.next(), parts.next()) {
                        if let (Ok(done), Ok(total)) = (d.parse::<u64>(), t.parse::<u64>()) {
                            let _ = tx_prog.send(IndexEvent::Progress { done, total });
                        }
                    }
                }
            }
        })
        .expect("failed to spawn stderr reader");

    // Poll for completion with a hard deadline and cancellation.
    let deadline = started + TIMEOUT;
    let status = loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return Err(IndexerError::Exited("cancelled".into()));
        }
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = stdout_reader.join();
                    let _ = stderr_reader.join();
                    return Err(IndexerError::Timeout(TIMEOUT.as_secs()));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                return Err(IndexerError::Exited(format!("cannot wait for guix: {e}")));
            }
        }
    };

    let raw = stdout_reader
        .join()
        .map_err(|_| IndexerError::Exited("stdout reader panicked".into()))?
        .map_err(IndexerError::Exited)?;
    let _ = stderr_reader.join();

    if !status.success() {
        let tail: String = String::from_utf8_lossy(&raw)
            .chars()
            .rev()
            .take(400)
            .collect();
        return Err(IndexerError::Exited(format!(
            "guix repl exited with {status}; output tail: {tail}"
        )));
    }
    if raw.is_empty() {
        return Err(IndexerError::NoOutput);
    }

    let doc: crate::model::IndexDoc = serde_json::from_slice(&raw)
        .map_err(|e| IndexerError::Exited(format!("cannot parse indexer JSON: {e}")))?;
    Ok((doc, raw, commit.to_string()))
}

fn guix_search_paths() -> String {
    let mut parts = [
        "$GUIX/bin/guix",
        "/run/current-system/profile/bin/guix",
        "~/.config/guix/current/bin/guix",
        "~/.guix-profile/bin/guix",
        "$PATH/guix",
    ]
    .join(", ");
    parts.push_str(". Install GNU Guix or export GUIX to its profile.");
    parts
}
