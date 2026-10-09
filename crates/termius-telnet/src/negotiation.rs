//! Telnet protocol filter: IAC byte handling, an option-negotiation state
//! machine, and sub-negotiation (TERMINAL-TYPE / NAWS / NEW-ENVIRON) answers.
//!
//! # Provenance
//!
//! The recovered background JS never touches IAC bytes itself: the Electron
//! app drives a `TelnetClient` handle exported by the native addon
//! (`@termius/libtermius`, whose Rust core vendors `core/telnet/libtelnet.c`,
//! an RFC 1143 Q-method engine — symbol strings recovered from `termius.node`:
//! `_telnet_negotiate`, `_telnet_ttype_send/is`, `_telnet_begin_newenviron`,
//! `unexpected byte after IAC inside SB`, `DONT answered by WILL`,
//! `WONT answered by DO`). This module reproduces the observable wire
//! behaviour of that transport:
//!
//! * server `IAC WILL ECHO` / `IAC WILL SUPPRESS-GO-AHEAD` → `IAC DO …`
//!   (server-side echo / SGA, per the Termius spec); any other server
//!   `IAC WILL <option>` → `IAC DONT …`
//! * `IAC DO <option>` for options the terminal does not perform → `IAC WONT …`
//!   (and `IAC WILL` for TERMINAL-TYPE / NAWS / NEW-ENVIRON, which we do)
//! * `IAC SB TERMINAL-TYPE SEND IAC SE` → `IAC SB TERMINAL-TYPE IS xterm IAC SE`
//!   (the native addon answers with the literal `xterm` string)
//! * `IAC DO NAWS` → `IAC WILL NAWS` + a 4-byte big-endian window-size
//!   sub-negotiation, matching `TelnetClientImpl::Resize`/`SendInitialSize`
//! * `IAC SB NEW-ENVIRON SEND …` → `TERM`/`USER` values (the addon sends
//!   `IAC WILL NEW-ENVIRON` from `OnConnect` and answers with env values)
//! * `IAC IAC` in the stream → a single `0xFF` payload byte
//! * everything else (BREAK, NOOP, GA, … and unsupported options) is stripped
//!   so consumers only ever see payload bytes
//!
//! The parser is a pure byte state machine: it may be fed one byte at a time
//! or in arbitrary chunks and produces identical output.

use bytes::{BufMut, BytesMut};

use termius_core::TerminalSize;

// --- Telnet protocol bytes (RFC 854) --------------------------------------

/// Interpret As Command. Doubled (`IAC IAC`) to carry a literal 0xFF payload.
pub const IAC: u8 = 255;
pub const DONT: u8 = 254;
pub const DO: u8 = 253;
pub const WONT: u8 = 252;
pub const WILL: u8 = 251;
pub const SB: u8 = 250;
pub const GA: u8 = 249;
pub const EL: u8 = 248;
pub const EC: u8 = 247;
pub const AYT: u8 = 246;
pub const AO: u8 = 245;
pub const IP: u8 = 244;
pub const BREAK: u8 = 243;
pub const DM: u8 = 242;
pub const NOOP: u8 = 241;
pub const SE: u8 = 240;

/// Carriage return — the NVT line-ending rules below apply to it.
pub const CR: u8 = 0x0D;
/// Line feed — the NVT line-ending rules below apply to it.
pub const LF: u8 = 0x0A;

// --- Telnet options --------------------------------------------------------

pub const OPT_BINARY: u8 = 0;
pub const OPT_ECHO: u8 = 1;
pub const OPT_SGA: u8 = 3;
pub const OPT_TTYPE: u8 = 24;
pub const OPT_NAWS: u8 = 31;
pub const OPT_NEW_ENVIRON: u8 = 39;

/// Terminal type reported for TERMINAL-TYPE queries. The Termius native addon
/// carries a single `xterm` string for this purpose; the renderer runs
/// xterm.js, so `xterm` is what the remote end is told.
pub const DEFAULT_TERMINAL_TYPE: &str = "xterm";

