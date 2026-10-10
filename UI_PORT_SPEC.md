# termius-ui 1:1 port — implementer spec (read fully before editing)

You are porting the **original Termius v10 desktop UI** (an Electron/React app) to
**GPUI 0.2.2** in the Rust crate `crates/termius-ui`. The goal is *visual parity*:
same layout, icons, colours, typography, spacing — driven by the original's own
design tokens and the recovered component sources.

## Ground truth — READ THESE, do not guess

| What | Where |
|---|---|
| Exact gpui API (authoritative) | `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/gpui-0.2.2/src/` (esp. `styled.rs`, `elements/`, `style.rs`, `text_system.rs`) |
| Original component sources (rollup ESM) | `analysis/termius-extracted/ui-process/assets/<Name>-<hash>.js` |
| Beautified copies of key components | `analysis/readable/*.js` (regenerate any file with `analysis/tools/wakaru <src> -o <out>`) |
| Whole-app bundle (shell lives here) | `analysis/readable/_main.js` (beautified `ui-process-a9c01aa6.js`) |
| Design tokens (CSS vars, dark+light) | `analysis/termius-theme.json`, `analysis/termius-extracted/ui-process/assets/reconnectSaga-e37b5571.css` |
| Typography scale | `analysis/readable/_main.js` + `reconnectSaga-f0db0c3c.js` (`typography.fonts.*`) — **already ported** to `theme::text` |
| Icons (464 SVG, embedded) | `crates/termius-ui/assets/icons/*.svg` → use `termius_ui::icon("<name>.svg")` |
| Fonts | CircularXX, embedded; family name is `CircularXX` (const `termius_ui::UI_FONT`) |

### Already built (use it, don't rebuild)
- `termius_ui::icon(name)` → `gpui::Svg`, tinted by the element's `text_color`.
  Size it with `.w(px(..)).h(px(..))`. Icon names: `ls crates/termius-ui/assets/icons`
  (e.g. `addCircle.svg`, `lock.svg`, `snippet.svg`, `vault.svg`, `sftp.svg`,
  `settings_gear.svg`, `bell.svg`, `search.svg`, `host.svg`, `key.svg`).
- `termius_ui::assets::{TermiusAssets, load_fonts, ICON_NAMES, has_icon}`.
- `theme::text::{R12P, B12P, R14P, B14P, R16P?…}` → `TextToken`; apply with
  `token.style(div()).text_color(color)`. `TextToken` also has `.uppercase`.
- `TermiusTheme` (dark/light) in `theme.rs` — colours come from here, never literals.

## Hard rules
- **No local cargo/git.** No `unwrap/expect/panic!` outside `#[cfg(test)]`.
- Only edit the files your task names. Never touch another agent's files.
- Preserve every `pub` signature other files depend on unless your task says
  otherwise. If you must change one, list the callers you updated.
- Prefer `div().flex()...` composition; no absolute positioning unless the
  original does. Spacing values come from the original CSS (px).
- Keep the code compiling: check every gpui method against the 0.2.2 source.

## The original shell (target structure)

From the running original app and `analysis/readable/_main.js`
(`ConnectedAppLayout`, `ConnectedTwoPanelMode`, top tab strip around line
110460–110560; `iconType: "vault-tab" | "sftp" | "hosts" | …`):

```
┌─ window (frameless-ish, dark) ───────────────────────────────────────────┐
│ TOP BAR (h≈44)                                                           │
│   traffic-light spacer | page tabs: ▣Hosts ▤Vaults ⇄SFTP … | connection   │
│   tabs (title, close ×) | ＋ |            right: Update · 🔔 · ⚙        │
│ ─────────────────────────────────────────────────────────────────────── │
│ LEFT PANEL (w≈240–260)                    │ MAIN AREA                     │
│   search + filter header row              │  terminal(s) OR               │
│   host/list rows (grouped)                │  centred empty state          │
│                                           │                               │
└──────────────────────────────────────────────────────────────────────────┘
```

- Top bar: page tabs are **icon+label** buttons (`iconType`); the active page tab
  is highlighted; connection tabs sit beside them with a close × and a trailing ＋.
  Right cluster: "Update" pill, notification bell, settings gear.
- Left panel: a **search field** row and a **filter/sort** control, then the list.
- Main area empty state: centred illustration + text ("Select a host…").

The prior port was wrong: a vertical text sidebar with **emoji** icons, a fake
"status bar" ("Hosts ⌘B", "☾ dark"), and no top bar. Replace it.

## Deliverable / acceptance
- All icons are real SVGs from `termius_ui::icon`, never emoji/unicode.
- All text uses `theme::text::*` tokens + `TermiusTheme` colours.
- Layout matches the diagram above.
- `cargo test -p termius-ui` logic unchanged; public API of untouched modules stable.
