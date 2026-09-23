//! Binary index snapshot under `$XDG_CACHE_HOME/guixvis/`.
//!
//! The snapshot holds the resolved index (see [`crate::blob`]), which loads in
//! a fraction of the time the indexer's JSON needs. Writes are transactional
//! (temp file + atomic rename); corrupt files are quarantined instead of
//! silently deleted.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::blob;
use crate::error::CacheError;
use crate::index::Index;

const MAX_SNAPSHOT_BYTES: u64 = 512 * 1024 * 1024;
const MAX_DESCRIBE_BYTES: u64 = 1024 * 1024;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Whether a cached index is usable and current.
#[derive(Debug)]
pub enum CacheStatus {
    /// Snapshot decoded and matches the live channel commit.
    Fresh(Index),
    /// Snapshot is readable but was produced against a different Guix commit.
    Stale { reason: String },
    /// No cache file present.
    Absent,
}

pub struct Cache {
    dir: PathBuf,
}

impl Cache {
    pub fn new() -> Result<Self, CacheError> {
        let base = dirs::cache_dir().ok_or_else(|| {
            CacheError::Read("no cache directory available (XDG_CACHE_HOME unset?)".into())
        })?;
        let dir = base.join("guixvis");
        fs::create_dir_all(&dir).map_err(|e| CacheError::Read(e.to_string()))?;
        Ok(Cache { dir })
    }

    /// Build a cache rooted at an explicit directory (used by tests).
    pub fn at(dir: PathBuf) -> Self {
        let _ = fs::create_dir_all(&dir);
        Cache { dir }
    }

    pub fn path(&self) -> PathBuf {
        self.dir.join("index-v4.bin")
    }

    /// Load the cached snapshot. `live_commit` is the commit of the currently
    /// active Guix channel (`None` when it cannot be determined — the cache is
    /// then accepted with an "unverified" badge by the caller).
    pub fn load(
        &self,
        live_commit: Option<&str>,
        built_ms: u64,
    ) -> Result<CacheStatus, CacheError> {
        let path = self.path();
        if !path.exists() {
            return Ok(CacheStatus::Absent);
        }
        // A snapshot is ~14 MB for 32.5k packages; anything far beyond that
        // is not ours and must not be read into memory.
        let file = File::open(&path).map_err(|e| CacheError::Read(e.to_string()))?;
        let len = file
            .metadata()
            .map_err(|e| CacheError::Read(e.to_string()))?
            .len();
        if len > MAX_SNAPSHOT_BYTES {
            return Err(CacheError::Read(format!(
                "snapshot is implausibly large ({len} bytes)"
            )));
        }
        // Read the same open file we inspected and retain the cap if another
        // writer grows it after the metadata check.
        let mut bytes = Vec::with_capacity(len as usize);
        file.take(MAX_SNAPSHOT_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| CacheError::Read(e.to_string()))?;
        if bytes.len() as u64 > MAX_SNAPSHOT_BYTES {
            return Err(CacheError::Read(
                "snapshot exceeded the size limit while reading".into(),
            ));
        }
        let index = blob::decode(&bytes, built_ms).map_err(|e| CacheError::Parse(e.to_string()))?;

        if let Some(live) = live_commit.filter(|c| !c.is_empty()) {
            if index.guix_commit != live {
                return Ok(CacheStatus::Stale {
                    reason: format!("cache commit {} != live commit {}", index.guix_commit, live),
                });
            }
        }
        Ok(CacheStatus::Fresh(index))
    }

