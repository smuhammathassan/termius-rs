//! Auto-update stub — the seam where the Sparkle/Squirrel-equivalent lives.
//!
//! # What the Electron app did
//!
//! Termius (Squirrel.mac) exposed `check-for-update` / `update-restart` IPC
//! channels from the main-process menu; the renderer asked, the main process
//! talked to `autoUpdater`.
//!
//! # PORT-TODO — the real implementation
//!
//! * Poll a signed `releases.json` manifest (feed URL from settings) with
//!   `reqwest` (already in the workspace via `termius-sync`).
//! * Download + verify a signature (ed25519/minisign-style) before touching
//!   anything on disk; stage into the data dir.
//! * Install + relaunch: macOS needs the app-bundle dance Sparkle does
//!   (quarantine xattr, `ditto`, `NSWorkspace` relaunch); a bare binary can
//!   replace itself via `rename` + re-exec.
//! * Surface results to the UI (menu item → status bar), mirroring
//!   `check-for-update` / `update-restart`, and gate on a `--no-update` flag.
//!
//! Until then [`check_for_updates`] is a callable no-op that logs its intent.

use anyhow::Result;

/// Check the update feed for a newer version.
///
/// Never fails today: the check is a no-op until the feed/verification
/// pipeline above lands.
pub async fn check_for_updates() -> Result<()> {
    tracing::info!("update check requested");
    // PORT-TODO: real feed poll (see module docs); no network I/O happens here.
    tracing::warn!("auto-update is not implemented yet — skipping the update check");
    Ok(())
}

/// Fire the (stubbed) check on the shared runtime without blocking startup.
pub fn spawn_check_for_updates(runtime: &tokio::runtime::Handle) {
    let _task = runtime.spawn(async {
        if let Err(err) = check_for_updates().await {
            tracing::error!(error = %err, "update check failed");
        }
    });
    // `_task` is dropped here: dropping a JoinHandle detaches the task, it
    // keeps running on the shared runtime.
}
