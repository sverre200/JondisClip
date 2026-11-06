# ADR-0018: Language (English, bokmål, nynorsk) and date format

Status: **Proposed** (2026-11-06).

## Context
The owner and their friends are Norwegian. They want the app in Norwegian Bokmål or Nynorsk. They also want dates in the order they use: the app showed American-style dates ("Nov 6"), and new clip files were always named `YYYY-MM-DD`.

## Decision
- **Two settings in `settings.toml`, validated like the others:**
  - `language`: `en` (default), `nb` or `nn`. Unknown values fall back to `en` with a warning.
  - `date_format`: `ymd` (default, 2026-11-06), `dmy` (06.11.2026) or `mdy` (11/06/2026). Unknown values fall back to `ymd`.
  - Both are under Settings → Language and dates. They apply to the window at once; the tray follows within 2 s, because it re-reads the file.
- **Window strings.** Components write English and wrap it in `t("…", { vars })` or `tn(n, one, other)` (`ui/src/lib/i18n.svelte.ts`).
  - The English text is the key into `ui/src/lib/i18n/nb.ts` and `nn.ts`.
  - `t()` reads reactive state, so switching language re-renders without a reload.
  - `npm run check` runs `scripts/i18n-check.mjs`. It lists every key and fails on a missing translation, a changed `{placeholder}` or an unused entry.
- **Rust messages** (tray menu, tooltip and notifications, errors the window shows, mic-check tips, exporter errors) use `jondisclip_core::tr!("…", name = value)`.
  - The text is looked up in `crates/core/src/i18n_table.rs`, which holds English, bokmål and nynorsk side by side.
  - Each process sets its language from the settings: the tray on load and change, the window on every settings read, the exporter at start.
  - A unit test scans the workspace for `tr!` and fails on gaps; an integration test checks Norwegian output.
- **Not translated:**
  - Logs stay English.
  - Low-level error details stay as they are: FFmpeg, MP4 parse reasons cached in the library index, and OS error text (Windows already localises its own).
  - Product names stay as they are: Discord, Google Drive, H.264.
- **Dates in the app:**
  - Cards: `06.11.` (`dmy`), `11/06` (`mdy`) or `2026-11-06` (`ymd`), with the year when it isn't this year.
  - Day headings: "I dag" / "I går", or the weekday and full date ("Onsdag, 04.11.2026").
  - Weekday names come from `Intl` with `nb-NO` or `nn-NO`.
- **Dates in new file names** follow `date_format`: `DayZ 06.11.2026 21-03-11.mp4`. The time always uses dashes, because `:` isn't allowed in Windows file names.
  - The library reads all three formats. They can't be confused, because the separators differ: `-` with the year first, `.` with day first, `-` with the year last.
  - Every format is 19 characters, so the title rule (ADR-0017) and the Edit suffix work unchanged.
  - Existing files keep their names; nothing is renamed automatically.

## Consequences
- New user-visible strings must go through `t()` or `tr!`. The checks catch a key without a translation, but not English that was never wrapped, so review new components for that.
- `mdy` file names are `11-06-2026` (`/` isn't allowed). They're told apart from `ymd` by the 4-digit year's position.
- Translations were written for this change, without a native review. The owner should flag awkward wording.
- **Verified 2026-11-06:**
  - Unit tests: all three stamp formats parse back to the same time, including titled and edit names.
  - The Rust table is complete (147 messages), and the Norwegian integration test passes.
  - `npm run check` passes with 343 window strings, complete in both languages.
  - Browser preview: switching to bokmål re-renders the library and settings live, and `dmy` dates show on cards and in headings.
