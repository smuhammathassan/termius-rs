# Termius → Rust (GPUI) Port Plan

> **For agentic workers:** This plan is executed by parallel sub-agent waves. Each agent owns whole crate(s); never edit another agent's crate or the root `Cargo.toml`/CI. Steps use checkbox syntax.

**Goal:** 1:1 behavior port of the Termius desktop SSH client (Electron app v10.1.3) to a native Rust application using GPUI (Zed's UI framework) — no Electron, no webview.

**Architecture:** Rust workspace of engine crates (ssh/telnet/serial/terminal/storage/sync) + a GPUI UI crate + an app binary that replaces the Electron main/background/renderer process split with in-process async tasks. Dependencies flow inward: engines → core; ui/app → engines.

**Tech Stack:** Rust stable, GPUI `0.2` (native UI), `russh` (SSH2 — replaces libssh2 native addon `@termius/libtermius`), `alacritty_terminal` (VT emulation — replaces xterm.js), `rusqlite` (local DB — replaces schema.js), `keyring` (OS keychain — replaces keytar), `serialport`, `reqwest` (sync/cloud).

**Spec (source of truth):** REA analysis of the Termius.app bundle:
- Recovered sources: `/Users/muhammadhassan/termius/analysis/recovered-{main,background,ui}/*.js` (deobfuscated webpack modules, byte-range provenance in `provenance.json`)
- Extracted app: `/Users/muhammadhassan/termius/analysis/termius-extracted/` (main-process.js, background-process, ui-process/assets, utilities-process, package.json deps)
- Evidence summary: `/Users/muhammadhassan/termius/analysis/termius-summary.json`, `arch-nodes.json`
- Full application graph: `termius-js-app.json` (1.2 GB — do NOT fully parse; grep/brace-scope if needed)

## Key substitutions (Electron → Rust)

| Termius (Electron) | Rust port |
|---|---|
| `@termius/libtermius` (libssh2 .node) | `russh` + `russh-sftp` (termius-ssh) |
| `@termius/node-pty` | russh PTY channel / serialport (termius-ssh/serial) |
| xterm.js | `alacritty_terminal` (termius-terminal) |
| React/Redux renderer | GPUI views + app state (termius-ui) |
| main-process lifecycle | app binary + tokio runtime (termius-app) |
| background-process (conn engine) | in-process tokio tasks (engines) |
| `@termius/keytar` | `keyring` (termius-storage) |
| `schema.js` + repositories | `rusqlite` + migrations (termius-storage) |
| `serialport-bindings` | `serialport` (termius-serial) |
| JS telnet transport | tokio TCP + IAC (termius-telnet) |
| `@termius/mosh` | shell-out / deferred (termius-mosh) |
| sync saga / API | `reqwest` client (termius-sync) |
| `@azure/*` SDK | deferred (termius-cloud) |
| `@termius/libfido2` | deferred (termius-fido) |

## Known evidence (from REA)

- **Processes:** main (windows/menus/updater/IPC hub/keychain), background (conn engine: telnet/mosh/sftp + config repositories), utilities (`schema.js` crypto/zlib), renderer (Redux UI).
- **Native addons:** keytar, moshclient, serialport-bindings, mac-single-sign-in, restore-mas-purchase, minidump-parser, registry-js, libfido2, libtermius (libssh2).
- **IPC channels (42):** `init-redux`, `redux-action`, `open-connections-in-new-window`, `check-for-update`, `update-restart`, `update-system-menu`, `menu:*`, `dock:*`, `dialog.showOpenDialog/SaveDialog`, `open-external`, `wasAppOpenedWithDeeplink`, `Zi.ADD_METRIC`.
- **Storage surfaces:** local-storage, indexed-db, session-storage, `deviceToken`.
- **SSH API surface (JS→native):** `connect(options)`, `auth(...)`, `exec(cmd, cb)`, `write`, `read`, `resize(rows,cols)`, `sftp()`, `disconnect()`.

## Crate ownership map

| Wave | Crate | Spec reference |
|---|---|---|
| W1 | `termius-core` | domain models — infer from storage keys, IPC payloads, recovered models |
| W1 | `termius-storage` | schema.js, repositories, keytar usage in recovered-main |
| W1 | `termius-ssh` | libtermius API + ssh2 usage in recovered-background/entry.js |
| W1 | `termius-telnet` | telnet/telnet_config in recovered-background |
| W1 | `termius-serial` | serialport in recovered-background + UI serial terminal |
| W1 | `termius-terminal` | xterm.js usage in recovered-ui + shell-integration |
| W2 | `termius-sync` | sync/saga in recovered-main + deviceToken |
| W2 | `termius-ui` | Redux views/components in recovered-ui |
| W2 | `termius-app` | main-process.js + background + window/menu/dock |
| W3 | `termius-mosh`, `termius-cloud`, `termius-fido` | experimental/deferred |

## Global constraints

- **No local cargo builds** — CI (`.github/workflows/ci.yml`) is the build/test gate: `core` job on Ubuntu (excludes gpui crates), `app` job on macOS (full incl. gpui). Push to trigger.
- **Crate ownership is exclusive.** Pre-created stubs: only edit files under `crates/<your-crate>/`.
- **Do not edit** root `Cargo.toml`, `.github/`, other crates. If you need a new shared dep, note it in your report (coordinator adds it).
- Rust idioms: `snake_case` fns/modules, `PascalCase` types, `#[serde(rename_all = "camelCase")]` on wire/domain types to match Termius' JS field names.
- Errors: `thiserror` per crate, `TermiusError`-style enums; preserve failure reasons (auth vs network vs protocol vs permission).
- Async: `tokio` + `async-trait` for engine traits; streams via `futures`.
- No `unwrap()`/`expect()`/`panic!` outside `#[cfg(test)]`.
- Port behavior 1:1 from the recovered JS; where the native addon hides logic, reimplement with the equivalent Rust crate.

## Progress log
- 2026-10-10: workspace scaffold + 12 crate stubs + CI + plan created; repo `termius-rs` created; wave 1 dispatched.
- 2026-10-10: all 12 crates implemented by parallel agents (core, storage, ssh, telnet, serial, terminal, sync, mosh, cloud, fido, ui, app) — ~28k lines.
- 2026-10-10: dependency/feature fixes (reqwest `rustls`+`query`, keyring 3.x `apple-native`).
- 2026-10-10: fixed all 10 core/engines crates against real APIs — russh 0.64 (connect_stream, Handler::check_server_key, Channel::exec/wait/window_change, russh_sftp::SftpSession), alacritty_terminal 0.26 (Processor, index::Point, vte::ansi::Color/CursorShape, Flags::STRIKEOUT, TermMode::ALT_SCREEN), reqwest `.query`, keyring error variants. **All 20 core test suites pass (0 failures).**
- In progress: GPUI 0.2 compile of `termius-ui`/`termius-app` (CI macOS job + local check).
- 2026-10-10: fixed `termius-ui` against real gpui 0.2.2 (InteractiveElement::id, rgb arity, FocusHandle::focus, observe/update_global via BorrowAppContext, spawn lifetimes). **CI fully green: core(linux) ✓, app(macos incl gpui) ✓, clippy ✓.** 12 crates, ~19.2k LOC, 231 tests passing, 0 failing.

