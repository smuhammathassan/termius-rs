//! `TelnetClient`: interactive telnet sessions over raw tokio TCP.
//!
//! Mirrors the native `TelnetClient` handle the Electron app drives from its
//! telnet shell provider (`connect` / `send` / `resize` / `close` callbacks in
//! the recovered UI sources), with the IAC stream handled by
//! [`NegotiationFilter`](crate::NegotiationFilter).
//!
//! Read side yields only payload bytes (IAC sequences stripped, negotiation
//! answers written back under the hood); the write side encodes client bytes
//! (IAC doubling + NVT line-ending normalisation). Both the client itself and
//! the [`TelnetSession`] returned by [`TelnetClient::shell`] implement
//! `tokio::io::AsyncRead` + `AsyncWrite`, so `AsyncReadExt` / `AsyncWriteExt`
//! apply directly.

use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::BytesMut;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader, ReadBuf};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;

use termius_core::{ConnectionParams, TerminalSize};

use crate::error::{Result, TelnetError};
use crate::negotiation::{self, NegotiationFilter, DEFAULT_TERMINAL_TYPE};

/// Read granularity for the underlying socket. Telnet servers frequently
/// write single bytes; the `BufReader` (plus the filter's payload staging)
/// coalesces those into consumer-sized reads.
const READ_CHUNK: usize = 8 * 1024;

/// A telnet connection over raw TCP.
///
/// Create with [`TelnetClient::connect`], then obtain an interactive handle
/// via [`TelnetClient::shell`] (or read/write the client directly — both
/// implement `AsyncRead`/`AsyncWrite`).
pub struct TelnetClient {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
    filter: NegotiationFilter,
    /// Cleaned payload bytes waiting for the consumer (never contains IAC
    /// sequences).
    payload: BytesMut,
    /// Outgoing bytes staged but not yet written to the socket: negotiation
    /// replies and escaped payload fragments that hit a partial write.
    out: BytesMut,
    /// Scratch buffer for socket reads (AsyncRead side).
    scratch: [u8; READ_CHUNK],
    /// `host:port` target, for error messages and logs.
    peer: String,
    eof: bool,
    closed: bool,
}

impl TelnetClient {
    /// Connect to `params.host:params.port`, honouring
    /// `params.connect_timeout_secs` for the whole resolve+connect sequence.
    ///
    /// The username (`telnet_config.identity.username`) seeds NEW-ENVIRON
    /// `USER`; the initial window size comes from `params.terminal` (the
    /// native addon received `ptyOptions` at shell-open time — the UI should
    /// call [`TelnetClient::resize`] right after [`shell`](TelnetClient::shell)
    /// if dimensions change).
    ///
    /// PORT-TODO: `telnet_config.charset` was validated/converted in the JS
    /// layer around the native handle; `ConnectionParams` has no charset field
    /// yet, so byte transcoding must be wired at the engine-integration layer
    /// when core gains one.
    pub async fn connect(params: &ConnectionParams) -> Result<Self> {
        let host = params.host.trim();
        let port = params.port;
        if host.is_empty() {
            return Err(TelnetError::connect(
                format!(":{port}"),
                io::Error::new(io::ErrorKind::InvalidInput, "telnet host is empty"),
            ));
        }
        let target = format!("{host}:{port}");

        let connect_future = resolve_and_connect(host, port);
        let stream = if params.connect_timeout_secs > 0 {
            match tokio::time::timeout(
                Duration::from_secs(params.connect_timeout_secs),
                connect_future,
            )
            .await
            {
                Ok(result) => result,
                Err(_) => Err(TelnetError::timeout(target, params.connect_timeout_secs)),
            }
        } else {
            connect_future.await
        }?;

        if let Err(error) = stream.set_nodelay(true) {
            tracing::debug!(%error, "failed to enable TCP_NODELAY");
        }

        let mut filter = NegotiationFilter::new(
            DEFAULT_TERMINAL_TYPE,
            params.username.trim(),
            params.terminal,
        );
        let greeting = filter.startup();

        let (reader, writer) = stream.into_split();
        let mut client = Self {
            reader: BufReader::with_capacity(READ_CHUNK, reader),
            writer,
            filter,
            payload: BytesMut::new(),
            out: BytesMut::new(),
            scratch: [0; READ_CHUNK],
            peer: target,
            eof: false,
            closed: false,
        };

        // The native addon issues `IAC WILL NEW-ENVIRON` from `OnConnect`.
        if !greeting.is_empty() {
            client.writer.write_all(&greeting).await?;
        }

        tracing::info!(peer = %client.peer, "telnet session established");
        Ok(client)
    }

