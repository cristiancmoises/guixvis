//! Bounded read-only Guix subprocesses, isolated from the caller's process group.
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

/// Drain both streams without blocking on inherited pipes. Only complete,
/// bounded stderr lines reach the progress callback; all other stderr is dropped.
#[cfg(target_os = "linux")]
pub(crate) fn capture(
    command: &mut Command,
    cancel: &AtomicBool,
    timeout: Duration,
    limit: usize,
    mut on_line: impl FnMut(&str),
) -> Result<Vec<u8>, String> {
    use nix::fcntl::{fcntl, FcntlArg, OFlag};
    use nix::sys::signal::{killpg, Signal};
    use nix::sys::wait::{waitid, Id, WaitPidFlag, WaitStatus};
    use nix::unistd::Pid;
    use std::io::{ErrorKind, Read};
    use std::os::fd::AsRawFd;
    use std::os::unix::process::CommandExt;
    use std::process::{Child, Stdio};
    use std::sync::atomic::Ordering;
    use std::time::Instant;

    struct OwnedChild(Child);
    impl Drop for OwnedChild {
        fn drop(&mut self) {
            // The leader is NOT reaped during polling (WNOWAIT), so its PID
            // cannot be reused before we signal our group and finally reap it.
            if let Ok(pid) = i32::try_from(self.0.id()) {
                if pid > 1 {
                    let _ = killpg(Pid::from_raw(pid), Signal::SIGKILL);
                }
            }
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    if cancel.load(Ordering::Relaxed) {
        return Err("cancelled".into());
    }
    let mut child = OwnedChild(
        command
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?,
    );
    let mut stdout = child.0.stdout.take().ok_or("missing stdout")?;
    let mut stderr = child.0.stderr.take().ok_or("missing stderr")?;
    for fd in [stdout.as_raw_fd(), stderr.as_raw_fd()] {
        let flags = fcntl(fd, FcntlArg::F_GETFL).map_err(|e| e.to_string())?;
        fcntl(
            fd,
            FcntlArg::F_SETFL(OFlag::from_bits_truncate(flags) | OFlag::O_NONBLOCK),
        )
        .map_err(|e| e.to_string())?;
    }
    let deadline = Instant::now() + timeout;
    let mut output = Vec::new();
    let mut line = Vec::new();
    let mut oversized_line = false;
    let mut out_done = false;
    let mut err_done = false;
    let mut status = None;
    let mut chunk = [0u8; 64 * 1024];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        if Instant::now() >= deadline {
            return Err(format!("timed out after {}s", timeout.as_secs()));
        }
        let mut received = false;
        if !out_done {
            match stdout.read(&mut chunk) {
                Ok(0) => out_done = true,
                Ok(n) => {
                    if n > limit.saturating_sub(output.len()) {
                        return Err("subprocess output limit exceeded".into());
                    }
                    output.extend_from_slice(&chunk[..n]);
                    received = true;
                }
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) => {}
                Err(e) => return Err(e.to_string()),
            }
        }
        if !err_done {
            match stderr.read(&mut chunk) {
                Ok(0) => err_done = true,
                Ok(n) => {
                    received = true;
                    for byte in &chunk[..n] {
                        if *byte == b'\n' {
                            if !oversized_line {
                                if let Ok(text) = std::str::from_utf8(&line) {
                                    on_line(text);
                                }
                            }
                            line.clear();
                            oversized_line = false;
                        } else if line.len() < 4096 {
                            line.push(*byte);
                        } else {
                            oversized_line = true;
                        }
                    }
                }
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) => {}
                Err(e) => return Err(e.to_string()),
            }
        }
        if status.is_none() {
            match waitid(
                Id::Pid(Pid::from_raw(child.0.id() as i32)),
                WaitPidFlag::WEXITED | WaitPidFlag::WNOHANG | WaitPidFlag::WNOWAIT,
            )
            .map_err(|e| e.to_string())?
            {
                WaitStatus::StillAlive => {}
                exit => status = Some(exit),
            }
        }
        if let Some(exit) = status {
            if !matches!(exit, WaitStatus::Exited(_, 0)) {
                return Err(format!("Guix exited with {exit:?}"));
            }
            if out_done && err_done {
                return Ok(output);
            }
        }
        if !received {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn capture(
    _: &mut Command,
    _: &AtomicBool,
    _: Duration,
    _: usize,
    _: impl FnMut(&str),
) -> Result<Vec<u8>, String> {
    Err("Guix indexing requires a Linux host".into())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn inherited_pipes_deadline_and_stream_limits() {
        let cancel = AtomicBool::new(false);
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 2 & printf ok"]);
        let started = Instant::now();
        assert!(capture(
            &mut command,
            &cancel,
            Duration::from_millis(100),
            100,
            |_| {}
        )
        .is_err());
        assert!(started.elapsed() < Duration::from_secs(1));
        let mut lines = Vec::new();
        let mut command = Command::new("sh");
        command.args([
            "-c",
            "printf '%02000000d\\n' 1 >&2; printf 'PROGRESS 1 2\\n' >&2; printf ok",
        ]);
        assert_eq!(
            capture(&mut command, &cancel, Duration::from_secs(2), 100, |s| {
                lines.push(s.to_string())
            })
            .unwrap(),
            b"ok"
        );
        assert_eq!(lines, ["PROGRESS 1 2"]);
        let mut command = Command::new("sh");
        command.args(["-c", "printf '%01000d' 1"]);
        assert!(capture(&mut command, &cancel, Duration::from_secs(2), 100, |_| {}).is_err());
    }
}
