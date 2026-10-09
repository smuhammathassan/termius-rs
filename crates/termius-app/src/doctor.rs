//! `termius doctor` — a headless diagnostics report: which engines, paths
//! and external helpers this installation can actually use.
//!
//! Deliberately small: every line answers "is this thing available here?"
//! the way a support engineer would ask before debugging a connection.

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::single_instance::{self, InstanceLock};

/// Print the report. Only the plumbing failures (none expected — `main` has
/// already resolved the paths) return `Err`.
pub fn run(data_dir: &Path, db_path: &Path, log_file: Option<&Path>) -> Result<()> {
    println!("Termius doctor {}", env!("CARGO_PKG_VERSION"));
    println!();

    row("data dir", &data_dir.display().to_string());
    row("database", &database_line(db_path));
    row("log file", &optional_path(log_file));
    row("instance lock", &lock_line(data_dir));
    row("ssh agent", &ssh_agent_line());
    row("mosh", &mosh_line());
    row("serial ports", &serial_line());
    row("sync endpoint", termius_sync::DEFAULT_BASE_URL);
    row(
        "engines",
        "ssh: in-process (russh) · telnet: in-process (tokio TCP) \
         · serial: in-process (serialport) · mosh: external binary (deferred) \
         · sync: in-process (reqwest) · terminal: in-process (alacritty)",
    );

    Ok(())
}

fn row(label: &str, value: &str) {
    println!("{label:<14} {value}");
}

/// Open the database and report its path + schema version (or the failure).
fn database_line(db_path: &Path) -> String {
    match termius_storage::Store::open(db_path) {
        Ok(store) => match store.with_connection(termius_storage::schema_version) {
            Ok(version) => format!(
                "{} (schema v{version}, latest v{})",
                db_path.display(),
                termius_storage::LATEST_VERSION
            ),
            Err(err) => format!("{} (opened, schema unreadable: {err})", db_path.display()),
        },
        Err(err) => format!("{} (FAILED to open: {err})", db_path.display()),
    }
}

fn optional_path(path: Option<&Path>) -> String {
    match path {
        Some(path) => path.display().to_string(),
        None => "(unavailable — logging to stderr only)".to_owned(),
    }
}

/// Is another instance holding the lock right now?
fn lock_line(data_dir: &Path) -> String {
    let path = data_dir.join(single_instance::LOCK_FILE_NAME);
    match InstanceLock::holder_pid(data_dir) {
        Some(pid) => format!("{} (held by pid {pid})", path.display()),
        None if path.exists() => format!(
            "{} (present but unreadable — a live instance will take it over)",
            path.display()
        ),
        None => format!("{} (free)", path.display()),
    }
}

/// SSH agent availability via the standard env vars (agent auth depends on it).
fn ssh_agent_line() -> String {
    match std::env::var_os("SSH_AUTH_SOCK") {
        Some(sock) => {
            let sock = PathBuf::from(sock);
            let state = if sock.exists() {
                "socket present"
            } else {
                "socket missing"
            };
            format!("SSH_AUTH_SOCK={} ({state})", sock.display())
        }
        None => "SSH_AUTH_SOCK not set — agent authentication unavailable".to_owned(),
    }
}

/// Mosh needs external binaries until the `termius-mosh` crate lands.
fn mosh_line() -> String {
    match (find_in_path("mosh"), find_in_path("mosh-client")) {
        (Some(mosh), Some(client)) => format!("{} (+ {})", mosh.display(), client.display()),
        (Some(mosh), None) => format!("{} (mosh-client missing)", mosh.display()),
        (None, _) => "not found in PATH — mosh sessions unavailable".to_owned(),
    }
}

/// Enumerate serial ports (blocking udev/IOKit probe — fine in a CLI).
fn serial_line() -> String {
    match termius_serial::list_ports() {
        Ok(ports) if ports.is_empty() => "none found".to_owned(),
        Ok(ports) => {
            let names: Vec<String> = ports.iter().take(8).map(|port| port.path.clone()).collect();
            let extra = ports.len().saturating_sub(names.len());
            if extra > 0 {
                format!("{} (+{extra} more)", names.join(", "))
            } else {
                names.join(", ")
            }
        }
        Err(err) => format!("enumeration failed: {err}"),
    }
}

fn find_in_path(binary: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(binary))
        .find(|candidate| candidate.is_file())
}
