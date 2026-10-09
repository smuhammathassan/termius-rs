//! `MoshClient`: mosh (mobile shell) sessions by shelling out to the system
//! `mosh` binary.
//!
//! The Electron app bundled the native `@termius/mosh` addon
//! (`moshclient.node`), which embedded both halves of the mosh design: an SSH
//! bootstrap that launches `mosh-server` on the remote, and the local
//! `mosh-client` speaking the SSHP state-synchronization protocol over UDP.
//! Re-implementing SSHP in Rust is a large protocol effort; per the porting
//! plan (`@termius/mosh` → "shell-out / deferred") the pragmatic 1:1 bridge is
//! the upstream `mosh` client itself, which this module drives as a child
//! process:
//!
//! * [`MoshClient::connect`] verifies a `mosh` binary is on `PATH`
//!   ([`find_mosh_binary`]), materialises an SSH private key (if that is the
//!   auth method) to a `0600` temp file, and spawns
//!   `mosh --ssh=<ssh-command> <user>@<host>` with piped stdio.
//! * [`MoshClient::recv`] / [`MoshClient::write`] bridge the session as an
//!   async byte stream over the child's stdout/stdin — the same shape the
//!   SSH shell provider exposes to the terminal.
//! * [`MoshClient::resize`] records the new size (see PORT-TODO below).
//! * [`MoshClient::disconnect`] closes stdin, waits, kills if needed, and
//!   wipes the temp key file.
//!
//! The SSH phase (bootstrap) is delegated to the system `ssh` via mosh's
//! `--ssh` option, so agent/key/known-host handling stays with OpenSSH, same
//! as `mosh` behaves outside Termius. Bootstrap failures (bad credentials,
//! host unreachable, …) surface as [`MoshError::Connect`] with the captured
//! stderr tail on the first [`recv`](MoshClient::recv) after the child dies.
//!
//! # Known gaps (shell-out bridge)
//!
//! * **No PTY**: `mosh-client` wants a real terminal on stdin/stdout (it
//!   issues `tcgetattr`); with piped stdio a strict `mosh-client` build may
//!   refuse the session. This bridge keeps the pipe model for portability and
//!   because it matches the app's byte-stream API; a PTY-backed spawn (via a
//!   `openpty`/`portable-pty` dependency) is the drop-in upgrade path.
//! * **resize** cannot be delivered without a PTY (see
//!   [`MoshClient::resize`]).
//! * Password / encrypted-key auth cannot ride the non-interactive system
//!   `ssh`; those return [`MoshError::Unsupported`] with the reason preserved.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use termius_core::{AuthMethod, ConnectionParams, TerminalSize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use crate::error::{MoshError, Result};

/// `$TERM` advertised to the spawned `mosh` (mosh forwards it to the remote
/// `mosh-server`). Matches the term used by the SSH engine.
pub const DEFAULT_TERM: &str = "xterm-256color";

/// How long `disconnect` / early-EOF handling waits for the child to exit on
/// its own before killing it.
const EXIT_WAIT: Duration = Duration::from_secs(2);

/// Cap on the captured bootstrap stderr tail (bytes). The most recent bytes
/// are kept so the actual ssh/mosh error message survives.
const STDERR_TAIL_CAP: usize = 8 * 1024;

/// Counter making temp key file names unique within one process.
static KEY_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A mosh session backed by a `mosh` child process.
///
/// Create with [`MoshClient::connect`]; read remote output with
/// [`recv`](Self::recv), send keystrokes with [`write`](Self::write), and
/// tear down with [`disconnect`](Self::disconnect).
pub struct MoshClient {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: Option<ChildStdout>,
    /// Shared with the stderr-drain task; holds the trailing bootstrap
    /// output for error reporting.
    stderr_tail: Arc<Mutex<Vec<u8>>>,
    /// Temp private-key file to wipe on disconnect/drop, if any.
    key_file: Option<PathBuf>,
    /// Full display command line (`mosh --ssh=… user@host`), for errors/logs.
    command_line: String,
    /// `user@host` target.
    target: String,
    /// Last size handed to [`resize`](Self::resize) (kept for the future
    /// PTY-backed resize).
    size: TerminalSize,
    /// Whether any session bytes were ever received (distinguishes a
    /// bootstrap failure from a mid-session exit).
    received_any: bool,
    eof: bool,
    closed: bool,
    exit_status: Option<ExitStatus>,
}

