# ADR-0016: Accent themes

Status: **Proposed** (2026-11-05).

## Context
The owner wants colour themes like JondisBoard's, all dark, without losing the current look. Until now the record-red accent also meant "recording" and "danger". Swapping the accent alone would have turned the Edit badge, the recording pill and the Delete buttons purple or green.

## Decision
- **Six themes, accent only.** Crimson (default), Toxic Green, Midnight Purple, Ocean Blue, Sunset Orange and Graphite. Surfaces, text, spacing and type stay the same in every theme.
- **Crimson keeps the original values** (`#e5484d`), so the default look is unchanged. The requested `#FF3D5E` would have shifted it.
- **The other themes** set `--accent`, `--accent-hover`, `--accent-press`, `--accent-soft` and `--on-accent` in `:root[data-theme="…"]` blocks in `tokens.css`. `--focus` follows the accent.
- **Text on the accent** is dark (`#0c0d0f`) for the light accents: Toxic Green, Sunset Orange and Graphite (white would be 1.2–2.4:1). It's white for Midnight Purple (4.35:1) and Ocean Blue (3.3:1, comparable to Crimson's 3.9:1).
- **Status colours are separate tokens and never change:** `--success`, `--warning`, `--danger`, `--danger-soft`, `--on-danger`, `--recording` and `--recording-soft`/`--recording-glow`. The Edit badge and the live recording pill use them. Destructive dialog buttons use a new `danger` Button variant.
- **Storage.** `theme` in `settings.toml` (validated; unknown names fall back to crimson with a warning) is the source of truth. `localStorage` caches it so `main.ts` sets `data-theme` before the first render, while the window is still hidden. `settings.toml` is then read to confirm it.
- **Windows title bar.** Windows 11's DWM sets the caption colour to the window background (`#121316`) and the border to the accent: at startup and when the theme changes. Windows 10 ignores both and keeps the dark bar.
- **The recorder is unaffected.** The tray reads the same settings file but ignores `theme`.

## Consequences
- New components must use `--accent*` for interactive highlights and the status tokens for meaning. Never use a raw red.
- **Verified 2026-11-05:**
  - In the browser: all six themes.
  - In the release window (sandbox profile): a change is applied instantly and saved to `settings.toml`; after a restart the theme and the accent border come back; the Edit badge and the recording pill stay red; Delete stays red under Graphite.
- **A small flash is possible** only if `settings.toml`'s theme is edited by hand while the window is closed. The cached theme shows until the file is read.

## Update 2026-11-05: themed window icon
- **The window and taskbar icon follow the theme.** The record mark (`icons/mark128.rgba`, drawn in `#e5484d` on `#1a1c20`) is recoloured at runtime, keeping its anti-aliasing. It's set as both the small icon (`set_icon`) and the big icon (`WM_SETICON`/`ICON_BIG`, which the taskbar and Alt+Tab use).
- Verified by reading both icons back from the window handle: Ocean `#3b8cff`, and Toxic `#45e33f` after a change.
- **Still red:** the exe's own icon (Explorer, shortcuts, a pinned taskbar button before the window opens) and the tray.

## Update 2026-11-05: themes tint the whole window; no accent border
- **Owner request:** drop the bright accent-coloured window border and make each theme read as its colour throughout, as JondisBoard does.
- **Surfaces, borders and text now carry the theme's hue.** Each theme sets `--bg`, `--surface`, `--surface-raised/-hover/-press`, `--border`, `--border-strong` and `--text/-muted/-faint`. These come from JondisBoard's palettes (`rust/src/theme.rs`: bg, card, card hover, border, text, muted), with the in-between steps mixed the same way it mixes them. Crimson, the default, is a dark wine tint. The accents are unchanged.
- **Unchanged:** status colours (success, warning, danger, recording), the red Edit badge, the blue Drive badge and the folder colours are the same in every theme.
- **Title bar:** the caption takes the theme's `--bg` (`theme_bg` in `main.rs`), so the bar and the page are one surface. The border is `DWMWA_COLOR_NONE`, so there is no coloured outline.
- **Contrast (WCAG):** text is 14.0–14.9:1 on cards in every theme. Muted text is 4.9–5.7:1 on cards and 5.7–6.4:1 on the background. Faint text is 3.3–3.7:1, used for hints only, as before.
- Checked in the browser on the library, Settings, a dialog and the context menu across the themes.
