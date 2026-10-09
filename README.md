# termius-rs

A native Rust port of the [Termius](https://termius.com/) desktop SSH client — built on [GPUI](https://gpui.rs/), Zed's GPU-accelerated UI framework. No Electron, no webview.

The port is derived from a static reverse-engineering analysis of the Termius 10.1.3 Electron app (see `PORTING.md` for the evidence and crate map).

## Architecture

| Crate | Role |
|---|---|
| `termius-core` | Domain models (hosts, identities, port-forwarding, snippets, keychains, …) |
| `termius-storage` | SQLite persistence + OS keychain (replaces schema.js + keytar) |
| `termius-ssh` | SSH engine over `russh` (replaces libssh2 native addon) |
| `termius-telnet` | Telnet engine (tokio TCP + IAC) |
| `termius-serial` | Serial-port engine (`serialport`) |
| `termius-terminal` | VT emulation (`alacritty_terminal`, replaces xterm.js) |
| `termius-sync` | Termius cloud sync client (`reqwest`) |
| `termius-mosh` / `termius-cloud` / `termius-fido` | Experimental / deferred |
| `termius-ui` | GPUI views (replaces React/Redux renderer) |
| `termius-app` | The desktop binary (window/menu/lifecycle, replaces Electron main+background) |

## Build & test

All builds run in GitHub Actions (`.github/workflows/ci.yml`):

- **core (linux):** `cargo check/test --workspace` excluding the GPUI crates
- **app (macos):** full workspace including GPUI

```
cargo check --workspace --all-targets
cargo test --workspace
```

## Status

Port in progress. See `PORTING.md` for the plan and progress log.