impl MoshClient {
    /// Spawn a mosh session for `params`.
    ///
    /// Returns as soon as the `mosh` child is spawned — the SSH bootstrap
    /// runs inside the child, and bootstrap failures surface on the first
    /// [`recv`](Self::recv) as [`MoshError::Connect`] (mirroring how the
    /// Electron app reported connect errors asynchronously via a callback).
    ///
    /// Auth mapping:
    /// * `None` / `Agent` — passed through; the system `ssh` invoked by mosh
    ///   uses the inherited `SSH_AUTH_SOCK` / default identities.
    /// * `PrivateKey` (unencrypted) — materialised to a `0600` temp file and
    ///   passed as `-i` with `IdentitiesOnly=yes`; wiped on disconnect.
    /// * `Password` / encrypted key — [`MoshError::Unsupported`]: the
    ///   shell-out bridge has no askpass channel into system `ssh`.
    pub async fn connect(params: &ConnectionParams) -> Result<Self> {
        let host = params.host.trim();
        let username = params.username.trim();
        if host.is_empty() {
            return Err(MoshError::connect(
                if username.is_empty() {
                    "<empty host>".to_string()
                } else {
                    format!("{username}@")
                },
                "host is empty",
            ));
        }
        let target = if username.is_empty() {
            host.to_string()
        } else {
            format!("{username}@{host}")
        };

        let mosh_path = find_mosh_binary().ok_or(MoshError::ClientNotFound)?;

        // Resolve auth to either "no key file" or a materialised temp key.
        let key_file = match &params.auth {
            AuthMethod::None | AuthMethod::Agent => None,
            AuthMethod::Password(_) => {
                return Err(MoshError::unsupported(
                    "password authentication is not supported by the shell-out mosh \
                     bridge (system ssh cannot be prompted non-interactively); use an \
                     SSH agent or key file",
                ));
            }
            AuthMethod::PrivateKey { private_key, passphrase } => {
                if passphrase.is_some() {
                    return Err(MoshError::unsupported(
                        "encrypted private keys are not supported by the shell-out mosh \
                         bridge; unlock the key via an SSH agent instead",
                    ));
                }
                Some(write_temp_key(private_key)?)
            }
        };

        let ssh_cmd = build_ssh_command(params, key_file.as_deref());

        // argv for the child; `command_line` is the display/error form.
        let mut args: Vec<String> = vec![format!("--ssh={ssh_cmd}"), target.clone()];
        if let Some(remote_command) = params
            .command
            .as_deref()
            .map(str::trim)
            .filter(|c| !c.is_empty())
        {
            args.push("--".to_string());
            args.push(remote_command.to_string());
        }
        let command_line = format!(
            "mosh {}",
            args.iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(" ")
        );

        let mut cmd = Command::new(&mosh_path);
        cmd.args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .env("TERM", DEFAULT_TERM);

        let child = match cmd.spawn() {
            Ok(child) => child,
            Err(source) => {
                // Don't leak the temp key if we never got a session.
                if let Some(path) = &key_file {
                    let _ = std::fs::remove_file(path);
                }
                return Err(MoshError::spawn(command_line, source));
            }
        };

        let stderr_tail = Arc::new(Mutex::new(Vec::new()));
        let mut child = child;
        spawn_stderr_drain(child.stderr.take(), Arc::clone(&stderr_tail));

        let stdin = child.stdin.take();
        let stdout = child.stdout.take();

        tracing::info!(session = %target, command = %command_line, "mosh session spawned");

        Ok(Self {
            child,
            stdin,
            stdout,
            stderr_tail,
            key_file,
            command_line,
            target,
            size: params.terminal,
            received_any: false,
            eof: false,
            closed: false,
            exit_status: None,
        })
    }

