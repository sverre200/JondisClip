# ADR-0017: Clip titles are written into the file names

Status: **Proposed** (2026-11-06). This supersedes point 4 of ADR-0014 ("Titles are display names only. The file name never changes."). Everything else in ADR-0014 and ADR-0015 stands.

## Context
The owner renames clips in JondisClip and expects to see those names in Explorer, in file pickers (Discord's upload dialog) and anywhere the file goes. Until now only `organize.json` knew the title. The library also reads the game, the recording time and the Edit badge from the recorder's file name (`<Game> <YYYY-MM-DD HH-MM-SS>[ edit[ N]].mp4`), so a plain `<title>.mp4` would lose all three.

## Decision
- **The file name carries the title in front of the recorder name:** `<title> — <Game> <stamp>[ edit[ N]].mp4`, for example `Our Base Got Raided — DayZ 2026-11-04 14-35-41.mp4`. The separator is ` — ` (an em dash with spaces). Explorer sorts and shows the title first, and the game, time and Edit badge still come from the name.
- **The title becomes a safe file name** with the same rule as Discord shares (`title_to_file_stem`): no `<>:"/\|?*`, at most 80 characters, no reserved names. `organize.json` keeps the exact title (`Raid: day 2?`) for display.
- **A rename never overwrites.** It is a same-folder `MoveFileEx` without replace; on a clash the clip gets ` (2)`… If the file is in use, the title is still saved and the window says the file kept its old name.
- **Clearing a title** renames the file back to its recorder name. A file that never had a recorder name becomes `<title>.mp4` and keeps that name when the title is cleared.
- **What the window shows:** the stored title while the file name still matches it, otherwise the title in the file name (renamed in Explorer, or `organize.json` lost), otherwise the stored title (the rename failed).
- **Editing a titled clip** names the edit `<title> — <Game> <stamp> edit.mp4`, so the edit shows the title too.
- **Once, on the first library load after this change,** clips that already had a title are renamed (`titles_in_names` in `organize.json` records that this ran). A clip that is in use then keeps its old name until it is renamed again.

## Consequences
- Renaming changes the clip's path and id. The selection, order, folder, thumbnail and Drive mark follow it by content key, as with moves (ADR-0015).
- This is a rename the owner asks for, not an automatic change to an original (non-negotiable 3): the content is untouched. The one-time rename only applies titles the owner already chose.
- Game names containing ` — ` would be split wrongly. The recorder takes names from window titles; none seen so far contain it.
- **Verified 2026-11-06** (throwaway profile, release build): the stored title "Sniper from the tower" was written into its file name on first open; renaming an edit to `Truck: "flip" #2` gave `Truck flip #2 — DayZ 2026-11-05 13-02-00 edit.mp4` with game, date, Edit badge and keyboard focus kept; clearing it restored `DayZ 2026-11-05 13-02-00 edit.mp4`; a renamed clip opened and played. Unit tests cover parsing, clashes, clearing and Explorer renames.