    /// Interactive handle over this connection: async read (payload bytes),
    /// async write (encoded keystrokes), [`resize`](TelnetClient::resize) and
    /// [`disconnect`](TelnetClient::disconnect).
    pub fn shell(&mut self) -> TelnetSession<'_> {
        TelnetSession { client: self }
    }

    /// Send `IAC SB NAWS cols rows IAC SE` for a new window size.
    ///
    /// PORT-TODO: the native `TelnetClient::Resize` gates the sub-negotiation
    /// on a per-connection "naws negotiated" flag; we always send (harmless
    /// for peers that never asked, and keeps early resizes from being lost).
    pub async fn resize(&mut self, size: TerminalSize) -> Result<()> {
        self.filter.set_size(size);
        self.flush_out().await?;
        let naws = self.filter.naws();
        self.writer.write_all(&naws).await?;
        tracing::debug!(cols = size.cols, rows = size.rows, "sent NAWS");
        Ok(())
    }

    /// Read payload bytes (IAC sequences stripped) until `buf` is filled or
    /// the peer closes. Returns `0` on EOF. Negotiation replies generated
    /// while reading are flushed to the socket first.
    pub async fn read_payload(&mut self, buf: &mut [u8]) -> Result<usize> {
        loop {
            // Negotiation replies go out before we hand payload back.
            self.flush_out().await?;
            if !self.payload.is_empty() {
                let n = buf.len().min(self.payload.len());
                buf[..n].copy_from_slice(&self.payload[..n]);
                let _ = self.payload.split_to(n);
                return Ok(n);
            }
            if self.eof {
                return Ok(0);
            }
            let n = self.reader.read(&mut self.scratch).await?;
            if n == 0 {
                self.eof = true;
                return Ok(0);
            }
            let cleaned = self.filter.feed(&self.scratch[..n]);
            if !cleaned.is_empty() {
                self.payload.extend_from_slice(&cleaned);
            }
            let replies = self.filter.take_responses();
            if !replies.is_empty() {
                self.out.extend_from_slice(&replies);
            }
        }
    }

    /// Flush staged bytes and half-close the write side; further
    /// [`resize`](TelnetClient::resize) / writes may fail. Idempotent.
    pub async fn disconnect(&mut self) -> Result<()> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        let flush = self.flush_out().await;
        let shutdown = Pin::new(&mut self.writer).shutdown().await;
        flush?;
        shutdown?;
        tracing::debug!(peer = %self.peer, "telnet session closed");
        Ok(())
    }

    /// `host:port` this client is connected to.
    pub fn peer(&self) -> &str {
        &self.peer
    }

    /// Whether the peer has closed its read side (EOF observed).
    pub fn is_eof(&self) -> bool {
        self.eof
    }

    /// Write any staged negotiation replies / escaped payload fragments.
    async fn flush_out(&mut self) -> Result<()> {
        if !self.out.is_empty() {
            let staged = std::mem::take(&mut self.out);
            self.writer.write_all(&staged).await?;
        }
        Ok(())
    }

    // --- AsyncRead core -----------------------------------------------------

    fn poll_read_inner(
        &mut self,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        loop {
            // 1. Staged replies must reach the socket before we wait for more
            //    input (they are written in response to what we just parsed).
            match self.poll_flush_out(cx) {
                Poll::Ready(Ok(())) => {}
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Pending => return Poll::Pending,
            }
            // 2. Deliver buffered payload.
            if !self.payload.is_empty() {
                let n = buf.remaining_mut().min(self.payload.len());
                buf.put_slice(&self.payload[..n]);
                let _ = self.payload.split_to(n);
                return Poll::Ready(Ok(()));
            }
            if self.eof {
                return Poll::Ready(Ok(()));
            }
            // 3. Pull more bytes from the socket and run the filter. The
            //    scratch-buffer borrow is scoped to the block so the filter
            //    can reuse `self.scratch` immediately afterwards.
            let n = {
                let mut read_buf = ReadBuf::new(&mut self.scratch);
                let poll = Pin::new(&mut self.reader).poll_read(cx, &mut read_buf);
                match poll {
                    Poll::Ready(Ok(())) => read_buf.filled().len(),
                    Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                    Poll::Pending => return Poll::Pending,
                }
            };
            if n == 0 {
                self.eof = true;
                return Poll::Ready(Ok(()));
            }
            let cleaned = self.filter.feed(&self.scratch[..n]);
            if !cleaned.is_empty() {
                self.payload.extend_from_slice(&cleaned);
            }
            let replies = self.filter.take_responses();
            if !replies.is_empty() {
                self.out.extend_from_slice(&replies);
            }
        }
    }

    // --- AsyncWrite core ----------------------------------------------------

    fn poll_flush_out(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        while !self.out.is_empty() {
            let poll = Pin::new(&mut self.writer).poll_write(cx, &self.out);
            match poll {
                Poll::Ready(Ok(0)) => {
                    // A zero-byte write on a non-empty buffer should not
                    // happen (tokio maps WouldBlock to Pending); stop without
                    // spinning — the staged bytes are retried on the next
                    // poll_read / poll_write / poll_flush.
                    break;
                }
                Poll::Ready(Ok(n)) => {
                    let take = n.min(self.out.len());
                    let _ = self.out.split_to(take);
                }
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Pending => return Poll::Pending,
            }
        }
        Poll::Ready(Ok(()))
    }

    fn poll_write_inner(&mut self, cx: &mut Context<'_>, data: &[u8]) -> Poll<io::Result<usize>> {
        match self.poll_flush_out(cx) {
            Poll::Ready(Ok(())) => {}
            Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
            Poll::Pending => return Poll::Pending,
        }
        // Fast path: no IAC and no CR/LF, so the wire bytes equal the input.
        let needs_encoding = data.contains(&negotiation::IAC)
            || data.contains(&negotiation::CR)
            || data.contains(&negotiation::LF);
        if !needs_encoding {
            return Pin::new(&mut self.writer).poll_write(cx, data);
        }
        // Encode (IAC doubling + NVT line endings, like the native Send)
        // and stage it; partial writes are picked up by the next
        // poll_write / poll_flush / poll_read.
        let encoded = negotiation::encode(data);
        self.out.extend_from_slice(&encoded);
        match self.poll_flush_out(cx) {
            Poll::Ready(Ok(())) | Poll::Pending => Poll::Ready(Ok(data.len())),
            Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
        }
    }

    fn poll_close_inner(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.poll_flush_out(cx) {
            Poll::Ready(Ok(())) => Pin::new(&mut self.writer).poll_shutdown(cx),
            other => other,
        }
    }
}