// Sub-negotiation commands (RFC 1091 TERMINAL-TYPE, RFC 1572 NEW-ENVIRON).
const TTYPE_IS: u8 = 0;
const TTYPE_SEND: u8 = 1;
const ENV_IS: u8 = 0;
const ENV_SEND: u8 = 1;
const ENV_VAR: u8 = 0;
const ENV_VALUE: u8 = 1;
const ENV_ESC: u8 = 2;
const ENV_USERVAR: u8 = 3;

/// Hard cap on sub-negotiation payload so a hostile or broken peer cannot
/// grow the parser buffer without bound by never sending `IAC SE`.
const MAX_SUBNEGOTIATION: usize = 8 * 1024;

/// Parser state. Kept in the struct (not in `feed`'s stack) so negotiation
/// sequences may straddle read chunks or arrive byte-at-a-time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Neutral: bytes pass through as payload.
    Data,
    /// Saw `IAC`; next byte selects a command.
    Iac,
    /// Saw `IAC WILL`; next byte is the option.
    Will,
    /// Saw `IAC WONT`; next byte is the option.
    Wont,
    /// Saw `IAC DO`; next byte is the option.
    Do,
    /// Saw `IAC DONT`; next byte is the option.
    Dont,
    /// Inside `IAC SB …`; bytes accumulate until `IAC SE`.
    Subnegotiation,
    /// Saw `IAC` inside a sub-negotiation.
    SubnegotiationIac,
}

/// Telnet protocol filter: strips IAC sequences from the server→client stream,
/// answers option negotiations, and encodes client→server bytes.
///
/// One instance lives per connection; it is not shared between tasks.
#[derive(Debug)]
pub struct NegotiationFilter {
    state: State,
    /// Accumulated bytes of the current sub-negotiation (option byte first).
    subnegotiation: Vec<u8>,
    /// Negotiation replies produced by the last [`feed`] call, waiting to be
    /// written to the socket via [`take_responses`].
    ///
    /// [`feed`]: NegotiationFilter::feed
    /// [`take_responses`]: NegotiationFilter::take_responses
    responses: BytesMut,
    terminal_type: String,
    username: String,
    size: TerminalSize,
    /// Our reply to `DO`/`DONT` per option: `0` (none), [`WILL`] or [`WONT`].
    /// Used to suppress duplicate replies and to answer `DONT` correctly
    /// (reply `WONT` only when we had accepted the option).
    us: [u8; 256],
    /// Our reply to `WILL`/`WONT` per option: `0`, [`DO`] or [`DONT`].
    him: [u8; 256],
    /// Whether the server negotiated that *it* performs ECHO (remote echo).
    server_echo: bool,
}

impl NegotiationFilter {
    /// Build a filter with an explicit terminal type, login username (used
    /// for NEW-ENVIRON `USER`) and initial window size.
    pub fn new(
        terminal_type: impl Into<String>,
        username: impl Into<String>,
        size: TerminalSize,
    ) -> Self {
        Self {
            state: State::Data,
            subnegotiation: Vec::new(),
            responses: BytesMut::new(),
            terminal_type: terminal_type.into(),
            username: username.into(),
            size,
            us: [0; 256],
            him: [0; 256],
            server_echo: false,
        }
    }

    /// Filter with `xterm`, no username and the default 80×24 window.
    pub fn with_defaults() -> Self {
        Self::new(DEFAULT_TERMINAL_TYPE, "", TerminalSize::default())
    }

    pub fn set_terminal_type(&mut self, terminal_type: impl Into<String>) {
        self.terminal_type = terminal_type.into();
    }

    pub fn set_username(&mut self, username: impl Into<String>) {
        self.username = username.into();
    }

    /// Update the window size reported for NAWS. Call [`naws`] afterwards to
    /// obtain the bytes to write.
    ///
    /// [`naws`]: NegotiationFilter::naws
    pub fn set_size(&mut self, size: TerminalSize) {
        self.size = size;
    }

    pub fn size(&self) -> TerminalSize {
        self.size
    }

    /// `true` once the server has negotiated `WILL ECHO` (remote echo is on
    /// and the UI should not echo keystrokes locally).
    pub fn server_echo(&self) -> bool {
        self.server_echo
    }

