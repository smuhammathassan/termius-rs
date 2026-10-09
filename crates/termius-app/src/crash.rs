//! Crash-reporting stub — the seam where minidump capture lives.
//!
//! # What the Electron app did
//!
//! Termius shipped native addons (`minidump-parser`, crash-report plumbing in
//! the main process) to collect and upload crash dumps.
//!
//! # PORT-TODO — the real implementation
//!
//! * Capture a **minidump** on SIGSEGV/SIGBUS/SIGILL/SIGFPE/SIGABRT via a
//!   breakpad-style handler (`breakpad-sys` / `minidump-writer`), plus a
//!   backtrace for panics (`std::backtrace::Backtrace` is enough there).
//! * Persist dumps under `<data dir>/crashes/` with a manifest (app version,
//!   OS, arch) and upload them to the crash endpoint on next launch — only
//!   with user consent (the Electron app asked first; mirror that dialog).
//! * The signal handlers need a small `libc`/`nix` dependency (not in the
//!   current dep set) — coordinator to add when this is implemented.
//!
//! Today [`install_crash_handler`] installs a panic hook that logs panics
//! with location + message through `tracing` and then chains to the previous
//! hook, so crashes are at least visible in `termius-app.log`.

use anyhow::Result;

/// Install process-level crash handling.
///
/// No-op beyond the panic hook (see module docs for the minidump plan);
/// returns `Result` so the future dump-writer can fail visibly without
/// changing call sites.
pub fn install_crash_handler() -> Result<()> {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Structured fields instead of `Display` on the hook info — keeps the
        // log line machine-readable and avoids relying on formatting impls.
        let location = info
            .location()
            .map(|loc| loc.to_string())
            .unwrap_or_else(|| "<unknown location>".to_owned());
        let payload = info.payload();
        let message = payload
            .downcast_ref::<&str>()
            .map(|msg| (*msg).to_owned())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "<non-string panic payload>".to_owned());
        tracing::error!(location = %location, message = %message, "panic captured by the crash handler");
        // PORT-TODO: serialize + persist a minidump here.
        previous(info);
    }));
    tracing::info!("crash handler installed (panic hook only; minidump capture is PORT-TODO)");
    Ok(())
}
