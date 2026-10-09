//! shell — see crate docs.
//!
//! The interactive PTY shell channel. Replaces `@termius/node-pty` +
//! `libtermius`'s `write`/`read`/`resize(rows, cols)` surface: the terminal
//! emulator (`termius-terminal`) writes keystrokes into [`ChannelHandle`] and
//! reads the remote output back as a [`futures::Stream`] of byte chunks.

use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures::Stream;
use termius_core::TerminalSize;
use tokio::sync::mpsc;

use crate::error::{Result, SshError};

/// `$TERM` advertised in the pty request — matches the term the Electron app
/// sent through `libtermius`.
pub const DEFAULT_TERM: &str = "xterm-256color";

/// Convert the domain terminal size into SSH `pty-req` dimensions:
/// `(cols, rows, pixel_width, pixel_height)`.
///
/// The tuple is deliberately typed `(u16, u16, u32, u32)`: the source model
/// stores cols/rows as `u16` and pixels as `Option<u32>`, and call sites
/// widen via `.into()` so this keeps compiling whether `russh` takes `u16`
/// or `u32` wire-style arguments (PORT-TODO: pin to the russh 0.64 signature).
pub fn pty_args(size: &TerminalSize) -> (u16, u16, u32, u32) {
    (
        size.cols,
        size.rows,
        size.width_px.unwrap_or(0),
        size.height_px.unwrap_or(0),
    )
}

/// Commands the terminal side can push into the channel pump task.
#[derive(Debug, Clone)]
pub(crate) enum ShellCommand {
    Resize(TerminalSize),
    Eof,
}

/// Report delivered when the remote shell terminates.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExitEvent {
    /// `exit-status` from the remote shell, when it sent one.
    pub exit_status: Option<u32>,
    /// Signal name when the shell was killed (`SIGKILL`, …).
    pub signal: Option<String>,
}

/// Handle to an interactive PTY shell channel.
///
/// The underlying `russh` channel lives in a pump task (spawned by
/// [`crate::client::SshClient::shell`]); this handle is the app-facing bridge:
///
/// * **write side** — [`write`](Self::write) pushes keystrokes towards the
///   remote shell (bounded, so a stalled server applies backpressure),
/// * **read side** — [`recv`](Self::recv) *or* the [`Stream`] impl yield the
///   remote output as [`Bytes`] chunks for the terminal to render,
/// * **control** — [`resize`](Self::resize) sends a `window-change` request,
///   [`eof`](Self::eof) signals end-of-input, [`exit_event`](Self::exit_event)
///   resolves when the shell terminates.
pub struct ChannelHandle {
    tx_input: mpsc::Sender<Bytes>,
    tx_command: mpsc::Sender<ShellCommand>,
    rx_output: mpsc::Receiver<Bytes>,
    rx_exit: mpsc::Receiver<ExitEvent>,
}

impl ChannelHandle {
    pub(crate) fn from_parts(
        tx_input: mpsc::Sender<Bytes>,
        tx_command: mpsc::Sender<ShellCommand>,
        rx_output: mpsc::Receiver<Bytes>,
        rx_exit: mpsc::Receiver<ExitEvent>,
    ) -> Self {
        Self { tx_input, tx_command, rx_output, rx_exit }
    }

    /// Send terminal input (keystrokes) to the remote shell.
    pub async fn write(&self, data: &[u8]) -> Result<()> {
        self.tx_input
            .send(Bytes::copy_from_slice(data))
            .await
            .map_err(|_| SshError::channel_closed("shell channel is closed"))
    }

    /// Ask the remote pty to re-match the given terminal dimensions
    /// (SSH `window-change`). This is `resize(rows, cols)` from the JS API —
    /// the caller passes the full [`TerminalSize`] so pixel dimensions ride
    /// along when known.
    pub async fn resize(&self, size: TerminalSize) -> Result<()> {
        self.tx_command
            .send(ShellCommand::Resize(size))
            .await
            .map_err(|_| SshError::channel_closed("shell channel is closed"))
    }

    /// Signal end-of-input (SSH `EOF`); the shell keeps running until it
    /// exits on its own.
    pub async fn eof(&self) -> Result<()> {
        self.tx_command
            .send(ShellCommand::Eof)
            .await
            .map_err(|_| SshError::channel_closed("shell channel is closed"))
    }

    /// Read the next chunk of remote output. `None` once the shell's output
    /// side has closed.
    pub async fn recv(&mut self) -> Option<Bytes> {
        self.rx_output.recv().await
    }

    /// Resolve when the remote shell exits (dropped output is reported as a
    /// default event rather than panicking).
    pub async fn exit_event(&mut self) -> ExitEvent {
        self.rx_exit.recv().await.unwrap_or_default()
    }
}

/// The read side is also a `Stream`, so the terminal layer can poll it with
/// any combinator (`futures::StreamExt`, `try_stream`, …).
impl Stream for ChannelHandle {
    type Item = Bytes;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // ChannelHandle is Unpin (all fields are Unpin channel halves).
        let this = self.get_mut();
        this.rx_output.poll_recv(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pty_args_widen_the_domain_size() {
        let size = TerminalSize { cols: 132, rows: 43, width_px: Some(1280), height_px: Some(800) };
        assert_eq!(pty_args(&size), (132, 43, 1280, 800));
    }

    #[test]
    fn pty_args_default_pixels_are_zero() {
        let size = TerminalSize::default();
        assert_eq!(pty_args(&size), (80, 24, 0, 0));
    }

    #[tokio::test]
    async fn handle_roundtrips_bytes_and_exit() {
        let (tx_input, mut rx_input) = mpsc::channel::<Bytes>(4);
        let (tx_command, mut rx_command) = mpsc::channel::<ShellCommand>(4);
        let (tx_output, rx_output) = mpsc::channel::<Bytes>(4);
        let (tx_exit, rx_exit) = mpsc::channel::<ExitEvent>(1);
        let mut handle =
            ChannelHandle::from_parts(tx_input, tx_command, rx_output, rx_exit);

        handle.write(b"\x03").await.expect("write");
        assert_eq!(rx_input.recv().await.expect("input"), Bytes::from_static(b"\x03"));

        handle.resize(TerminalSize { cols: 100, rows: 50, width_px: None, height_px: None })
            .await
            .expect("resize");
        match rx_command.recv().await.expect("command") {
            ShellCommand::Resize(size) => {
                assert_eq!((size.cols, size.rows), (100, 50));
            }
            other => panic!("unexpected command: {other:?}"),
        }

        tx_output.send(Bytes::from_static(b"prompt$ ")).await.expect("output");
        assert_eq!(handle.recv().await.expect("output"), Bytes::from_static(b"prompt$ "));

        tx_exit.send(ExitEvent { exit_status: Some(0), signal: None }).await.expect("exit");
        assert_eq!(handle.exit_event().await.exit_status, Some(0));
    }

    #[tokio::test]
    async fn stream_impl_yields_output() {
        use futures::StreamExt;

        let (tx_input, _rx_input) = mpsc::channel::<Bytes>(1);
        let (tx_command, _rx_command) = mpsc::channel::<ShellCommand>(1);
        let (tx_output, rx_output) = mpsc::channel::<Bytes>(1);
        let (_tx_exit, rx_exit) = mpsc::channel::<ExitEvent>(1);
        let mut handle = ChannelHandle::from_parts(tx_input, tx_command, rx_output, rx_exit);

        tx_output.send(Bytes::from_static(b"ok")).await.expect("send");
        let next = handle.next().await;
        assert_eq!(next, Some(Bytes::from_static(b"ok")));
    }
}