    /// Bytes the client should write immediately after the TCP connection is
    /// established. The Termius native addon issues `IAC WILL NEW-ENVIRON`
    /// from `TelnetClientImpl::OnConnect` (conditionally, when its env-var
    /// list is non-empty — we always carry at least `TERM`).
    ///
    /// PORT-TODO: the addon's env list may contain more than `TERM`/`USER`
    /// (e.g. host `env_variables`); those live in the SSH config surface and
    /// are not present on `ConnectionParams`, so they are not sent yet.
    pub fn startup(&mut self) -> BytesMut {
        let mut out = BytesMut::with_capacity(3);
        out.put_u8(IAC);
        out.put_u8(WILL);
        out.put_u8(OPT_NEW_ENVIRON);
        self.us[OPT_NEW_ENVIRON as usize] = WILL;
        out
    }

    /// Feed raw bytes read from the socket; returns the payload bytes with all
    /// IAC sequences removed. Any negotiation replies are staged internally —
    /// fetch them with [`take_responses`] and write them back to the socket.
    ///
    /// [`take_responses`]: NegotiationFilter::take_responses
    pub fn feed(&mut self, input: &[u8]) -> BytesMut {
        let mut payload = BytesMut::with_capacity(input.len());
        for &byte in input {
            match self.state {
                State::Data => {
                    if byte == IAC {
                        self.state = State::Iac;
                    } else {
                        payload.put_u8(byte);
                    }
                }
                State::Iac => match byte {
                    // Escaped IAC: a literal 0xFF payload byte.
                    IAC => {
                        payload.put_u8(IAC);
                        self.state = State::Data;
                    }
                    WILL => self.state = State::Will,
                    WONT => self.state = State::Wont,
                    DO => self.state = State::Do,
                    DONT => self.state = State::Dont,
                    SB => {
                        self.subnegotiation.clear();
                        self.state = State::Subnegotiation;
                    }
                    // Remaining in-band commands (SE outside SB, GA, EL, EC,
                    // AYT, AO, IP, BREAK, DM, NOOP) and unknown commands are
                    // consumed without a reply: they carry no payload.
                    byte => {
                        tracing::trace!(byte, "stripped telnet command");
                        self.state = State::Data;
                    }
                },
                State::Will => {
                    self.on_will(byte);
                    self.state = State::Data;
                }
                State::Wont => {
                    self.on_wont(byte);
                    self.state = State::Data;
                }
                State::Do => {
                    self.on_do(byte);
                    self.state = State::Data;
                }
                State::Dont => {
                    self.on_dont(byte);
                    self.state = State::Data;
                }
                State::Subnegotiation => {
                    if byte == IAC {
                        self.state = State::SubnegotiationIac;
                    } else {
                        if self.subnegotiation.len() >= MAX_SUBNEGOTIATION {
                            // Desynchronise rather than grow without bound.
                            tracing::warn!(
                                limit = MAX_SUBNEGOTIATION,
                                "telnet sub-negotiation exceeded limit; discarded"
                            );
                            self.subnegotiation.clear();
                            self.state = State::Data;
                        } else {
                            self.subnegotiation.push(byte);
                        }
                    }
                }
                State::SubnegotiationIac => match byte {
                    SE => {
                        let subnegotiation = std::mem::take(&mut self.subnegotiation);
                        self.on_subnegotiation(&subnegotiation);
                        self.state = State::Data;
                    }
                    IAC => {
                        // Escaped literal 0xFF inside the sub-negotiation.
                        self.subnegotiation.push(IAC);
                        self.state = State::Subnegotiation;
                    }
                    _ => {
                        // IAC <other> inside SB is illegal (RFC 854); drop the
                        // byte and keep collecting until a well-formed end.
                        tracing::debug!(byte, "unexpected byte after IAC inside SB");
                        self.state = State::Subnegotiation;
                    }
                },
            }
        }
        payload
    }

    /// Drain the negotiation replies staged by [`feed`]. Must be written to
    /// the socket before or alongside further payload.
    ///
    /// [`feed`]: NegotiationFilter::feed
    pub fn take_responses(&mut self) -> BytesMut {
        std::mem::take(&mut self.responses)
    }

