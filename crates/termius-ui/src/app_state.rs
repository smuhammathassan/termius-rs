//! app_state — the central state store replacing Termius' React/Redux model.
//!
//! [`TermiusState`] is one GPUI entity holding everything the renderer shows:
//! the loaded library (hosts/groups/snippets/identities/keys/keychains), the
//! selection, the open terminal sessions (each pairing a main-thread
//! [`Terminal`] with its engine bridge), the active tab, and the UI flags.
//!
//! # Threading model
//!
//! * **Render thread (GPUI main thread)** — `TermiusState`, every `Terminal`,
//!   snapshots. Never blocked: storage and network always run elsewhere.
//! * **tokio (engine runtime)** — [`run_bridge`] owns the `SshClient`, its
//!   shell [`ChannelHandle`](termius_ssh::ChannelHandle) and the optional
//!   SFTP client. Input commands arrive on an `mpsc`; output chunks leave on a
//!   second `mpsc`.
//! * **Bridge → UI hop** — a GPUI foreground task pumps the event channel and
//!   folds bytes into the `Terminal` via `this.update(..)` →
//!   [`TermiusState::apply_bridge_event`] → `Terminal::advance` → fresh
//!   [`GridSnapshot`] → `cx.notify()`. Awaiting an `mpsc` receiver needs no
//!   tokio reactor, so that hop is safe on the GPUI executor.
//!
//! Storage (`Store` is synchronous `std::Mutex`) is opened and listed through
//! `cx.background_executor()`, never on the render thread.
//!
//! # Screen contract (UI waves)
//!
//! Besides the session plumbing, the state carries the per-section slices the
//! screens render ([`TermiusState::library`], [`TermiusState::port_forwardings`],
//! [`TermiusState::known_hosts`], [`TermiusState::settings`],
//! [`TermiusState::account`]), the navigation target
//! ([`TermiusState::current_section`], set through
//! [`TermiusState::set_section`]) and the modal host
//! ([`TermiusState::active_dialog`], [`TermiusState::open_dialog`],
//! [`TermiusState::close_dialog`]). CRUD helpers (`add_host`,
//! `delete_snippet`, …) mutate the slice, best-effort persist through the
//! held [`Store`], then `cx.notify()`.

use std::sync::{Arc, OnceLock};

use gpui::Context;
use termius_core::{
    AuthMethod, ConnectionParams, Group, Host, Identity, Key, Keychain, KnownHost,
    PortForwardingConfig, RecordId, Snippet, TerminalSize,
};
use termius_ssh::{join_path, Connector, SftpClient, SftpEntry};
use termius_storage::Store;
use termius_terminal::{
    GridSnapshot, Key as TermKey, SelectionMode, StyledCell, Terminal, TermEvent,
};
use tokio::sync::{mpsc, oneshot};

use crate::error::{Result, UiError};
use crate::navigation::Section;
use crate::theme::ThemeMode;

/// Bounded input queue per session (keystrokes + control commands).
const INPUT_CAPACITY: usize = 256;
/// Bounded output queue between bridge and UI pump.
const OUTPUT_CAPACITY: usize = 64;

/// Commands the UI sends to a session's bridge task.
pub enum SessionCommand {
    /// Keystrokes / paste bytes for the PTY.
    Input(Vec<u8>),
    /// `window-change` after a pane resize.
    Resize(TerminalSize),
    /// List a remote directory; the answer travels on the oneshot.
    SftpList {
        path: String,
        reply: oneshot::Sender<std::result::Result<Vec<SftpEntry>, String>>,
    },
    /// Graceful shutdown (EOF + disconnect).
    Disconnect,
}

/// Events the bridge task sends back to the main thread.
enum BridgeEvent {
    /// Shell channel is open; the session is usable.
    Ready,
    /// A chunk of remote output for `Terminal::advance`.
    Output(Vec<u8>),
    /// Connect/shell setup failed; the session is dead.
    Failed(String),
    /// The shell ended (remote exit, disconnect, channel loss).
    Closed { reason: String },
}

/// Lifecycle of one open session tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionStatus {
    Connecting,
    Ready,
    Closed { reason: String },
    Failed { message: String },
}

impl SessionStatus {
    /// Short label for tabs and the status bar.
    pub fn describe(&self) -> String {
        match self {
            Self::Connecting => "connecting…".to_owned(),
            Self::Ready => "connected".to_owned(),
            Self::Closed { reason } => format!("closed ({reason})"),
            Self::Failed { message } => format!("failed: {message}"),
        }
    }

    /// True while the PTY can still accept input.
    pub fn is_live(&self) -> bool {
        matches!(self, Self::Connecting | Self::Ready)
    }
}

/// State of the SFTP browser for one session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SftpState {
    /// Remote directory currently listed.
    pub path: String,
    pub entries: Vec<SftpEntry>,
    pub loading: bool,
    pub error: Option<String>,
}

impl Default for SftpState {
    fn default() -> Self {
        Self { path: "/".to_owned(), entries: Vec::new(), loading: false, error: None }
    }
}

/// One open terminal session = a tab.
pub struct Session {
    pub id: String,
    pub host: Host,
    /// Tab label: OSC title when the remote sets one, else the host label.
    pub title: String,
    pub status: SessionStatus,
    /// The VT emulator — main-thread only, never touched off-thread.
    pub term: Terminal,
    /// Last painted frame; refreshed whenever the terminal is dirty.
    pub frame: GridSnapshot,
    /// Current emulator geometry.
    pub size: TerminalSize,
    /// Geometry last sent to the remote PTY (dedupes resize traffic).
    pub sent_size: TerminalSize,
    /// Control channel into the bridge task (input/resize/sftp/disconnect).
    pub input: mpsc::Sender<SessionCommand>,
    pub sftp: SftpState,
}

impl Session {
    /// The label shown on the session tab.
    pub fn tab_label(&self) -> String {
        if self.title.trim().is_empty() {
            self.host.label.clone()
        } else {
            self.title.clone()
        }
    }

    /// True while the bridge can still deliver input.
    pub fn is_live(&self) -> bool {
        self.status.is_live()
    }
}

/// The loaded local library (everything the sidebar/panels render).
#[derive(Debug, Default)]
pub struct Library {
    pub hosts: Vec<Host>,
    pub groups: Vec<Group>,
    pub snippets: Vec<Snippet>,
    pub identities: Vec<Identity>,
    pub keys: Vec<Key>,
    pub keychains: Vec<Keychain>,
}

impl Library {
    pub fn host(&self, id: &str) -> Option<&Host> {
        self.hosts.iter().find(|host| host.id == id)
    }
}

