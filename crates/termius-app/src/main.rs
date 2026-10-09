//! The Termius desktop binary: wires engines + storage + GPUI UI, owns
//! window/menu/dock lifecycle, terminal session multiplexing (replaces the
//! Electron main+background+renderer IPC split), auto-update, crash
//! reporting, and the single-instance lock.
//!
//! # Startup order (GUI mode)
//!
//! 1. **CLI** — `clap` parses the command surface; `--version` / `--help`
//!    exit here, `termius doctor` diverts to the diagnostics report.
//! 2. **Data dir** — resolved through [`termius_storage::default_db_path`]
//!    (`~/Library/Application Support/Termius` on macOS).
//! 3. **Logging** — a `tracing` subscriber (`RUST_LOG`, default `info`)
//!    tee-ing to stderr and `<data dir>/termius-app.log`.
//! 4. **Tokio runtime** — one multi-thread runtime, *entered* on this thread
//!    so `Handle::try_current()` resolves inside GPUI's main-thread dispatch;
//!    `termius-ui`'s `engine_runtime()` then reuses it (one reactor for every
//!    session instead of two).
//! 5. **Single-instance lock** — PID-stamped file in the data dir; a second
//!    launch reports and exits gracefully instead of double-starting.
//! 6. **Store** — `Store::open_default()` health check; failure aborts with a
//!    clear error. The UI opens its own connection afterwards (WAL-safe).
//! 7. **Crash handler + update check** — callable stubs (`crash`, `update`).
//! 8. **GPUI** — `termius_ui::init(cx)` + `open_window(cx)` inside
//!    `gpui::Application::new().run(...)`.
//! 9. **Shutdown** — when the run loop returns (⌘Q / app termination): abort
//!    the ctrl-c task, shut the runtime down (cancelling engine tasks) and
//!    release the single-instance lock.

mod cli;
mod crash;
mod doctor;
mod logging;
mod single_instance;
mod update;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context as _, Result};
use clap::Parser as _;

use crate::cli::{Cli, Command};
use crate::single_instance::InstanceLock;

fn main() -> Result<()> {
    // 1. CLI — `Cli::parse()` prints help/version and exits on its own.
    let cli = Cli::parse();

    // 2. Data directory (shared by the log file, the instance lock and the DB).
    let db_path = termius_storage::default_db_path()
        .context("could not resolve the per-platform application data directory")?;
    let data_dir = parent_of(&db_path)?;

    // 3. Logging — stderr always, `<data dir>/termius-app.log` when writable.
    let log_file = logging::init(&data_dir);
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        data_dir = %data_dir.display(),
        "termius starting"
    );

    match cli.command {
        Some(Command::Doctor) => doctor::run(&data_dir, &db_path, log_file.as_deref()),
        None => run_gui(data_dir, db_path),
    }
}