    /// `IAC SB NAWS <cols:16> <rows:16> IAC SE` for the current window size
    /// (4-byte big-endian, exactly what `TelnetClient::Resize` emits in the
    /// native addon). Payload byte 0xFF is doubled per the SB rules.
    pub fn naws(&self) -> BytesMut {
        let mut out = BytesMut::with_capacity(11);
        out.put_u8(IAC);
        out.put_u8(SB);
        out.put_u8(OPT_NAWS);
        let cols = self.size.cols;
        let rows = self.size.rows;
        for byte in [
            (cols >> 8) as u8,
            (cols & 0xFF) as u8,
            (rows >> 8) as u8,
            (rows & 0xFF) as u8,
        ] {
            out.put_u8(byte);
            if byte == IAC {
                out.put_u8(IAC);
            }
        }
        out.put_u8(IAC);
        out.put_u8(SE);
        out
    }

    // --- option negotiation -------------------------------------------------

    /// Server offers to perform an option. Reply `DO` (accept) or `DONT`
    /// (refuse) — those are the answers RFC 854 defines for `WILL`.
    fn on_will(&mut self, option: u8) {
        // Server-side roles we honour: ECHO and SUPPRESS-GO-AHEAD.
        let accept = matches!(option, OPT_ECHO | OPT_SGA);
        let reply = if accept { DO } else { DONT };
        if self.him[option as usize] == reply {
            // Duplicate request; our previous reply still stands.
            return;
        }
        self.him[option as usize] = reply;
        if option == OPT_ECHO {
            self.server_echo = accept;
        }
        tracing::debug!(option, accept, "telnet WILL");
        self.responses.put_u8(IAC);
        self.responses.put_u8(reply);
        self.responses.put_u8(option);
    }

    /// Server withdraws an option. Per RFC 1143 no reply is required.
    fn on_wont(&mut self, option: u8) {
        self.him[option as usize] = 0;
        if option == OPT_ECHO {
            self.server_echo = false;
        }
        tracing::debug!(option, "telnet WONT");
    }

    /// Server asks us to perform an option.
    fn on_do(&mut self, option: u8) {
        // The terminal performs TERMINAL-TYPE, window size and env vars —
        // everything else is refused.
        let accept = matches!(option, OPT_TTYPE | OPT_NAWS | OPT_NEW_ENVIRON);
        let reply = if accept { WILL } else { WONT };
        if self.us[option as usize] == reply {
            return;
        }
        self.us[option as usize] = reply;
        tracing::debug!(option, accept, "telnet DO");
        self.responses.put_u8(IAC);
        self.responses.put_u8(reply);
        self.responses.put_u8(option);
        if accept && option == OPT_NAWS {
            let naws = self.naws();
            self.responses.extend_from_slice(&naws[..]);
        }
    }

    /// Server tells us to stop performing an option. Reply `WONT` only if we
    /// had accepted it (RFC 1143: closing an active negotiation).
    fn on_dont(&mut self, option: u8) {
        let was_enabled = self.us[option as usize] == WILL;
        self.us[option as usize] = 0;
        if was_enabled {
            tracing::debug!(option, "telnet DONT");
            self.responses.put_u8(IAC);
            self.responses.put_u8(WONT);
            self.responses.put_u8(option);
        }
    }

    // --- sub-negotiations ---------------------------------------------------