    /// Read the next chunk of session output into `buf`.
    ///
    /// Returns `0` once the session ends. If the child dies before ever
    /// producing output (i.e. the SSH bootstrap failed), returns
    /// [`MoshError::Connect`] carrying the captured stderr tail instead of a
    /// plain EOF.
    pub async fn recv(&mut self, buf: &mut [u8]) -> Result<usize> {
        if self.closed {
            return Err(MoshError::Closed);
        }
        if self.eof {
            return Ok(0);
        }
        let n = match self.stdout.as_mut() {
            Some(stdout) => stdout.read(buf).await?,
            None => {
                self.eof = true;
                return Ok(0);
            }
        };
        if n == 0 {
            self.eof = true;
            if !self.received_any {
                // Likely a bootstrap failure — give the child a beat to exit
                // so we can report the reason instead of a silent EOF.
                match tokio::time::timeout(EXIT_WAIT, self.child.wait()).await {
                    Ok(Ok(status)) => {
                        self.exit_status = Some(status);
                        if !status.success() {
                            return Err(self.bootstrap_failure(status));
                        }
                    }
                    Ok(Err(source)) => return Err(MoshError::Io(source)),
                    // Still running (closed stdout but alive): treat as EOF.
                    Err(_) => {}
                }
            } else if self.exit_status.is_none() {
                if let Ok(Some(status)) = self.child.try_wait() {
                    self.exit_status = Some(status);
                }
            }
            return Ok(0);
        }
        self.received_any = true;
        Ok(n)
    }

    /// Send keystrokes (or piped input) to the session.
    pub async fn write(&mut self, data: &[u8]) -> Result<()> {
        if self.closed {
            return Err(MoshError::Closed);
        }
        let stdin = self.stdin.as_mut().ok_or(MoshError::Closed)?;
        stdin.write_all(data).await?;
        stdin.flush().await?;
        Ok(())
    }

    /// Record a new terminal size.
    ///
    /// PORT-TODO: delivering a resize needs a PTY (a `TIOCSWINSZ` ioctl on
    /// the master fd makes `mosh-client` take a `SIGWINCH` and sync the new
    /// geometry to `mosh-server`). The pipe-mode bridge has no fd to signal,
    /// so this stores the size for the future PTY-backed spawn and logs; it
    /// is deliberately not an error so routine UI resizes stay harmless.
    pub async fn resize(&mut self, size: TerminalSize) -> Result<()> {
        self.size = size;
        tracing::debug!(
            cols = size.cols,
            rows = size.rows,
            "mosh resize recorded (no-op in pipe-mode bridge)"
        );
        Ok(())
    }

    /// Close the session: drop stdin (child sees EOF), wait briefly, kill if
    /// it lingers, and wipe the temp key file. Idempotent.
    pub async fn disconnect(&mut self) -> Result<()> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        // Dropping the stdin handle closes the pipe.
        drop(self.stdin.take());

