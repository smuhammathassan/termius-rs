//! Logging: a global `tracing` subscriber filtered by `RUST_LOG` (default
//! `info`), tee-ing every event to **stderr** and — when the data directory
//! is writable — an appending **`termius-app.log`** next to the database.
//!
//! Replaces the Electron main-process `electron-log` file logging. The log
//! file is best-effort: if it cannot be opened or written, logging degrades
//! to stderr-only instead of failing startup.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::EnvFilter;

/// Filter applied when `RUST_LOG` is unset or fails to parse.
const DEFAULT_FILTER: &str = "info";

/// Log file name inside the data directory.
pub const LOG_FILE_NAME: &str = "termius-app.log";

/// Install the global `tracing` subscriber.
///
/// Returns the log file path when one was opened (`None` ⇒ stderr only).
/// Called once, before anything else that logs; never panics — a failed
/// install is reported on stderr instead.
pub fn init(data_dir: &Path) -> Option<PathBuf> {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));
    let (sink, log_path) = TeeSink::open(data_dir);

    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(sink)
        // Plain text: the file must stay free of ANSI escapes.
        .with_ansi(false)
        .with_target(true)
        .finish();

    if let Err(err) = tracing::subscriber::set_global_default(subscriber) {
        eprintln!("termius: could not install the tracing subscriber: {err}");
    }
    log_path
}

/// Writes each event to stderr and, when available, the log file.
struct TeeSink {
    file: Option<Arc<Mutex<File>>>,
}

impl TeeSink {
    /// Open `<data dir>/termius-app.log` for appending; on any failure the
    /// sink falls back to stderr-only (reported on stderr, since tracing is
    /// not installed yet).
    fn open(data_dir: &Path) -> (Self, Option<PathBuf>) {
        let path = data_dir.join(LOG_FILE_NAME);
        if let Err(err) = std::fs::create_dir_all(data_dir) {
            eprintln!(
                "termius: cannot create the data directory {}: {err}",
                data_dir.display()
            );
            return (Self { file: None }, None);
        }
        match OpenOptions::new().create(true).append(true).open(&path) {
            Ok(file) => (
                Self {
                    file: Some(Arc::new(Mutex::new(file))),
                },
                Some(path),
            ),
            Err(err) => {
                eprintln!(
                    "termius: cannot open the log file {}: {err}",
                    path.display()
                );
                (Self { file: None }, None)
            }
        }
    }
}

impl<'a> MakeWriter<'a> for TeeSink {
    type Writer = TeeWriter<'a>;

    fn make_writer(&'a self) -> Self::Writer {
        TeeWriter {
            file: self.file.as_ref(),
            buf: Vec::new(),
        }
    }
}

/// Buffers one event's bytes and flushes them to stderr + the log file as a
/// single unit (on `flush`/drop), so concurrent events never interleave
/// mid-line.
struct TeeWriter<'a> {
    file: Option<&'a Arc<Mutex<File>>>,
    buf: Vec<u8>,
}

impl TeeWriter<'_> {
    /// Emit the buffered event: stderr first (best effort), then the file
    /// under its mutex (best effort — stderr already carries the line).
    fn emit(&mut self) -> io::Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }
        // stderr may be closed when launched from Finder — ignore that.
        let _ = io::stderr().write_all(&self.buf);
        if let Some(file) = self.file {
            let result = match file.lock() {
                Ok(mut guard) => write_all_flush(&mut *guard, &self.buf),
                Err(poisoned) => write_all_flush(&mut *poisoned.into_inner(), &self.buf),
            };
            if let Err(err) = result {
                // Degrade to console-only logging rather than losing events.
                let _ = writeln!(io::stderr(), "termius: log file write failed: {err}");
            }
        }
        self.buf.clear();
        Ok(())
    }
}

impl Write for TeeWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.emit()
    }
}

impl Drop for TeeWriter<'_> {
    fn drop(&mut self) {
        // The fmt layer may drop the writer without flushing; emit here so
        // no event is lost.
        let _ = self.emit();
    }
}

fn write_all_flush(file: &mut File, buf: &[u8]) -> io::Result<()> {
    file.write_all(buf)?;
    file.flush()
}