    fn on_subnegotiation(&mut self, bytes: &[u8]) {
        let (Some(&option), Some(&command)) = (bytes.first(), bytes.get(1)) else {
            tracing::debug!("empty telnet sub-negotiation ignored");
            return;
        };
        match (option, command) {
            // IAC SB TERMINAL-TYPE SEND IAC SE → reply with our type.
            (OPT_TTYPE, TTYPE_SEND) => {
                tracing::debug!(terminal = %self.terminal_type, "TERMINAL-TYPE requested");
                let mut reply = BytesMut::with_capacity(self.terminal_type.len() + 6);
                reply.put_u8(IAC);
                reply.put_u8(SB);
                reply.put_u8(OPT_TTYPE);
                reply.put_u8(TTYPE_IS);
                push_escaped(&mut reply, self.terminal_type.as_bytes());
                reply.put_u8(IAC);
                reply.put_u8(SE);
                self.responses.extend_from_slice(&reply[..]);
            }
            // IAC SB NEW-ENVIRON SEND … IAC SE → reply with TERM (+ USER when
            // a login name is configured). Requested variable lists are not
            // filtered: extra values are tolerated by RFC 1572 peers.
            (OPT_NEW_ENVIRON, ENV_SEND) => {
                tracing::debug!("NEW-ENVIRON requested");
                let mut reply = BytesMut::with_capacity(32);
                reply.put_u8(IAC);
                reply.put_u8(SB);
                reply.put_u8(OPT_NEW_ENVIRON);
                reply.put_u8(ENV_IS);
                push_env_var(&mut reply, b"TERM", self.terminal_type.as_bytes());
                if !self.username.is_empty() {
                    push_env_var(&mut reply, b"USER", self.username.as_bytes());
                }
                reply.put_u8(IAC);
                reply.put_u8(SE);
                self.responses.extend_from_slice(&reply[..]);
            }
            // Servers do not send NAWS to clients; ignore defensively.
            (OPT_NAWS, _) => {}
            _ => {
                tracing::debug!(option, command, "telnet sub-negotiation ignored");
            }
        }
    }
}

/// Client→server encoding: doubles every `IAC` byte and normalises line
/// endings the way `TelnetClientImpl::Send` does in the native addon —
/// `CR` becomes `CR LF`, a lone `LF` becomes `CR LF`, and an `LF` directly
/// following a `CR` is dropped (the `CR` already emitted it).
///
/// This is the encode path for interactive keystrokes and pasted text; do not
/// use it for data whose 0x0D/0x0A bytes must survive verbatim.
pub fn encode(data: &[u8]) -> BytesMut {
    let mut out = BytesMut::with_capacity(data.len() + 4);
    for (index, &byte) in data.iter().enumerate() {
        match byte {
            CR => {
                out.put_u8(CR);
                out.put_u8(LF);
            }
            LF => {
                let previous_is_cr = index > 0 && data[index - 1] == CR;
                if !previous_is_cr {
                    out.put_u8(CR);
                    out.put_u8(LF);
                }
            }
            _ => push_escaped(&mut out, &[byte]),
        }
    }
    out
}

/// Append raw bytes, doubling `IAC` (telnet data escaping inside and outside
/// sub-negotiations).
fn push_escaped(out: &mut BytesMut, bytes: &[u8]) {
    for &byte in bytes {
        out.put_u8(byte);
        if byte == IAC {
            out.put_u8(IAC);
        }
    }
}