/// The GUI path: runtime → lock → store → stubs → GPUI → shutdown.
fn run_gui(data_dir: PathBuf, db_path: PathBuf) -> Result<()> {
    // 4. One multi-thread runtime for the whole process (engine futures, the
    //    update check, the ctrl-c listener).
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to build the tokio runtime")?;

    // Entering the runtime on this thread makes `Handle::try_current()`
    // succeed wherever GPUI dispatches on the main thread, so
    // `termius-ui::app_state::engine_runtime()` reuses *this* handle and all
    // sessions share a single IO reactor.
    // PORT-TODO: if a future gpui ever dispatches view updates off the main
    // thread, the UI falls back to its private runtime — still correct, but
    // two reactors; long-term the handle should be injected into termius-ui.
    let entered = runtime.enter();

    // 5. Single instance — the second launch exits here, cleanly.
    let lock = match InstanceLock::acquire(&data_dir)? {
        Some(lock) => Arc::new(lock),
        None => {
            tracing::info!("another instance is already running; exiting");
            eprintln!(
                "Termius is already running — focusing it instead of starting a second copy."
            );
            // PORT-TODO: actually focus the existing window (macOS
            // `NSRunningApplication::activate`, or a unix-socket ping that
            // asks termius-ui to raise its window).
            return Ok(());
        }
    };

    // 6. Storage — fail fast with a clear message if the DB cannot open
    //    (corrupt file, permissions, disk full). No dialog API is reachable
    //    before the GPUI run loop exists, so the error goes to the log and
    //    stderr.
    // PORT-TODO: show a native error dialog (gpui prompt/agent) instead of
    //    bailing to stderr when running from Finder.
    let store = termius_storage::Store::open_default().with_context(|| {
        format!(
            "failed to open the local database at {} (permissions / disk / corruption?)",
            db_path.display()
        )
    })?;
    match store.with_connection(termius_storage::schema_version) {
        Ok(version) => tracing::info!(
            schema = version,
            latest = termius_storage::LATEST_VERSION,
            path = %db_path.display(),
            "local database ready"
        ),
        Err(err) => {
            tracing::warn!(error = %err, "database opened but its schema version is unreadable")
        }
    }
    // `store` stays open for the whole run. The UI opens a *second*
    // connection (`TermiusState` → `load_library()` → `Store::open_default()`),
    // which is safe under WAL + busy timeout — but a single shared open would
    // be cleaner.
    // PORT-TODO: inject `Arc<Store>` into `termius-ui` (its `init`/`launch`
    // take no store today) so both halves share one handle.

    // 7. Crash handling + update check (documented stubs).
    crash::install_crash_handler().context("failed to install the crash handler")?;
    update::spawn_check_for_updates(runtime.handle());

    // Ctrl-C: release the lock, then exit — the UI owns sessions, so there is
    // no in-process disconnect broadcast yet.
    let ctrl_lock = Arc::clone(&lock);
    let ctrl_task = runtime.spawn(async move {
        match tokio::signal::ctrl_c().await {
            Ok(()) => {
                tracing::warn!("ctrl-c received; releasing the instance lock and exiting");
                // PORT-TODO: broadcast `SessionCommand::Disconnect` to every
                // open session (needs a shutdown hook exported by
                // termius-ui) before exiting; sockets die with the process
                // today, which the servers accept, but no exit-status is
                // reported to the UI.
                ctrl_lock.release();
                std::process::exit(0);
            }
            Err(err) => tracing::error!(error = %err, "could not install the ctrl-c handler"),
        }
    });

    // 8. GPUI — same shape as `termius_ui::launch()`, but wrapped in the
    //    lifecycle above.
    gpui::Application::new().run(|cx| {
        termius_ui::init(cx);
        if let Err(err) = termius_ui::open_window(cx) {
            tracing::error!(error = %err, "failed to open the main window");
            eprintln!("Termius failed to open its main window: {err}");
        }
    });

    // 9. The run loop returned (window close on platforms that quit on last
    //    window, ⌘Q, or app termination) → orderly shutdown.
    drop(entered);
    shutdown(runtime, lock, ctrl_task, &db_path);
    Ok(())
}

/// Orderly shutdown: stop waiting for ctrl-c, cancel engine tasks (sessions),
/// then release the single-instance lock so the next launch is clean.
fn shutdown(
    runtime: tokio::runtime::Runtime,
    lock: Arc<InstanceLock>,
    ctrl_task: tokio::task::JoinHandle<()>,
    db_path: &Path,
) {
    tracing::info!(db = %db_path.display(), "GPUI run loop exited; shutting down");
    ctrl_task.abort();
    // PORT-TODO: before this, drain sessions — send `Disconnect` to the UI's
    // bridge tasks and await a bounded grace period so servers see a clean
    // channel close instead of task cancellation.
    runtime.shutdown_timeout(Duration::from_secs(3));
    lock.release();
    tracing::info!("shutdown complete");
}

/// The data directory is the parent of the database path.
fn parent_of(db_path: &Path) -> Result<PathBuf> {
    match db_path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => Ok(parent.to_path_buf()),
        _ => Err(anyhow!(
            "database path {} has no parent directory",
            db_path.display()
        )),
    }
}
