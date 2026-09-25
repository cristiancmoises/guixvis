//! Resolve one Guix executable and fingerprint its complete channel environment.
use crate::model::{ChannelPin, GuixOrigin};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub fn resolve_guix(
    override_profile: Option<&Path>,
    path: Option<&OsStr>,
    fallbacks: &[PathBuf],
) -> Result<PathBuf, String> {
    fn executable(path: &Path) -> bool {
        let Ok(meta) = path.metadata() else {
            return false;
        };
        if !meta.is_file() {
            return false;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            meta.permissions().mode() & 0o111 != 0
        }
        #[cfg(not(unix))]
        {
            true
        }
    }
    if let Some(profile) = override_profile {
        let candidate = if profile.is_file() {
            profile.to_path_buf()
        } else {
            profile.join("bin/guix")
        };
        if !executable(&candidate) {
            return Err("GUIX does not select an executable Guix profile or binary".into());
        }
        // Guix discovers profile channel extensions relative to argv[0].
        // Resolving symlinks here silently drops those channels.
        return std::path::absolute(candidate).map_err(|e| e.to_string());
    }
    let paths = path
        .map(std::env::split_paths)
        .into_iter()
        .flatten()
        .map(|p| p.join("guix"));
    for candidate in paths.chain(fallbacks.iter().cloned()) {
        if executable(&candidate) {
            return std::path::absolute(candidate).map_err(|e| e.to_string());
        }
    }
    Err("Guix not found in GUIX, PATH or standard profiles".into())
}

pub fn current_guix() -> Result<PathBuf, String> {
    let mut fallbacks = vec![PathBuf::from("/run/current-system/profile/bin/guix")];
    if let Some(home) = dirs::home_dir() {
        fallbacks.push(home.join(".config/guix/current/bin/guix"));
        fallbacks.push(home.join(".guix-profile/bin/guix"));
    }
    let explicit = std::env::var_os("GUIX")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    resolve_guix(
        explicit.as_deref(),
        std::env::var_os("PATH").as_deref(),
        &fallbacks,
    )
}

/// Use the same bounded/cancellable process boundary as full index extraction.
fn capture(
    guix: &Path,
    args: &[&OsStr],
    cancel: &AtomicBool,
    timeout: Duration,
) -> Option<Vec<u8>> {
    crate::process::capture(
        Command::new(guix).args(args),
        cancel,
        timeout,
        1024 * 1024,
        |_| {},
    )
    .ok()
}

pub fn probe_origin(guix: &Path, cancel: &AtomicBool) -> Result<GuixOrigin, String> {
    let mut origin = GuixOrigin {
        executable: guix.to_string_lossy().into_owned(),
        mutable_package_path: std::env::var_os("GUIX_PACKAGE_PATH").is_some_and(|v| !v.is_empty()),
        ..GuixOrigin::default()
    };
    if let Some(bytes) = capture(
        guix,
        &[OsStr::new("describe"), OsStr::new("--format=json")],
        cancel,
        Duration::from_secs(15),
    ) {
        if let Ok(mut channels) = serde_json::from_slice::<Vec<ChannelPin>>(&bytes) {
            if channels.len() <= 4096
                && channels.iter().all(|c| {
                    !c.name.is_empty()
                        && !c.commit.is_empty()
                        && c.name.len() <= 128
                        && c.commit.len() <= 128
                        && c.name
                            .chars()
                            .chain(c.commit.chars())
                            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
                })
            {
                channels.sort();
                channels.dedup();
                origin.channels = channels;
            }
        }
    }
    if cancel.load(Ordering::Relaxed) {
        return Err("cancelled".into());
    }
    let (script, _guard) = crate::indexer::write_private_script(
        "(use-modules (guix utils))\n(display (%current-system))\n",
    )
    .map_err(|e| format!("cannot create origin probe: {e}"))?;
    if let Some(bytes) = capture(
        guix,
        &[
            OsStr::new("repl"),
            OsStr::new("-q"),
            OsStr::new("--"),
            script.as_os_str(),
        ],
        cancel,
        Duration::from_secs(15),
    ) {
        if let Ok(system) = String::from_utf8(bytes) {
            let system = system.trim();
            if !system.is_empty()
                && system.len() <= 128
                && system
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_".contains(c))
            {
                origin.system = system.into();
            }
        }
    }
    if cancel.load(Ordering::Relaxed) {
        return Err("cancelled".into());
    }
    origin.verified = true;
    origin.verified = origin.is_verified();
    Ok(origin)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::time::Instant;

    fn capture_script(body: &str, timeout: Duration, cancel: &AtomicBool) -> Option<Vec<u8>> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _lock = LOCK.lock().unwrap();
        let shell = std::env::split_paths(&std::env::var_os("PATH").unwrap())
            .map(|p| p.join("sh"))
            .find(|p| p.is_file())
            .unwrap();
        let shell = std::fs::canonicalize(shell).unwrap();
        let (script, _guard) =
            crate::indexer::write_private_script(&format!("#!{}\n{body}\n", shell.display()))
                .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        capture(&script, &[], cancel, timeout)
    }

    #[test]
    fn probes_bound_stdout_discard_stderr_and_observe_cancellation() {
        let cancel = AtomicBool::new(false);
        assert_eq!(
            capture_script(
                "printf '%02000000d' 1 >&2; printf ok",
                Duration::from_secs(2),
                &cancel
            ),
            Some(b"ok".to_vec())
        );
        assert!(capture_script("printf '%02000000d' 1", Duration::from_secs(2), &cancel).is_none());
        let start = Instant::now();
        assert!(capture_script("exec sleep 2", Duration::from_millis(50), &cancel).is_none());
        assert!(start.elapsed() < Duration::from_secs(1));
        cancel.store(true, Ordering::Relaxed);
        assert!(capture_script("printf unexpected", Duration::from_secs(2), &cancel).is_none());
    }

    #[test]
    fn explicit_then_path_then_fallback_without_silent_override_fallback() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("guixvis-origin-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        let profile = dir.join("profile");
        let path_dir = dir.join("path with spaces");
        std::fs::create_dir_all(profile.join("bin")).unwrap();
        std::fs::create_dir(&path_dir).unwrap();
        let explicit = profile.join("bin/guix");
        let on_path = path_dir.join("guix");
        let fallback = dir.join("fallback");
        for p in [&explicit, &on_path, &fallback] {
            std::fs::write(p, b"fixture").unwrap();
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let paths = std::env::join_paths([&path_dir]).unwrap();
        assert_eq!(
            resolve_guix(
                Some(&profile),
                Some(&paths),
                std::slice::from_ref(&fallback)
            )
            .unwrap(),
            explicit
        );
        assert_eq!(
            resolve_guix(None, Some(&paths), std::slice::from_ref(&fallback)).unwrap(),
            on_path
        );
        assert_eq!(
            resolve_guix(None, None, std::slice::from_ref(&fallback)).unwrap(),
            fallback
        );
        assert!(resolve_guix(Some(&dir.join("missing")), Some(&paths), &[fallback]).is_err());
        let launcher = path_dir.join("launcher");
        std::os::unix::fs::symlink(&explicit, &launcher).unwrap();
        assert_eq!(
            resolve_guix(Some(&launcher), None, &[]).unwrap(),
            launcher,
            "Guix resolves channel extensions relative to its launcher, not the symlink target"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