impl AsyncRead for TelnetClient {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        self.get_mut().poll_read_inner(cx, buf)
    }
}

impl AsyncWrite for TelnetClient {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.get_mut().poll_write_inner(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.get_mut().poll_flush_out(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.get_mut().poll_close_inner(cx)
    }
}

/// Interactive session handle returned by [`TelnetClient::shell`]. Borrows the
/// client for its lifetime (drop the session before reusing the client).
///
/// Exposes the async read side (cleaned payload bytes) and write side plus
/// `resize` / `disconnect`, mirroring the JS handle's
/// `sendData` / `sendSize` / `close` surface.
pub struct TelnetSession<'a> {
    client: &'a mut TelnetClient,
}

impl<'a> TelnetSession<'a> {
    /// See [`TelnetClient::read_payload`].
    pub async fn read_payload(&mut self, buf: &mut [u8]) -> Result<usize> {
        self.client.read_payload(buf).await
    }

    /// See [`TelnetClient::resize`] (sends NAWS).
    pub async fn resize(&mut self, size: TerminalSize) -> Result<()> {
        self.client.resize(size).await
    }

    /// See [`TelnetClient::peer`].
    pub fn peer(&self) -> &str {
        self.client.peer()
    }

    /// Flush, half-close and release the borrowed client.
    pub async fn disconnect(self) -> Result<()> {
        self.client.disconnect().await
    }
}

impl AsyncRead for TelnetSession<'_> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let session = self.get_mut();
        Pin::new(&mut *session.client).poll_read(cx, buf)
    }
}

impl AsyncWrite for TelnetSession<'_> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let session = self.get_mut();
        Pin::new(&mut *session.client).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let session = self.get_mut();
        Pin::new(&mut *session.client).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let session = self.get_mut();
        Pin::new(&mut *session.client).poll_shutdown(cx)
    }
}

/// Resolve the host, then try each address in order (IPv4/IPv6 fallback),
/// preserving which stage failed so errors keep their reason.
async fn resolve_and_connect(host: &str, port: u16) -> Result<TcpStream> {
    let resolved = tokio::net::lookup_host((host, port))
        .await
        .map_err(|source| TelnetError::dns(host, source))?;
    let addresses: Vec<SocketAddr> = resolved.collect();
    if addresses.is_empty() {
        return Err(TelnetError::dns(
            host,
            io::Error::new(io::ErrorKind::NotFound, "host resolved to no addresses"),
        ));
    }
    let primary = if addresses.len() == 1 {
        addresses[0].to_string()
    } else {
        format!("{host}:{port}")
    };
    let mut last_error: Option<io::Error> = None;
    for address in &addresses {
        match TcpStream::connect(address).await {
            Ok(stream) => return Ok(stream),
            Err(error) => {
                tracing::debug!(%address, %error, "telnet connect attempt failed");
                last_error = Some(error);
            }
        }
    }
    Err(TelnetError::connect(
        primary,
        last_error.unwrap_or_else(|| {
            io::Error::new(io::ErrorKind::ConnectionRefused, "no addresses to connect")
        }),
    ))
}
