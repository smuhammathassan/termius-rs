//! Single-instance guard: a PID-stamped lock file in the data directory.
//!
//! Replaces Electron's `requestSingleInstanceLock` / `second-instance`
//! behaviour: the first process owns `<data dir>/termius-app.lock`; a second
//! launch finds the file, probes whether the recorded PID is still alive, and
//! exits gracefully when it is.
//!
//! Design notes:
//! * **Std-only** — no `flock`/`fs2` dependency was added (crate dep set is
//!   fixed); the PID file + liveness probe covers the real-world cases.
//! * **Crash recovery** — a lock whose holder died is detected as *stale* and
//!   taken over, so a crash never permanently blocks startup.
//! * **Release** — `release()` (also run on `Drop`) removes the file exactly
//!   once, so a late release can never delete a newer instance's lock.
//!
//! PORT-TODO: the Electron original *focused* the existing window via the
//! `second-instance` event; here the second process only reports and exits.
//! Focus hand-off needs a small IPC (macOS `NSRunningApplication::activate`,
//! or a unix-socket ping answered by termius-ui).

use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context as _, Result};

/// Lock file inside the data directory; content = holder PID + newline.
pub const LOCK_FILE_NAME: &str = "termius-app.lock";

/// Held for as long as this process is the running instance.
pub struct InstanceLock {
    path: PathBuf,
    released: AtomicBool,
}

impl InstanceLock {
    /// Try to become the running instance.
    ///
    /// * `Ok(Some(lock))` — we own the lock (keep it alive; releasing on drop).
    /// * `Ok(None)` — another live instance owns it; the caller should exit.
    /// * `Err(_)` — the data directory could not be created/written.
    pub fn acquire(data_dir: &Path) -> Result<Option<Self>> {
        fs::create_dir_all(data_dir).with_context(|| {
            format!("could not create the data directory {}", data_dir.display())
        })?;
        let path = data_dir.join(LOCK_FILE_NAME);

        match try_create(&path) {
            Ok(()) => return Ok(Some(Self::new(path))),
            // Held (or left behind) by someone else — investigated below.
            Err(err) if err.kind() == ErrorKind::AlreadyExists => {}
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("could not write the lock file {}", path.display()));
            }
        }

        // The lock file exists: live holder → step aside; dead/unreadable →
        // take over (stale-lock recovery after a crash).
        match read_pid(&path) {
            Some(pid) if pid_is_alive(pid) => {
                tracing::info!(pid, path = %path.display(), "another instance is already running");
                return Ok(None);
            }
            Some(pid) => tracing::warn!(pid, "stale lock file (holder is gone); taking over"),
            None => tracing::warn!("unreadable lock file; taking over"),
        }

        let _ = fs::remove_file(&path);
        match try_create(&path) {
            Ok(()) => Ok(Some(Self::new(path))),
            Err(err) => {
                // Raced with another launch that won the retry.
                tracing::warn!(error = %err, "lost the race for the lock file");
                Ok(None)
            }
        }
    }

    /// The PID recorded in the lock file, if any (`doctor` reports it).
    pub fn holder_pid(data_dir: &Path) -> Option<u32> {
        read_pid(&data_dir.join(LOCK_FILE_NAME))
    }

    /// Remove the lock file exactly once (idempotent; also runs on `Drop`).
    pub fn release(&self) {
        if !self.released.swap(true, Ordering::SeqCst) {
            let _ = fs::remove_file(&self.path);
        }
    }

    fn new(path: PathBuf) -> Self {
        Self {
            path,
            released: AtomicBool::new(false),
        }
    }
}

impl Drop for InstanceLock {
    fn drop(&mut self) {
        self.release();
    }
}

/// Create the lock file exclusively and stamp it with our PID.
fn try_create(path: &Path) -> std::io::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true) // O_EXCL semantics: atomic create-or-fail
        .open(path)?;
    let pid = std::process::id();
    file.write_all(pid.to_string().as_bytes())?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}

/// Read the holder PID from the lock file (missing/unreadable/invalid → None).
fn read_pid(path: &Path) -> Option<u32> {
    let contents = fs::read_to_string(path).ok()?;
    let pid: u32 = contents.trim().parse().ok()?;
    (pid > 0).then_some(pid)
}

/// Best-effort liveness probe for the recorded PID (POSIX: ask `ps`).
///
/// Std-only — no libc/`kill(2)` binding in the dep set. On non-POSIX targets
/// there is no portable probe without an extra dependency, so a held lock is
/// conservatively assumed live.
/// PORT-TODO: use `OpenProcess` on Windows once that target matters.
#[cfg(unix)]
fn pid_is_alive(pid: u32) -> bool {
    let pid = pid.to_string();
    std::process::Command::new("ps")
        .args(["-p", pid.as_str(), "-o", "pid="])
        .output()
        .map(|out| out.status.success() && !out.stdout.is_empty())
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn pid_is_alive(_pid: u32) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acquire_is_exclusive_and_released_on_drop() {
        let dir =
            std::env::temp_dir().join(format!("termius-app-lock-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);

        let first = InstanceLock::acquire(&dir).expect("first acquire");
        assert!(first.is_some(), "first acquire must win");
        assert!(dir.join(LOCK_FILE_NAME).exists(), "lock file written");

        // Same process still counts as a live holder.
        let second = InstanceLock::acquire(&dir).expect("second acquire");
        assert!(
            second.is_none(),
            "second acquire must lose to a live holder"
        );
        assert_eq!(
            InstanceLock::holder_pid(&dir),
            Some(std::process::id()),
            "lock file records our PID"
        );

        drop(first);
        assert!(!dir.join(LOCK_FILE_NAME).exists(), "drop released the lock");

        let third = InstanceLock::acquire(&dir).expect("third acquire");
        assert!(third.is_some(), "acquire works again after release");
        drop(third);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn release_is_idempotent() {
        let dir =
            std::env::temp_dir().join(format!("termius-app-release-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);

        let lock = InstanceLock::acquire(&dir)
            .expect("acquire")
            .expect("we win");
        lock.release();
        lock.release(); // must not panic or misbehave
        assert!(!dir.join(LOCK_FILE_NAME).exists());

        drop(lock); // Drop after explicit release: still fine
        let _ = fs::remove_dir_all(&dir);
    }
}