// ---------------------------------------------------------------------------
// Navigation, dialogs and settings (the screen-side contract)
// ---------------------------------------------------------------------------

/// A modal the shell hosts on top of the whole window.
///
/// Bodies are filled in by the owning screen; for now the shell renders a
/// stub card (title + placeholder body + close button) — see
/// [`views::AppShell`](crate::views::AppShell).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dialog {
    /// "Add Host" form.
    AddHost,
    /// Edit the host with this id.
    EditHost(RecordId),
    /// "Add Snippet" form.
    AddSnippet,
    /// Edit the snippet with this id.
    EditSnippet(RecordId),
    /// "Add Key" form.
    AddKey,
    /// "Add Port Forward" form.
    AddPortForward,
    /// A generic confirmation (delete…).
    Confirm { title: String, message: String },
}

impl Dialog {
    /// The dialog's title-bar text.
    pub fn title(&self) -> String {
        match self {
            Self::AddHost => "Add Host".to_owned(),
            Self::EditHost(_) => "Edit Host".to_owned(),
            Self::AddSnippet => "Add Snippet".to_owned(),
            Self::EditSnippet(_) => "Edit Snippet".to_owned(),
            Self::AddKey => "Add Key".to_owned(),
            Self::AddPortForward => "Add Port Forward".to_owned(),
            Self::Confirm { title, .. } => title.clone(),
        }
    }

    /// The section whose screen owns this dialog (`None` for confirmations,
    /// which any section can raise).
    pub fn section(&self) -> Option<Section> {
        match self {
            Self::AddHost | Self::EditHost(_) => Some(Section::Hosts),
            Self::AddSnippet | Self::EditSnippet(_) => Some(Section::Snippets),
            Self::AddKey => Some(Section::Keys),
            Self::AddPortForward => Some(Section::PortForwarding),
            Self::Confirm { .. } => None,
        }
    }

    /// The confirmation message (only [`Dialog::Confirm`] has one).
    pub fn message(&self) -> Option<&str> {
        match self {
            Self::Confirm { message, .. } => Some(message.as_str()),
            _ => None,
        }
    }
}

/// User intent against the dialog host: the pure input to [`dialog_after`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DialogIntent {
    /// Show `dialog`, replacing whatever is open.
    Open(Dialog),
    /// Dismiss whatever is open.
    Close,
}

/// What the active dialog becomes after `intent`.
///
/// The contract (kept free of `Context` so it is unit-testable without a
/// window):
/// * `Open` **replaces** the active dialog — dialogs never stack,
/// * `Close` always clears — closing an already-closed host is a no-op.
pub fn dialog_after(_active: Option<Dialog>, intent: DialogIntent) -> Option<Dialog> {
    match intent {
        DialogIntent::Open(dialog) => Some(dialog),
        DialogIntent::Close => None,
    }
}

/// The app settings a Settings screen edits.
///
/// Defaults mirror the shipped Termius appearance (dark theme, Menlo 13).
///
/// PORT-TODO: persist through `store.settings()` (key/value) and apply
/// `theme_mode` to the [`TermiusTheme`](crate::theme::TermiusTheme) global on
/// change.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsState {
    /// Palette the app should show (see [`ThemeMode`]).
    pub theme_mode: ThemeMode,
    /// Terminal font family name.
    pub font_family: String,
    /// Terminal font size in points.
    pub font_size: u16,
    /// Terminal scrollback capacity in lines.
    pub scrollback_lines: usize,
    /// Blink the terminal cursor.
    pub cursor_blink: bool,
    /// Reconnect dropped sessions automatically.
    pub auto_reconnect: bool,
}

impl Default for SettingsState {
    fn default() -> Self {
        Self {
            theme_mode: ThemeMode::Dark,
            font_family: "Menlo".to_owned(),
            font_size: 13,
            scrollback_lines: 10_000,
            cursor_blink: true,
            auto_reconnect: true,
        }
    }
}

/// The signed-in account summary (Account / Team screens).
///
/// `None` on [`TermiusState::account`] means signed out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountInfo {
    /// Sign-in email (empty while local-only).
    pub email: String,
    /// Subscription plan label ("Free", "Pro"…).
    pub plan: String,
    /// How many devices are synced to this account.
    pub device_count: usize,
}

impl Default for AccountInfo {
    fn default() -> Self {
        Self {
            email: String::new(),
            plan: "Free".to_owned(),
            device_count: 0,
        }
    }
}

/// A best-effort unique id for records created outside storage
/// (`<prefix>-<millis>-<seq>`).
///
/// Termius proper uses UUIDv4; the local store only needs a stable unique
/// string, and no uuid crate is in the workspace yet.
fn fresh_id(prefix: &str) -> RecordId {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(1);
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0);
    format!("{prefix}-{millis}-{}", SEQ.fetch_add(1, Ordering::Relaxed))
}

/// The Redux-shaped application state, as one GPUI entity.
pub struct TermiusState {
    pub library: Library,
    /// Kept for CRUD the panels will grow (create/edit host…).
    pub store: Option<Arc<Store>>,
    pub library_loading: bool,
    pub library_error: Option<String>,
    /// Selected host row in the sidebar.
    pub selected_host: Option<String>,
    /// Open sessions, in tab order.
    pub sessions: Vec<Session>,
    /// The visible tab.
    pub active_session: Option<String>,
    pub sidebar_visible: bool,
    pub sftp_visible: bool,
    /// Status-bar text.
    pub status_text: String,
    // ----- screen contract (navigation, dialogs, per-section slices) -----
    /// The sidebar section currently routed in the shell.
    pub current_section: Section,
    /// The modal currently hosted above the window, if any.
    pub active_dialog: Option<Dialog>,
    /// Port-forwarding rules (Port Forwarding section).
    pub port_forwardings: Vec<PortForwardingConfig>,
    /// Pinned server fingerprints (Keychain / host-key prompts).
    pub known_hosts: Vec<KnownHost>,
    /// App settings (Settings section; defaults until persisted).
    pub settings: SettingsState,
    /// Signed-in account, if any (`None` = signed out).
    pub account: Option<AccountInfo>,
    next_session_seq: u64,
}

impl TermiusState {
    /// Create the state and kick off the background library load.
    pub fn new(cx: &mut Context<Self>) -> Self {
        let state = Self {
            library: Library::default(),
            store: None,
            library_loading: true,
            library_error: None,
            selected_host: None,
            sessions: Vec::new(),
            active_session: None,
            sidebar_visible: true,
            sftp_visible: false,
            status_text: "Loading hosts…".to_owned(),
            current_section: Section::Hosts,
            active_dialog: None,
            port_forwardings: Vec::new(),
            known_hosts: Vec::new(),
            settings: SettingsState::default(),
            account: None,
            next_session_seq: 0,
        };
        state.spawn_load_library(cx);
        state
    }

