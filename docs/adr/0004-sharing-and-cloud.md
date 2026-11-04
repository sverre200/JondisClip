# ADR-0004: Discord via clipboard and size-targeted export; Drive via sync folder first

Status: **Proposed** (2026-11-04)

## Discord
- There's no official API for uploading as a user. Bots and user tokens are off the table (insecure and against the terms of service).
- **Copy clip** puts the file on the clipboard (`CF_HDROP`), so the user pastes it into a Discord chat to upload it.
- **Export for Discord** re-encodes to a target size. As of Aug 2026, reports say the free limit is 20 MB (it was 10 MB from 2024), Nitro Basic is 50 MB and Nitro is 500 MB. The presets are editable because Discord changes these limits.

## Google Drive
- **v1 (Option A):** the user points the save folder at a Google Drive for desktop folder. That needs zero code, zero OAuth and zero overhead, and Drive's client handles retries and de-duplication. We'll add a hint in Settings, and Google Drive can be told to pause syncing while gaming.
- **Later (Option B, Phase 5, only if still wanted):** the `drive.file` scope (non-sensitive, so it can only see files the app created), OAuth with PKCE via a loopback redirect, refresh tokens in Credential Manager, and a resumable upload queue in SQLite. It would never upload while a game is in the foreground unless the user allows it, and it would never delete local files.