        let mut result = Ok(());
        if self.exit_status.is_none() {
            match tokio::time::timeout(EXIT_WAIT, self.child.wait()).await {
                Ok(Ok(status)) => self.exit_status = Some(status),
                Ok(Err(source)) => result = Err(MoshError::Io(source)),
                Err(_) => {
                    if let Err(source) = self.child.kill().await {
                        result = Err(MoshError::Io(source));
                    }
                }
            }
        }
        drop(self.stdout.take());
        self.remove_key_file();
        tracing::debug!(session = %self.target, "mosh session closed");
        result
    }

    /// `user@host` this session targets.
    pub fn target(&self) -> &str {
        &self.target
    }

    /// The full `mosh …` command line this session was spawned with.
    pub fn command_line(&self) -> &str {
        &self.command_line
    }

    /// Whether the session output has reached EOF.
    pub fn is_eof(&self) -> bool {
        self.eof
    }

    /// Last recorded terminal size (see [`resize`](Self::resize)).
    pub fn size(&self) -> TerminalSize {
        self.size
    }

    /// Exit status once the child has been reaped, if known.
    pub fn exit_status(&self) -> Option<ExitStatus> {
        self.exit_status
    }

    /// Build the [`MoshError::Connect`] for a bootstrap failure, using the
    /// captured stderr tail as the reason (falling back to the exit status).
    fn bootstrap_failure(&self, status: ExitStatus) -> MoshError {
        let tail = self.stderr_tail_string();
        let reason = if tail.is_empty() {
            format!("mosh exited with {status} before the session was established")
        } else {
            tail
        };
        MoshError::connect(&self.target, reason)
    }

    /// Trailing captured stderr, lossily decoded and trimmed.
    fn stderr_tail_string(&self) -> String {
        let guard = match self.stderr_tail.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        String::from_utf8_lossy(&guard).trim().to_string()
    }

    /// Best-effort wipe of the temp key file.
    fn remove_key_file(&mut self) {
        if let Some(path) = self.key_file.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

impl Drop for MoshClient {
    fn drop(&mut self) {
        // The child is killed via `kill_on_drop`; the temp key must not
        // outlive the session even on an abnormal teardown path.
        self.remove_key_file();
    }
}

/// Build the `ssh` command string handed to mosh via `--ssh=` (mosh
/// word-splits it, so it must stay free of quoted spaces).
///
/// PORT-TODO: a materialised key path containing spaces would break mosh's
/// word-splitting; the temp-file writer avoids spaces in the path, but if a
/// user-supplied path ever lands here it needs a different mechanism.
pub fn build_ssh_command(params: &ConnectionParams, key_file: Option<&Path>) -> String {
    let mut parts: Vec<String> = vec!["ssh".to_string(), "-p".to_string(), params.port.to_string()];

    if params.connect_timeout_secs > 0 {
        parts.push("-o".to_string());
        parts.push(format!("ConnectTimeout={}", params.connect_timeout_secs));
    }
    if params.keepalive_interval_secs > 0 {
        parts.push("-o".to_string());
        parts.push(format!(
            "ServerAliveInterval={}",
            params.keepalive_interval_secs
        ));
    }
    parts.push("-o".to_string());
    parts.push(format!(
        "StrictHostKeyChecking={}",
        if params.strict_host_key_checking { "yes" } else { "no" }
    ));

    if let Some(key) = key_file {
        parts.push("-o".to_string());
        parts.push("IdentitiesOnly=yes".to_string());
        parts.push("-i".to_string());
        parts.push(key.display().to_string());
    }

    if let Some(proxy_host) = params
        .proxy_host
        .as_deref()
        .map(str::trim)
        .filter(|h| !h.is_empty())
    {
        let user = params
            .proxy_username
            .as_deref()
            .map(str::trim)
            .filter(|u| !u.is_empty())
            .map(|u| format!("{u}@"))
            .unwrap_or_default();
        let port = params
            .proxy_port
            .map(|p| format!(":{p}"))
            .unwrap_or_default();
        parts.push("-J".to_string());
        parts.push(format!("{user}{proxy_host}{port}"));
    }

    parts.join(" ")
}

/// Locate the `mosh` binary on `PATH` (which-style lookup).
///
/// Returns [`MoshError::ClientNotFound`] as `None`-equivalent for the
/// caller: `connect` maps `None` to that error.
pub fn find_mosh_binary() -> Option<PathBuf> {
    let path_env = std::env::var_os("PATH")?;
    find_in_path("mosh", &path_env)
}

/// Which-style lookup of `binary` within a `PATH`-style value.
///
/// A candidate counts only if it is an existing regular file and, on unix,
/// has any execute bit set. Empty `PATH` entries are skipped (they would
/// otherwise mean "current directory", which we deliberately do not search).
/// On Windows, `<binary>.exe` is tried before the bare name.
pub fn find_in_path(binary: &str, path_env: &OsStr) -> Option<PathBuf> {
    let candidates: Vec<String> = if cfg!(windows) {
        vec![format!("{binary}.exe"), binary.to_string()]
    } else {
        vec![binary.to_string()]
    };
    for dir in std::env::split_paths(path_env) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        for name in &candidates {
            let candidate = dir.join(name);
            if is_executable_file(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    match std::fs::metadata(path) {
        Ok(meta) => meta.is_file() && meta.permissions().mode() & 0o111 != 0,
        Err(_) => false,
    }
}

#[cfg(not(unix))]
fn is_executable_file(path: &Path) -> bool {
    std::fs::metadata(path).map(|meta| meta.is_file()).unwrap_or(false)
}

/// Write an unencrypted private key to a `0600` temp file for `-i`.
///
/// The file lives under the OS temp dir with a per-process unique name (no
/// spaces, so mosh's word-splitting of `--ssh` is safe) and is removed on
/// disconnect/drop.
fn write_temp_key(private_key: &str) -> Result<PathBuf> {
    let n = KEY_COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "termius-mosh-key-{}-{n}",
        std::process::id()
    ));
    std::fs::write(&path, private_key)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(path)
}

/// Drain the child's stderr into the shared tail buffer so bootstrap errors
/// can be reported when the session dies early.
fn spawn_stderr_drain(
    stderr: Option<tokio::process::ChildStderr>,
    tail: Arc<Mutex<Vec<u8>>>,
) {
    tokio::spawn(async move {
        let Some(mut stderr) = stderr else { return };
        let mut buf = [0u8; 1024];
        loop {
            match stderr.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let mut guard = match tail.lock() {
                        Ok(guard) => guard,
                        Err(poisoned) => poisoned.into_inner(),
                    };
                    guard.extend_from_slice(&buf[..n]);
                    if guard.len() > STDERR_TAIL_CAP {
                        let excess = guard.len() - STDERR_TAIL_CAP;
                        guard.drain(..excess);
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "termius-mosh-which-test-{}-{tag}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn make_file(dir: &Path, name: &str, executable: bool) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, b"#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = if executable { 0o755 } else { 0o644 };
            fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
        }
        #[cfg(not(unix))]
        let _ = executable;
        path
    }

    #[test]
    fn finds_executable_in_path_entry() {
        let dir = temp_dir("find");
        make_file(&dir, "mosh", true);
        let path_env = std::env::join_paths([dir.as_path()]).unwrap();

        let found = find_in_path("mosh", &path_env);
        assert_eq!(found.as_deref(), Some(dir.join("mosh").as_path()));

        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn skips_non_executable_files() {
        let dir = temp_dir("noexec");
        make_file(&dir, "mosh", false);
        let path_env = std::env::join_paths([dir.as_path()]).unwrap();

        assert_eq!(find_in_path("mosh", &path_env), None);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_binary_is_none() {
        let dir = temp_dir("missing");
        let path_env = std::env::join_paths([dir.as_path()]).unwrap();

        assert_eq!(find_in_path("mosh", &path_env), None);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn first_matching_entry_wins_and_empty_entries_skipped() {
        let dir_a = temp_dir("order-a");
        let dir_b = temp_dir("order-b");
        make_file(&dir_a, "mosh", true);
        make_file(&dir_b, "mosh", true);
        // Leading/trailing empty entries ("" paths) must be ignored.
        let path_env = std::env::join_paths([
            Path::new(""),
            dir_a.as_path(),
            Path::new(""),
            dir_b.as_path(),
            Path::new(""),
        ])
        .unwrap();

        let found = find_in_path("mosh", &path_env);
        assert_eq!(found.as_deref(), Some(dir_a.join("mosh").as_path()));

        let _ = fs::remove_dir_all(&dir_a);
        let _ = fs::remove_dir_all(&dir_b);
    }

    #[test]
    fn build_ssh_command_includes_port_options_and_proxy() {
        let params = ConnectionParams {
            host: "example.com".to_string(),
            port: 2222,
            username: "root".to_string(),
            strict_host_key_checking: true,
            connect_timeout_secs: 15,
            keepalive_interval_secs: 30,
            proxy_host: Some("jump.example.com".to_string()),
            proxy_port: Some(2200),
            proxy_username: Some("bastion".to_string()),
            ..ConnectionParams::default()
        };

        let cmd = build_ssh_command(&params, None);
        assert!(cmd.starts_with("ssh -p 2222 "), "{cmd}");
        assert!(cmd.contains("ConnectTimeout=15"), "{cmd}");
        assert!(cmd.contains("ServerAliveInterval=30"), "{cmd}");
        assert!(cmd.contains("StrictHostKeyChecking=yes"), "{cmd}");
        assert!(cmd.contains("-J bastion@jump.example.com:2200"), "{cmd}");
        assert!(!cmd.contains("IdentitiesOnly"), "{cmd}");
    }

    #[test]
    fn build_ssh_command_passes_key_file() {
        let params = ConnectionParams::default();
        let cmd = build_ssh_command(&params, Some(Path::new("/tmp/key")));
        assert!(cmd.contains("-i /tmp/key"), "{cmd}");
        assert!(cmd.contains("IdentitiesOnly=yes"), "{cmd}");
        assert!(cmd.contains("StrictHostKeyChecking=no"), "{cmd}");
    }
}