    // ----- library --------------------------------------------------------

    /// Open the store and list every repository on a background thread.
    fn spawn_load_library(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { load_library() })
                .await;
            this.update(cx, |state, cx| {
                state.library_loading = false;
                match result {
                    Ok(loaded) => {
                        state.status_text = format!(
                            "{} hosts · {} groups",
                            loaded.library.hosts.len(),
                            loaded.library.groups.len()
                        );
                        state.library = loaded.library;
                        state.port_forwardings = loaded.port_forwardings;
                        state.known_hosts = loaded.known_hosts;
                        state.store = Some(loaded.store);
                        state.library_error = None;
                    }
                    Err(err) => {
                        tracing::error!(error = %err, "failed to load local library");
                        state.library_error = Some(err.to_string());
                        state.status_text = format!("Library error: {err}");
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    // ----- selection / flags ---------------------------------------------

    /// Select a host row (clears with `None`).
    pub fn select_host(&mut self, host_id: Option<String>, cx: &mut Context<Self>) {
        self.selected_host = host_id;
        cx.notify();
    }

    /// The selected host record.
    pub fn selected_host(&self) -> Option<&Host> {
        self.selected_host.as_deref().and_then(|id| self.library.host(id))
    }

    /// Toggle the host-list sidebar (⌘B).
    pub fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_visible = !self.sidebar_visible;
        cx.notify();
    }

    /// Toggle the SFTP side panel (⌘⇧F); opening it lists the active
    /// session's current directory.
    pub fn toggle_sftp(&mut self, cx: &mut Context<Self>) {
        self.sftp_visible = !self.sftp_visible;
        if self.sftp_visible {
            let target = self.active_session.clone();
            if let Some(session_id) = target {
                let path = self
                    .session(&session_id)
                    .map(|session| session.sftp.path.clone())
                    .unwrap_or_else(|| "/".to_owned());
                self.sftp_list(&session_id, path, cx);
            }
        }
        cx.notify();
    }

    // ----- navigation / dialogs ------------------------------------------

    /// Route the shell to a section (sidebar nav).
    ///
    /// Landing on [`Section::Sftp`] also opens the SFTP side panel (the same
    /// affordance as ⌘⇧F) so the section's browser is reachable immediately.
    pub fn set_section(&mut self, section: Section, cx: &mut Context<Self>) {
        if self.current_section == section {
            return;
        }
        self.current_section = section;
        if section == Section::Sftp && !self.sftp_visible {
            self.toggle_sftp(cx);
        }
        cx.notify();
    }

    /// Show a modal on top of the window (replaces any open dialog).
    pub fn open_dialog(&mut self, dialog: Dialog, cx: &mut Context<Self>) {
        self.active_dialog = dialog_after(self.active_dialog.take(), DialogIntent::Open(dialog));
        cx.notify();
    }

    /// Dismiss the active dialog (no-op when none is open).
    pub fn close_dialog(&mut self, cx: &mut Context<Self>) {
        if self.active_dialog.is_none() {
            return;
        }
        self.active_dialog = dialog_after(self.active_dialog.take(), DialogIntent::Close);
        cx.notify();
    }

    /// Select a session tab.
    pub fn select_session(&mut self, session_id: &str, cx: &mut Context<Self>) {
        if self.sessions.iter().any(|s| s.id == session_id) {
            self.active_session = Some(session_id.to_owned());
            cx.notify();
        }
    }

    /// The session currently rendered in the terminal pane.
    pub fn active_session(&self) -> Option<&Session> {
        let active = self.active_session.as_deref()?;
        self.sessions.iter().find(|s| s.id == active)
    }

    /// Look up an open session by id.
    pub fn session(&self, session_id: &str) -> Option<&Session> {
        self.sessions.iter().find(|s| s.id == session_id)
    }

    // ----- connection -----------------------------------------------------

    /// Open (or focus) a shell session for `host_id`.
    ///
    /// Creates the session row synchronously so the tab appears immediately
    /// with a `Connecting` status, then spawns the tokio bridge and the GPUI
    /// pump task that folds its output into the terminal.
    pub fn open_connection(&mut self, host_id: &str, cx: &mut Context<Self>) {
        // Re-focus a live session instead of opening a duplicate.
        if let Some(existing) = self.sessions.iter().find(|s| s.host.id == host_id) {
            if existing.is_live() {
                let id = existing.id.clone();
                self.active_session = Some(id);
                cx.notify();
                return;
            }
        }
        // Retire stale rows for the same host (closed/failed).
        if let Some(pos) = self.sessions.iter().position(|s| s.host.id == host_id && !s.is_live())
        {
            let stale = self.sessions.remove(pos);
            let _ = stale.input.try_send(SessionCommand::Disconnect);
        }

        let Some(host) = self.library.host(host_id).cloned() else {
            self.status_text = format!("Unknown host `{host_id}`");
            cx.notify();
            return;
        };
        let params = params_for(&host, &self.library);

        self.next_session_seq += 1;
        let session_id = format!("{}-{}", host.id, self.next_session_seq);
        let size = params.terminal;
        let term = match Terminal::new(size) {
            Ok(term) => term,
            Err(err) => {
                self.status_text = format!("Terminal error: {err}");
                cx.notify();
                return;
            }
        };
        let (input_tx, input_rx) = mpsc::channel::<SessionCommand>(INPUT_CAPACITY);
        let (event_tx, mut event_rx) = mpsc::channel::<BridgeEvent>(OUTPUT_CAPACITY);

        self.sessions.push(Session {
            id: session_id.clone(),
            host: host.clone(),
            title: host.label.clone(),
            status: SessionStatus::Connecting,
            term,
            frame: blank_frame(size),
            size,
            sent_size: size,
            input: input_tx,
            sftp: SftpState::default(),
        });
        self.active_session = Some(session_id.clone());
        self.status_text = format!("Connecting to {}…", host.label);
        cx.notify();

        // Engine side: tokio task owning client + shell + sftp.
        match engine_runtime() {
            Ok(runtime) => {
                runtime.spawn(async move {
                    run_bridge(params, input_rx, event_tx).await;
                });
            }
            Err(err) => {
                self.fail_session(&session_id, &err.to_string());
                cx.notify();
                return;
            }
        }

        // UI side: pump bridge events into the main-thread Terminal.
        //
        // PORT-TODO(gpui 0.2): this future captures `WeakEntity<TermiusState>`
        // where the state holds `!Send` Terminals. If `cx.spawn` ever grows a
        // `Send` bound, move the Terminals into a main-thread registry and
        // keep only an id + channel in the entity.
        let pump_id = session_id.clone();
        cx.spawn(async move |this, cx| {
            while let Some(event) = event_rx.recv().await {
                let keep =
                    this.update(cx, |state, cx| state.apply_bridge_event(&pump_id, event, cx))
                        .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        })
        .detach();
    }

    /// Fold one bridge event into state + terminal; returns `false` when the
    /// pump task should stop (session gone or terminal event).
    fn apply_bridge_event(
        &mut self,
        session_id: &str,
        event: BridgeEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        match event {
            BridgeEvent::Ready => {
                // Mutate the session, then (borrow released) touch the rest.
                let ready = self.session_mut(session_id).map(|session| {
                    session.status = SessionStatus::Ready;
                    (session.tab_label(), session.sftp.path.clone())
                });
                let Some((label, path)) = ready else { return false };
                self.status_text = format!("Connected to {label}");
                cx.notify();
                if self.sftp_visible {
                    self.sftp_list(session_id, path, cx);
                }
                true
            }
            BridgeEvent::Output(bytes) => {
                let Some(session) = self.session_mut(session_id) else { return false };
                session.term.advance(&bytes);
                // Emulator replies (DSR…) and titles ride the same hop.
                for event in session.term.drain_events() {
                    match event {
                        TermEvent::DataToWrite(data) => {
                            let _ = session.input.try_send(SessionCommand::Input(data.into_bytes()));
                        }
                        TermEvent::Title(title) => session.title = title,
                        TermEvent::Advance | TermEvent::Other => {}
                    }
                }
                if session.term.take_dirty() {
                    session.frame = session.term.snapshot();
                }
                cx.notify();
                true
            }
            BridgeEvent::Failed(message) => {
                self.fail_session(session_id, &message);
                cx.notify();
                false
            }
            BridgeEvent::Closed { reason } => {
                let closed = self.session_mut(session_id).map(|session| {
                    session.status = SessionStatus::Closed { reason: reason.clone() };
                    format!("{} — {}", session.tab_label(), session.status.describe())
                });
                if let Some(text) = closed {
                    self.status_text = text;
                }
                cx.notify();
                false
            }
        }
    }

    fn fail_session(&mut self, session_id: &str, message: &str) {
        let failed = self.session_mut(session_id).map(|session| {
            session.status = SessionStatus::Failed { message: message.to_owned() };
            format!("{} — failed: {message}", session.tab_label())
        });
        if let Some(text) = failed {
            self.status_text = text;
        }
    }

    /// Close a session tab: detach it, tell the bridge to disconnect, and
    /// pick the neighbour tab.
    pub fn close_session(&mut self, session_id: &str, cx: &mut Context<Self>) {
        let Some(pos) = self.sessions.iter().position(|s| s.id == session_id) else { return };
        let ids: Vec<String> = self.sessions.iter().map(|s| s.id.clone()).collect();
        let id_refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        self.active_session =
            active_after_close(&id_refs, self.active_session.as_deref(), session_id);

        let session = self.sessions.remove(pos);
        // Explicit disconnect first; dropping `input` then ends the bridge
        // loop even if the command never made it through a full queue.
        let _ = session.input.try_send(SessionCommand::Disconnect);
        self.status_text = format!("Closed {}", session.tab_label());
        cx.notify();
    }

    // ----- input / geometry ----------------------------------------------

    /// Encode + ship one key press for a session.
    pub fn send_key(&mut self, session_id: &str, key: TermKey, cx: &mut Context<Self>) {
        self.send_keys(session_id, &[key], cx);
    }

    /// Encode + ship a key sequence (alt needs ESC + key as two units).
    pub fn send_keys(&mut self, session_id: &str, keys: &[TermKey], cx: &mut Context<Self>) {
        let Some(session) = self.session(session_id) else { return };
        if !session.is_live() || keys.is_empty() {
            return;
        }
        let mut bytes = Vec::new();
        for key in keys {
            bytes.extend_from_slice(&session.term.key_down(*key));
        }
        let input = session.input.clone();
        // try_send never blocks the render thread; a full queue at human
        // typing speed means the bridge is wedged and the drop is correct.
        if input.try_send(SessionCommand::Input(bytes)).is_err() {
            tracing::warn!(session = session_id, "input queue unavailable");
        }
        cx.notify();
    }

    /// Ship a paste (bracketed when the app enabled it).
    pub fn send_paste(&mut self, session_id: &str, text: &str, cx: &mut Context<Self>) {
        let Some(session) = self.session(session_id) else { return };
        if !session.is_live() || text.is_empty() {
            return;
        }
        let bytes = session.term.paste(text);
        let _ = session.input.try_send(SessionCommand::Input(bytes));
        cx.notify();
    }

    /// Re-fit the emulator + PTY to a new geometry.
    pub fn resize_session(&mut self, session_id: &str, size: TerminalSize, cx: &mut Context<Self>) {
        let Some(session) = self.session_mut(session_id) else { return };
        if size.cols == 0 || size.rows == 0 || size == session.size {
            return;
        }
        if let Err(err) = session.term.resize(size) {
            tracing::warn!(session = session_id, error = %err, "resize rejected");
            return;
        }
        session.size = size;
        session.frame = session.term.snapshot();
        if session.sent_size != size {
            session.sent_size = size;
            let _ = session.input.try_send(SessionCommand::Resize(size));
        }
        cx.notify();
    }

    /// Scroll the visible viewport (positive = back into history).
    pub fn scroll_session(&mut self, session_id: &str, lines: i32, cx: &mut Context<Self>) {
        if lines == 0 {
            return;
        }
        let Some(session) = self.session_mut(session_id) else { return };
        session.term.scroll_display(lines);
        session.frame = session.term.snapshot();
        cx.notify();
    }

    // ----- selection ------------------------------------------------------

    /// Begin (or restart) a char selection at a viewport cell.
    pub fn begin_selection(&mut self, session_id: &str, row: u16, col: u16, cx: &mut Context<Self>) {
        let Some(session) = self.session_mut(session_id) else { return };
        session.term.begin_selection(row, col, SelectionMode::Char);
        session.frame = session.term.snapshot();
        cx.notify();
    }

    /// Extend the active selection to a viewport cell.
    pub fn extend_selection(&mut self, session_id: &str, row: u16, col: u16, cx: &mut Context<Self>) {
        let Some(session) = self.session_mut(session_id) else { return };
        session.term.extend_selection(row, col);
        session.frame = session.term.snapshot();
        cx.notify();
    }

    /// Drop the active selection.
    pub fn clear_selection(&mut self, session_id: &str, cx: &mut Context<Self>) {
        let Some(session) = self.session_mut(session_id) else { return };
        if session.term.has_selection() {
            session.term.clear_selection();
            session.frame = session.term.snapshot();
            cx.notify();
        }
    }

    /// The selected text (⌘C copies this).
    pub fn selection_text(&self, session_id: &str) -> Option<String> {
        self.session(session_id)?.term.selection_text()
    }

    /// Shift+arrow selection: anchor at the cursor on first press, then
    /// extend one cell per press.
    pub fn selection_key(&mut self, session_id: &str, key: &str, cx: &mut Context<Self>) {
        let Some(target) = self.shift_target(session_id, key) else { return };
        let Some(session) = self.session_mut(session_id) else { return };
        let already = session.term.has_selection();
        let (anchor_row, anchor_col) = session.frame.cursor.point.unwrap_or(target);
        if !already {
            session.term.begin_selection(anchor_row, anchor_col, SelectionMode::Char);
        }
        session.term.extend_selection(target.0, target.1);
        session.frame = session.term.snapshot();
        cx.notify();
    }

    /// The cell one shift+arrow step from the cursor, if the cursor exists.
    fn shift_target(&self, session_id: &str, key: &str) -> Option<(u16, u16)> {
        let session = self.session(session_id)?;
        let (row, col) = session.frame.cursor.point?;
        let cols = session.size.cols;
        let rows = session.size.rows;
        let target = match key {
            "left" => (row, col.saturating_sub(1)),
            "right" => (row, (col + 1).min(cols.saturating_sub(1))),
            "up" => (row.saturating_sub(1), col),
            "down" => ((row + 1).min(rows.saturating_sub(1)), col),
            _ => return None,
        };
        Some(target)
    }

    // ----- SFTP -----------------------------------------------------------

    /// List `path` over the session's SFTP channel (opens it on first use).
    pub fn sftp_list(&mut self, session_id: &str, path: String, cx: &mut Context<Self>) {
        let Some(session) = self.session_mut(session_id) else { return };
        if !session.is_live() {
            session.sftp.error = Some("session is not connected".to_owned());
            cx.notify();
            return;
        }
        session.sftp.path = path.clone();
        session.sftp.loading = true;
        session.sftp.error = None;
        let input = session.input.clone();
        let (reply_tx, reply_rx) = oneshot::channel();
        if input
            .try_send(SessionCommand::SftpList { path: path.clone(), reply: reply_tx })
            .is_err()
        {
            session.sftp.loading = false;
            session.sftp.error = Some("bridge unavailable".to_owned());
            cx.notify();
            return;
        }
        // gpui 0.2: `cx.spawn` futures must be `'static`; own the borrowed
        // `session_id` before it rides along with the reply receiver.
        let session_id = session_id.to_owned();
        cx.spawn(async move |this, cx| {
            let outcome = reply_rx
                .await
                .unwrap_or(Err("session closed before SFTP replied".to_owned()));
            this.update(cx, |state, cx| {
                state.apply_sftp_result(&session_id, path, outcome, cx)
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn apply_sftp_result(
        &mut self,
        session_id: &str,
        path: String,
        outcome: std::result::Result<Vec<SftpEntry>, String>,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session_mut(session_id) else { return };
        session.sftp.loading = false;
        if session.sftp.path != path {
            // A newer navigation superseded this listing.
            return;
        }
        match outcome {
            Ok(mut entries) => {
                // Dirs first, then files; both alphabetical (like Termius).
                entries.sort_by(|a, b| {
                    let a_dir = a.is_dir.unwrap_or(false);
                    let b_dir = b.is_dir.unwrap_or(false);
                    match (a_dir, b_dir) {
                        (left, right) if left == right => a.name.cmp(&b.name),
                        (true, false) => std::cmp::Ordering::Less,
                        (false, true) => std::cmp::Ordering::Greater,
                        // Unreachable: both arms cover equal/unequal bools.
                        _ => std::cmp::Ordering::Equal,
                    }
                });
                session.sftp.entries = entries;
                session.sftp.error = None;
            }
            Err(message) => {
                session.sftp.entries.clear();
                session.sftp.error = Some(message);
            }
        }
        cx.notify();
    }

    /// Enter a directory entry (`join_path` + list).
    pub fn sftp_open_entry(&mut self, session_id: &str, name: &str, cx: &mut Context<Self>) {
        let Some(session) = self.session(session_id) else { return };
        let is_dir = session
            .sftp
            .entries
            .iter()
            .find(|entry| entry.name == name)
            .and_then(|entry| entry.is_dir)
            .unwrap_or(false);
        if !is_dir {
            // PORT-TODO: file download / preview (Termius opens an editor).
            return;
        }
        let path = join_path(&session.sftp.path, name);
        self.sftp_list(session_id, path, cx);
    }

    /// Navigate to the parent directory.
    pub fn sftp_up(&mut self, session_id: &str, cx: &mut Context<Self>) {
        let Some(session) = self.session(session_id) else { return };
        let path = parent_path(&session.sftp.path);
        self.sftp_list(session_id, path, cx);
    }

    /// Re-list the current directory.
    pub fn sftp_refresh(&mut self, session_id: &str, cx: &mut Context<Self>) {
        let Some(session) = self.session(session_id) else { return };
        let path = session.sftp.path.clone();
        self.sftp_list(session_id, path, cx);
    }

    // ----- library CRUD ---------------------------------------------------
    //
    // Screens mutate the in-memory slice first (the UI always stays
    // responsive), then best-effort persist through the held store: a failed
    // write is logged and surfaced in `status_text`, never blocking the
    // render thread. Empty ids are minted with `fresh_id`.

    /// Best-effort persistence through the held store.
    fn persist<F>(&mut self, what: &str, op: F)
    where
        F: FnOnce(&Store) -> termius_storage::Result<()>,
    {
        let Some(store) = self.store.as_ref() else { return };
        if let Err(err) = op(store) {
            tracing::warn!(error = %err, record = what, "persist failed");
            self.status_text = format!("Save failed ({what}): {err}");
        }
    }

    /// Create a host (minting an id when the caller left it empty).
    pub fn add_host(&mut self, mut host: Host, cx: &mut Context<Self>) {
        if host.id.is_empty() {
            host.id = fresh_id("host");
        }
        self.persist("host", |store| store.hosts().create(&host).map(|_| ()));
        self.library.hosts.push(host);
        cx.notify();
    }

    /// Replace (upsert) the host with the same id.
    pub fn update_host(&mut self, mut host: Host, cx: &mut Context<Self>) {
        if host.id.is_empty() {
            host.id = fresh_id("host");
        }
        self.persist("host", |store| store.hosts().update(&host).map(|_| ()));
        match self.library.hosts.iter().position(|existing| existing.id == host.id) {
            Some(index) => self.library.hosts[index] = host,
            None => self.library.hosts.push(host),
        }
        cx.notify();
    }

    /// Delete a host, its orphaned forwards, and a matching selection.
    pub fn delete_host(&mut self, host_id: &str, cx: &mut Context<Self>) {
        self.persist("host", |store| store.hosts().delete(host_id).map(|_| ()));
        self.library.hosts.retain(|host| host.id != host_id);
        self.port_forwardings
            .retain(|forward| forward.host_id.as_deref() != Some(host_id));
        if self.selected_host.as_deref() == Some(host_id) {
            self.selected_host = None;
        }
        cx.notify();
    }

    /// Create a snippet (minting an id when the caller left it empty).
    pub fn add_snippet(&mut self, mut snippet: Snippet, cx: &mut Context<Self>) {
        if snippet.id.is_empty() {
            snippet.id = fresh_id("snippet");
        }
        self.persist("snippet", |store| store.snippets().create(&snippet).map(|_| ()));
        self.library.snippets.push(snippet);
        cx.notify();
    }

    /// Replace (upsert) the snippet with the same id.
    pub fn update_snippet(&mut self, mut snippet: Snippet, cx: &mut Context<Self>) {
        if snippet.id.is_empty() {
            snippet.id = fresh_id("snippet");
        }
        self.persist("snippet", |store| store.snippets().update(&snippet).map(|_| ()));
        match self
            .library
            .snippets
            .iter()
            .position(|existing| existing.id == snippet.id)
        {
            Some(index) => self.library.snippets[index] = snippet,
            None => self.library.snippets.push(snippet),
        }
        cx.notify();
    }

    /// Delete a snippet.
    pub fn delete_snippet(&mut self, snippet_id: &str, cx: &mut Context<Self>) {
        self.persist("snippet", |store| store.snippets().delete(snippet_id).map(|_| ()));
        self.library.snippets.retain(|snippet| snippet.id != snippet_id);
        cx.notify();
    }

    /// Create a key pair record (minting an id when the caller left it empty).
    pub fn add_key(&mut self, mut key: Key, cx: &mut Context<Self>) {
        if key.id.is_empty() {
            key.id = fresh_id("key");
        }
        self.persist("key", |store| store.keys().create(&key).map(|_| ()));
        self.library.keys.push(key);
        cx.notify();
    }

    /// Delete a key (and clear the binding on hosts that referenced it).
    pub fn delete_key(&mut self, key_id: &str, cx: &mut Context<Self>) {
        self.persist("key", |store| store.keys().delete(key_id).map(|_| ()));
        self.library.keys.retain(|key| key.id != key_id);
        for host in &mut self.library.hosts {
            if host.key_id.as_deref() == Some(key_id) {
                host.key_id = None;
            }
        }
        cx.notify();
    }

    /// Create a port-forwarding rule (minting an id when empty).
    pub fn add_port_forwarding(
        &mut self,
        mut forward: PortForwardingConfig,
        cx: &mut Context<Self>,
    ) {
        if forward.id.is_empty() {
            forward.id = fresh_id("forward");
        }
        self.persist("forward", |store| store.port_forwards().create(&forward).map(|_| ()));
        self.port_forwardings.push(forward);
        cx.notify();
    }

    /// Delete a port-forwarding rule.
    pub fn delete_port_forwarding(&mut self, forward_id: &str, cx: &mut Context<Self>) {
        self.persist("forward", |store| {
            store.port_forwards().delete(forward_id).map(|_| ())
        });
        self.port_forwardings
            .retain(|forward| forward.id != forward_id);
        cx.notify();
    }

    // ----- internals ------------------------------------------------------

    fn session_mut(&mut self, session_id: &str) -> Option<&mut Session> {
        self.sessions.iter_mut().find(|s| s.id == session_id)
    }
}

// ---------------------------------------------------------------------------
// Pure helpers (unit-tested without a window)
// ---------------------------------------------------------------------------

/// The tab to activate after closing `closed` from `order` (which still
/// contains it): keep the current tab unless it is the one closing, then step
/// right, then left, then none.
pub fn active_after_close(order: &[&str], active: Option<&str>, closed: &str) -> Option<String> {
    if active != Some(closed) {
        return active.map(str::to_owned);
    }
    let pos = order.iter().position(|id| *id == closed)?;
    order
        .get(pos + 1)
        .or_else(|| if pos > 0 { order.get(pos - 1) } else { None })
        .map(|id| (*id).to_owned())
}

/// The parent directory of a remote path (`/a/b` → `/a`, `/a` → `/`).
pub fn parent_path(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rsplit_once('/') {
        Some((parent, _)) if !parent.is_empty() => parent.to_owned(),
        _ => "/".to_owned(),
    }
}

/// A blank first frame matching `size` (tab shows something before output).
fn blank_frame(size: TerminalSize) -> GridSnapshot {
    let row = vec![StyledCell::blank(); size.cols as usize];
    GridSnapshot {
        cols: size.cols,
        rows: size.rows,
        cells: vec![row; size.rows as usize],
        cursor: Default::default(),
        alt_screen: false,
        generation: 0,
    }
}

/// Resolve `ConnectionParams` from a host plus its bound identity/key/keychain.
///
/// PORT-TODO: secrets may live in the OS keychain
/// (`termius_storage::get_host_passphrase` / `get_key_passphrase`) instead of
/// the record; consult both before falling back to `AuthMethod::None`.
fn params_for(host: &Host, library: &Library) -> ConnectionParams {
    let identity = host
        .identity_id
        .as_deref()
        .and_then(|id| library.identities.iter().find(|identity| identity.id == id));
    let mut username = host.username.clone();
    let mut auth = AuthMethod::None;

    if let Some(identity) = identity {
        if !identity.username.trim().is_empty() {
            username = identity.username.clone();
        }
        if let Some(password) = identity.password.as_ref().filter(|p| !p.is_empty()) {
            auth = AuthMethod::Password(password.clone());
        }
    }

    if matches!(auth, AuthMethod::None) {
        let key_id = host
            .key_id
            .as_deref()
            .or_else(|| identity.and_then(|identity| identity.key_id.as_deref()));
        if let Some(key) = key_id.and_then(|id| library.keys.iter().find(|key| key.id == id)) {
            if let Some(private_key) = key.private_key.as_ref().filter(|k| !k.is_empty()) {
                let passphrase = key
                    .keychain_id
                    .as_deref()
                    .and_then(|id| library.keychains.iter().find(|chain| chain.id == id))
                    .and_then(|chain| {
                        chain.entries.iter().find(|entry| entry.owner_id == key.id)
                    })
                    .map(|entry| entry.secret.clone());
                auth = AuthMethod::PrivateKey {
                    private_key: private_key.clone(),
                    passphrase,
                };
            }
        }
    }

    ConnectionParams {
        host: host.hostname.clone(),
        port: host.port,
        username,
        auth,
        ..ConnectionParams::default()
    }
}

// ---------------------------------------------------------------------------
// Background library load (sync storage, run off the render thread)
// ---------------------------------------------------------------------------

struct LoadedLibrary {
    store: Arc<Store>,
    library: Library,
    port_forwardings: Vec<PortForwardingConfig>,
    known_hosts: Vec<KnownHost>,
}

fn load_library() -> Result<LoadedLibrary> {
    let store = Store::open_default()?;
    let library = Library {
        hosts: store.hosts().list()?,
        groups: store.groups().list()?,
        snippets: store.snippets().list()?,
        identities: store.identities().list()?,
        keys: store.keys().list()?,
        keychains: store.keychains().list()?,
    };
    let port_forwardings = store.port_forwards().list()?;
    let known_hosts = store.known_hosts().list()?;
    Ok(LoadedLibrary {
        store: Arc::new(store),
        library,
        port_forwardings,
        known_hosts,
    })
}

// ---------------------------------------------------------------------------
// Engine runtime + bridge
// ---------------------------------------------------------------------------

/// The tokio runtime engine tasks run on.
///
/// Reuses the ambient runtime when the app host already entered one; otherwise
/// lazily builds a private multi-thread runtime (its IO driver keeps the
/// bridge's `russh` connections pumping independently of GPUI's executor).
fn engine_runtime() -> Result<tokio::runtime::Handle> {
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        return Ok(handle);
    }
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    if let Some(runtime) = RUNTIME.get() {
        return Ok(runtime.handle().clone());
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|err| UiError::message(format!("failed to start engine runtime: {err}")))?;
    let handle = runtime.handle().clone();
    let _installed = RUNTIME.set(runtime);
    match RUNTIME.get() {
        Some(runtime) => Ok(runtime.handle().clone()),
        // Another thread won the race with an identical runtime handle.
        None => Ok(handle),
    }
}

/// What the bridge loop learned this tick (borrows end before the handler
/// runs, so handlers may freely use `shell`/`client`).
enum BridgeTick {
    Output(Option<Vec<u8>>),
    Command(Option<SessionCommand>),
}

/// The engine half of one session: connect → shell → pump.
///
/// Owns `SshClient`, the shell `ChannelHandle` and the lazily opened
/// `SftpClient` for the whole session lifetime. Never touches GPUI state —
/// results leave through `events`.
async fn run_bridge(
    params: ConnectionParams,
    mut input: mpsc::Receiver<SessionCommand>,
    events: mpsc::Sender<BridgeEvent>,
) {
    let mut client = match Connector::new().connect(&params).await {
        Ok(client) => client,
        Err(err) => {
            tracing::error!(host = %params.host, error = %err, "connect failed");
            let _ = events.send(BridgeEvent::Failed(err.to_string())).await;
            return;
        }
    };
    let mut shell = match client.shell().await {
        Ok(shell) => shell,
        Err(err) => {
            tracing::error!(host = %params.host, error = %err, "shell channel failed");
            let _ = events.send(BridgeEvent::Failed(err.to_string())).await;
            let _ = client.disconnect().await;
            return;
        }
    };
    if events.send(BridgeEvent::Ready).await.is_err() {
        return;
    }

    let mut sftp: Option<SftpClient> = None;
    loop {
        // Futures are scoped to this `let`, so the handlers below can borrow
        // `shell` / `client` / `input` again.
        let tick = tokio::select! {
            chunk = shell.recv() => BridgeTick::Output(chunk.map(|bytes| bytes.to_vec())),
            command = input.recv() => BridgeTick::Command(command),
        };
        match tick {
            BridgeTick::Output(Some(bytes)) => {
                if events.send(BridgeEvent::Output(bytes)).await.is_err() {
                    return;
                }
            }
            BridgeTick::Output(None) => {
                let _ = events
                    .send(BridgeEvent::Closed { reason: "remote shell exited".to_owned() })
                    .await;
                let _ = client.disconnect().await;
                return;
            }
            BridgeTick::Command(None) => {
                // UI dropped the handle: last chance to close politely.
                let _ = shell.eof().await;
                let _ = client.disconnect().await;
                return;
            }
            BridgeTick::Command(Some(command)) => match command {
                SessionCommand::Input(bytes) => {
                    if let Err(err) = shell.write(&bytes).await {
                        tracing::debug!(error = %err, "shell write failed; closing session");
                        let _ = events
                            .send(BridgeEvent::Closed { reason: "channel closed".to_owned() })
                            .await;
                        let _ = client.disconnect().await;
                        return;
                    }
                }
                SessionCommand::Resize(size) => {
                    let _ = shell.resize(size).await;
                }
                SessionCommand::Disconnect => {
                    let _ = shell.eof().await;
                    let _ = client.disconnect().await;
                    let _ = events
                        .send(BridgeEvent::Closed { reason: "disconnected".to_owned() })
                        .await;
                    return;
                }
                SessionCommand::SftpList { path, reply } => {
                    if sftp.is_none() {
                        match client.sftp().await {
                            Ok(client) => sftp = Some(client),
                            Err(err) => {
                                let _ = reply.send(Err(err.to_string()));
                                continue;
                            }
                        }
                    }
                    let outcome = match &sftp {
                        Some(client) => client.list(&path).await.map_err(|err| err.to_string()),
                        None => Err("sftp unavailable".to_owned()),
                    };
                    let _ = reply.send(outcome);
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host_with(label: &str, identity: Option<&str>) -> Host {
        Host {
            label: label.to_owned(),
            hostname: format!("{label}.example.com"),
            username: label.to_owned(),
            identity_id: identity.map(str::to_owned),
            ..Host::default()
        }
    }

    #[test]
    fn close_keeps_other_tabs_and_walks_neighbours() {
        let order = ["a", "b", "c"];
        // Closing a background tab leaves the active one alone.
        assert_eq!(active_after_close(&order, Some("b"), "a"), Some("b".to_owned()));
        // Closing the active tab steps right…
        assert_eq!(active_after_close(&order, Some("b"), "b"), Some("c".to_owned()));
        // …or left at the end of the strip.
        assert_eq!(active_after_close(&order, Some("c"), "c"), Some("b".to_owned()));
        // Last tab closes to nothing.
        assert_eq!(active_after_close(&["solo"], Some("solo"), "solo"), None);
        // Unknown id keeps the current tab.
        assert_eq!(active_after_close(&order, Some("a"), "zz"), Some("a".to_owned()));
    }

    #[test]
    fn parent_path_climbs_to_root() {
        assert_eq!(parent_path("/home/dev"), "/home");
        assert_eq!(parent_path("/home/dev/"), "/home");
        assert_eq!(parent_path("/"), "/");
        assert_eq!(parent_path(""), "/");
        assert_eq!(parent_path("relative"), "/");
        assert_eq!(parent_path("/a"), "/");
    }

    #[test]
    fn params_resolve_identity_password() {
        let mut library = Library::default();
        let mut identity = Identity::default();
        identity.id = "id-1".into();
        identity.username = "deploy".into();
        identity.password = Some("s3cret".into());
        library.identities.push(identity);

        let host = host_with("web", Some("id-1"));
        let params = params_for(&host, &library);
        assert_eq!(params.host, "web.example.com");
        assert_eq!(params.port, 22);
        assert_eq!(params.username, "deploy");
        assert_eq!(params.auth, AuthMethod::Password("s3cret".into()));
    }

    #[test]
    fn params_fall_back_to_key_with_keychain_passphrase() {
        let mut library = Library::default();
        let mut key = Key::default();
        key.id = "key-1".into();
        key.private_key = Some("-----BEGIN OPENSSH PRIVATE KEY-----".into());
        key.keychain_id = Some("chain-1".into());
        library.keys.push(key);

        let mut chain = Keychain::default();
        chain.id = "chain-1".into();
        chain.entries.push(termius_core::keychain::KeychainEntry {
            owner_id: "key-1".into(),
            secret: "passphrase".into(),
        });
        library.keychains.push(chain);

        let mut host = host_with("db", None);
        host.key_id = Some("key-1".into());
        let params = params_for(&host, &library);
        assert_eq!(params.username, "db");
        assert_eq!(
            params.auth,
            AuthMethod::PrivateKey {
                private_key: "-----BEGIN OPENSSH PRIVATE KEY-----".into(),
                passphrase: Some("passphrase".into()),
            }
        );
    }

    #[test]
    fn params_without_credentials_stay_none() {
        let library = Library::default();
        let host = host_with("bare", None);
        let params = params_for(&host, &library);
        assert_eq!(params.auth, AuthMethod::None);
        assert_eq!(params.username, "bare");
    }

    #[test]
    fn blank_frame_matches_geometry() {
        let size = TerminalSize { cols: 4, rows: 2, width_px: None, height_px: None };
        let frame = blank_frame(size);
        assert_eq!(frame.cols, 4);
        assert_eq!(frame.rows, 2);
        assert_eq!(frame.cells.len(), 2);
        assert_eq!(frame.cells[0].len(), 4);
        assert!(!frame.alt_screen);
    }

    #[test]
    fn status_describe_and_live() {
        assert!(SessionStatus::Connecting.is_live());
        assert!(SessionStatus::Ready.is_live());
        assert!(!SessionStatus::Closed { reason: "x".into() }.is_live());
        assert!(!SessionStatus::Failed { message: "x".into() }.is_live());
        assert_eq!(SessionStatus::Ready.describe(), "connected");
        assert!(SessionStatus::Failed { message: "auth".into() }
            .describe()
            .contains("auth"));
    }

    #[test]
    fn dialogs_open_replace_and_close() {
        let mut active: Option<Dialog> = None;
        // Open: the empty host gains the dialog.
        active = dialog_after(active, DialogIntent::Open(Dialog::AddHost));
        assert_eq!(active, Some(Dialog::AddHost));
        // Opening again REPLACES — dialogs never stack.
        active = dialog_after(active, DialogIntent::Open(Dialog::EditHost("h1".into())));
        assert_eq!(active, Some(Dialog::EditHost("h1".into())));
        // Close clears…
        active = dialog_after(active, DialogIntent::Close);
        assert_eq!(active, None);
        // …and closing an already-closed host is a no-op.
        active = dialog_after(active, DialogIntent::Close);
        assert_eq!(active, None);
    }

    #[test]
    fn dialog_metadata_maps_to_sections() {
        assert_eq!(Dialog::AddHost.title(), "Add Host");
        assert_eq!(Dialog::AddSnippet.title(), "Add Snippet");
        assert_eq!(Dialog::AddKey.title(), "Add Key");
        assert_eq!(Dialog::AddPortForward.title(), "Add Port Forward");
        assert_eq!(Dialog::EditHost("h".into()).title(), "Edit Host");
        assert_eq!(Dialog::EditSnippet("s".into()).section(), Some(Section::Snippets));

        assert_eq!(Dialog::AddHost.section(), Some(Section::Hosts));
        assert_eq!(Dialog::AddPortForward.section(), Some(Section::PortForwarding));

        let confirm = Dialog::Confirm {
            title: "Delete host?".to_owned(),
            message: "web-1 will be removed.".to_owned(),
        };
        assert_eq!(confirm.title(), "Delete host?");
        assert_eq!(confirm.message(), Some("web-1 will be removed."));
        // Confirmations belong to no single section.
        assert_eq!(confirm.section(), None);
        assert_eq!(Dialog::AddKey.message(), None);
    }

    #[test]
    fn settings_and_account_defaults() {
        let settings = SettingsState::default();
        assert_eq!(settings.theme_mode, ThemeMode::Dark);
        assert_eq!(settings.font_family, "Menlo");
        assert!(settings.font_size > 0);
        assert!(settings.scrollback_lines > 0);
        assert!(settings.cursor_blink);
        assert!(settings.auto_reconnect);

        let account = AccountInfo::default();
        assert_eq!(account.plan, "Free");
        assert_eq!(account.device_count, 0);
        assert!(account.email.is_empty());
    }

    #[test]
    fn fresh_ids_are_prefixed_and_unique() {
        let first = fresh_id("host");
        let second = fresh_id("host");
        assert!(first.starts_with("host-"));
        assert!(second.starts_with("host-"));
        assert_ne!(first, second);
    }
}