    /// Atomically persist a snapshot of `index`.
    pub fn save(&self, index: &Index) -> Result<(), CacheError> {
        let bytes = blob::encode(index);
        let (tmp, mut file) = self.create_temp()?;
        let result = (|| -> Result<(), CacheError> {
            file.write_all(&bytes)
                .map_err(|e| CacheError::Write(e.to_string()))?;
            file.sync_all()
                .map_err(|e| CacheError::Write(e.to_string()))?;
            fs::rename(&tmp, self.path()).map_err(|e| CacheError::Write(e.to_string()))?;
            // The gzipped-JSON cache of 0.2.x has no reader any more.
            let _ = fs::remove_file(self.dir.join("index-v3.json.gz"));
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        result
    }

    /// Exclusive creation prevents existing files and symlinks from being
    /// followed or truncated. Each concurrent writer owns its own temporary.
    fn create_temp(&self) -> Result<(PathBuf, File), CacheError> {
        for _ in 0..128 {
            let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = self.dir.join(format!(
                "index-v4.bin.tmp-{}-{sequence}",
                std::process::id()
            ));
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(&path) {
                Ok(file) => return Ok((path, file)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(CacheError::Write(e.to_string())),
            }
        }
        Err(CacheError::Write(
            "could not reserve a cache temporary file".into(),
        ))
    }

    /// Move a broken cache file out of the way instead of deleting evidence.
    pub fn quarantine(&self) {
        let path = self.path();
        if !path.exists() {
            return;
        }
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let dest = self.dir.join(format!("index-v4.bin.corrupt-{ts}"));
        let _ = fs::rename(&path, &dest);
    }
}

/// Small helper: canonical location of the `guix` binary.
pub fn find_guix() -> Option<PathBuf> {
    let candidates: Vec<PathBuf> = [
        std::env::var_os("GUIX")
            .map(PathBuf::from)
            .map(|p| p.join("bin").join("guix")),
        Some(PathBuf::from("/run/current-system/profile/bin/guix")),
        dirs::home_dir().map(|h| h.join(".config/guix/current/bin/guix")),
        dirs::home_dir().map(|h| h.join(".guix-profile/bin/guix")),
    ]
    .into_iter()
    .flatten()
    .collect();
    candidates
        .iter()
        .find(|p| p.exists())
        .cloned()
        .or_else(which_guix)
}

fn which_guix() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join("guix"))
        .find(|p| p.is_file())
}

/// Read the active channel commit via `guix describe --format=json`.
/// Returns `None` when the command fails or exceeds 15 seconds.
pub fn describe_commit(guix: &Path) -> Option<String> {
    describe_commit_with_timeout(guix, Duration::from_secs(15))
}

fn describe_commit_with_timeout(guix: &Path, timeout: Duration) -> Option<String> {
    let deadline = Instant::now() + timeout;
    let mut child = std::process::Command::new(guix)
        .args(["describe", "--format=json"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let stdout = child.stdout.take()?;
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    // Drain concurrently so a full pipe cannot block the child. The reader
    // is bounded; the caller enforces the deadline even if a descendant
    // retains the stdout pipe after the direct child exits.
    let reader = std::thread::Builder::new()
        .name("guixvis-describe".into())
        .spawn(move || {
            let mut bytes = Vec::new();
            let result = stdout
                .take(MAX_DESCRIBE_BYTES + 1)
                .read_to_end(&mut bytes)
                .ok()
                .filter(|_| bytes.len() as u64 <= MAX_DESCRIBE_BYTES)
                .map(|_| bytes);
            let _ = tx.send(result);
        });
    if reader.is_err() {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    }
    let mut out = None;
    let status = loop {
        match rx.try_recv() {
            Ok(Some(bytes)) => out = Some(bytes),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) if out.is_none() => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            _ => {}
        }
        match child.try_wait() {
            Ok(Some(s)) if !s.success() || out.is_some() => break s,
            Ok(_) if Instant::now() < deadline => {
                std::thread::sleep(
                    Duration::from_millis(10)
                        .min(deadline.saturating_duration_since(Instant::now())),
                );
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    let out = out?;
    if !status.success() || out.is_empty() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(&out).ok()?;
    v.as_array()?
        .first()?
        .get("commit")?
        .as_str()
        .map(ToOwned::to_owned)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    // A simultaneous fork can briefly inherit another test's writable script
    // descriptor before exec closes it, making that script fail with ETXTBSY.
    static SCRIPT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn describe_script(body: &str, timeout: Duration) -> Option<String> {
        let _guard = SCRIPT_LOCK.lock().unwrap();
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "guixvis-describe-test-{}-{sequence}",
            std::process::id()
        ));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        write!(file, "#!/bin/sh\n{body}\n").unwrap();
        file.set_permissions(fs::Permissions::from_mode(0o700))
            .unwrap();
        drop(file);
        let result = describe_commit_with_timeout(&path, timeout);
        fs::remove_file(path).unwrap();
        result
    }

    #[test]
    fn describe_reads_a_successful_bounded_response() {
        assert_eq!(
            describe_script("printf '[{\"commit\":\"abc123\"}]'", Duration::from_secs(1)),
            Some("abc123".into())
        );
    }

    #[test]
    fn describe_deadline_covers_stdout_reading() {
        let start = Instant::now();
        assert_eq!(
            describe_script("exec sleep 2", Duration::from_millis(50)),
            None
        );
        assert!(start.elapsed() >= Duration::from_millis(50));
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn describe_rejects_excessive_stdout() {
        assert_eq!(
            describe_script("printf '%02000000d' 1", Duration::from_secs(1)),
            None
        );
    }
}