/// Append `VAR name VALUE value`, escaping `IAC` and the NEW-ENVIRON
/// structural octets inside the value (RFC 1572 §3).
fn push_env_var(out: &mut BytesMut, name: &[u8], value: &[u8]) {
    out.put_u8(ENV_VAR);
    push_escaped(out, name);
    out.put_u8(ENV_VALUE);
    for &byte in value {
        match byte {
            IAC => {
                out.put_u8(IAC);
                out.put_u8(IAC);
            }
            ENV_VAR | ENV_VALUE | ENV_ESC | ENV_USERVAR => {
                out.put_u8(ENV_ESC);
                out.put_u8(byte);
            }
            _ => out.put_u8(byte),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WILL_ECHO: [u8; 3] = [IAC, WILL, OPT_ECHO];
    const DO_ECHO: [u8; 3] = [IAC, DO, OPT_ECHO];

    #[test]
    fn will_echo_is_answered_with_do_echo() {
        let mut filter = NegotiationFilter::with_defaults();
        let payload = filter.feed(&WILL_ECHO);
        assert!(payload.is_empty(), "negotiation must not leak into payload");
        let responses = filter.take_responses();
        assert_eq!(&responses[..], &DO_ECHO);
        assert!(filter.server_echo());
    }

    #[test]
    fn payload_passes_through_unmodified() {
        let mut filter = NegotiationFilter::with_defaults();
        let payload = filter.feed(b"ls -la\r\n$ \x07");
        assert_eq!(&payload[..], b"ls -la\r\n$ \x07");
        assert!(filter.take_responses().is_empty());
    }

    #[test]
    fn payload_around_negotiation_is_preserved() {
        let mut filter = NegotiationFilter::with_defaults();
        let mut input = Vec::from(&b"ab"[..]);
        input.extend_from_slice(&WILL_ECHO);
        input.extend_from_slice(&b"cd"[..]);
        let payload = filter.feed(&input);
        assert_eq!(&payload[..], b"abcd");
        assert_eq!(&filter.take_responses()[..], &DO_ECHO);
    }

    #[test]
    fn escaped_iac_becomes_single_payload_ff() {
        let mut filter = NegotiationFilter::with_defaults();
        let payload = filter.feed(&[IAC, IAC]);
        assert_eq!(&payload[..], &[0xFF]);
        assert!(filter.take_responses().is_empty());
    }

    #[test]
    fn unsupported_server_will_is_refused_with_dont() {
        let mut filter = NegotiationFilter::with_defaults();
        let payload = filter.feed(&[IAC, WILL, 34 /* LINEMODE */]);
        assert!(payload.is_empty());
        assert_eq!(&filter.take_responses()[..], &[IAC, DONT, 34]);
    }

    #[test]
    fn will_sga_is_accepted() {
        let mut filter = NegotiationFilter::with_defaults();
        let _ = filter.feed(&[IAC, WILL, OPT_SGA]);
        assert_eq!(&filter.take_responses()[..], &[IAC, DO, OPT_SGA]);
    }

    #[test]
    fn do_echo_is_refused_with_wont() {
        let mut filter = NegotiationFilter::with_defaults();
        let _ = filter.feed(&DO_ECHO);
        assert_eq!(&filter.take_responses()[..], &[IAC, WONT, OPT_ECHO]);
        assert!(!filter.server_echo());
    }

    #[test]
    fn duplicate_negotiation_replies_are_suppressed() {
        let mut filter = NegotiationFilter::with_defaults();
        let _ = filter.feed(&WILL_ECHO);
        let first = filter.take_responses();
        assert_eq!(&first[..], &DO_ECHO);
        // Server repeats the same request; no second reply is produced.
        let _ = filter.feed(&WILL_ECHO);
        assert!(filter.take_responses().is_empty());
    }

    #[test]
    fn do_naws_accepts_and_reports_the_window_size() {
        let mut filter = NegotiationFilter::with_defaults();
        let _ = filter.feed(&[IAC, DO, OPT_NAWS]);
        let responses = filter.take_responses();
        let expected = [
            IAC, WILL, OPT_NAWS, IAC, SB, OPT_NAWS, 0, 80, 0, 24, IAC, SE,
        ];
        assert_eq!(&responses[..], &expected);
    }

    #[test]
    fn resize_updates_naws_payload() {
        let mut filter = NegotiationFilter::with_defaults();
        filter.set_size(TerminalSize {
            cols: 132,
            rows: 43,
            width_px: None,
            height_px: None,
        });
        let naws = filter.naws();
        let expected = [
            IAC, SB, OPT_NAWS, 0, 132, 0, 43, IAC, SE,
        ];
        assert_eq!(&naws[..], &expected);
    }

    #[test]
    fn naws_escapes_ff_inside_subnegotiation() {
        let mut filter = NegotiationFilter::with_defaults();
        filter.set_size(TerminalSize {
            cols: 65535, // 0xFF 0xFF — must be doubled
            rows: 1,
            width_px: None,
            height_px: None,
        });
        let naws = filter.naws();
        let expected = [
            IAC, SB, OPT_NAWS, IAC, IAC, IAC, IAC, 0, 1, IAC, SE,
        ];
        assert_eq!(&naws[..], &expected);
    }

    #[test]
    fn terminal_type_query_is_answered_with_xterm() {
        let mut filter = NegotiationFilter::with_defaults();
        let payload = filter.feed(&[IAC, SB, OPT_TTYPE, TTYPE_SEND, IAC, SE]);
        assert!(payload.is_empty());
        let mut expected = vec![IAC, SB, OPT_TTYPE, TTYPE_IS];
        expected.extend_from_slice(b"xterm");
        expected.extend_from_slice(&[IAC, SE]);
        assert_eq!(&filter.take_responses()[..], &expected);
    }

    #[test]
    fn new_environ_query_returns_term_and_user() {
        let mut filter =
            NegotiationFilter::new(DEFAULT_TERMINAL_TYPE, "root", TerminalSize::default());
        let request = [
            IAC, SB, OPT_NEW_ENVIRON, ENV_SEND, ENV_VAR, b'U', b'S', b'E', b'R', IAC, SE,
        ];
        let payload = filter.feed(&request);
        assert!(payload.is_empty());
        let mut expected = vec![IAC, SB, OPT_NEW_ENVIRON, ENV_IS];
        expected.extend_from_slice(&[ENV_VAR]);
        expected.extend_from_slice(b"TERM");
        expected.extend_from_slice(&[ENV_VALUE]);
        expected.extend_from_slice(b"xterm");
        expected.extend_from_slice(&[ENV_VAR]);
        expected.extend_from_slice(b"USER");
        expected.extend_from_slice(&[ENV_VALUE]);
        expected.extend_from_slice(b"root");
        expected.extend_from_slice(&[IAC, SE]);
        assert_eq!(&filter.take_responses()[..], &expected);
    }

    #[test]
    fn in_band_commands_are_stripped_without_reply() {
        let mut filter = NegotiationFilter::with_defaults();
        let payload = filter.feed(&[b'x', IAC, NOOP, IAC, BREAK, IAC, GA, b'y']);
        assert_eq!(&payload[..], b"xy");
        assert!(filter.take_responses().is_empty());
    }

    #[test]
    fn negotiation_split_across_byte_at_a_time_feeds() {
        let mut filter = NegotiationFilter::with_defaults();
        let mut payload = BytesMut::new();
        for byte in [IAC, WILL, OPT_ECHO, b'h', IAC, IAC, b'i'] {
            payload.extend_from_slice(&filter.feed(&[byte]));
        }
        assert_eq!(&payload[..], b"h\xFFi");
        assert_eq!(&filter.take_responses()[..], &DO_ECHO);
    }

    #[test]
    fn subnegotiation_split_across_chunks() {
        let mut filter = NegotiationFilter::with_defaults();
        let mut payload = BytesMut::new();
        for chunk in [
            &[IAC, SB, OPT_TTYPE][..],
            &[TTYPE_SEND, IAC, SE, b'p'][..],
        ] {
            payload.extend_from_slice(&filter.feed(chunk));
        }
        assert_eq!(&payload[..], b"p");
        let mut expected = vec![IAC, SB, OPT_TTYPE, TTYPE_IS];
        expected.extend_from_slice(b"xterm");
        expected.extend_from_slice(&[IAC, SE]);
        assert_eq!(&filter.take_responses()[..], &expected);
    }

    #[test]
    fn encode_escapes_iac_bytes() {
        let encoded = encode(b"a\xFFb");
        assert_eq!(&encoded[..], &[b'a', IAC, IAC, b'b']);
    }

    #[test]
    fn encode_normalises_line_endings_like_the_native_client() {
        // Bare CR → CR LF (NVT end of line).
        assert_eq!(&encode(b"ok\r")[..], b"ok\r\n");
        // Bare LF → CR LF.
        assert_eq!(&encode(b"\n")[..], b"\r\n");
        // CRLF stays a single CRLF (the LF after CR is dropped).
        assert_eq!(&encode(b"\r\n")[..], b"\r\n");
        // Mixed content keeps non-line bytes verbatim.
        assert_eq!(&encode(b"a\r\nb")[..], b"a\r\nb");
    }

    #[test]
    fn startup_issues_will_new_environ() {
        let mut filter = NegotiationFilter::with_defaults();
        let greeting = filter.startup();
        assert_eq!(&greeting[..], &[IAC, WILL, OPT_NEW_ENVIRON]);
        // A subsequent DO NEW-ENVIRON must not repeat the WILL.
        let _ = filter.feed(&[IAC, DO, OPT_NEW_ENVIRON]);
        assert!(filter.take_responses().is_empty());
        // A DONT closes the proactive negotiation with WONT.
        let _ = filter.feed(&[IAC, DONT, OPT_NEW_ENVIRON]);
        assert_eq!(
            &filter.take_responses()[..],
            &[IAC, WONT, OPT_NEW_ENVIRON]
        );
    }
}
