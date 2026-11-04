# ADR-0003: Capture via Windows Graphics Capture; no game hooking

Status: **Proposed** (2026-11-04)

## Options
- **Windows Graphics Capture (WGC):** an OS-supported API with D3D11 textures and low overhead. It handles windowed, borderless and most modern fullscreen (flip-model/FSO) games. Windows 11 can remove the border and cap the update rate.
- **DXGI Desktop Duplication:** monitor-only and mature. A good fallback for older Windows builds and edge cases.
- **Game hook / DLL injection** (OBS game capture): the best performance and true exclusive-fullscreen support, but **anti-cheat risk** (BattlEye in DayZ, EAC, Vanguard) and significant maintenance. OBS's hook is tolerated because it's signed and widely allow-listed. Ours would not be.

## Decision
WGC monitor capture is the default. WGC window capture is an advanced option, and Desktop Duplication is the fallback. No injection.

## Verified
On the dev machine, `ffmpeg ddagrab` (Desktop Duplication, D3D11 frames) piped straight into `hevc_amf` worked with zero CPU copies (2026-11-04). This confirms the zero-copy path on AMD.

## Open risks
- WGC behaviour in exclusive-fullscreen titles, high-refresh rates (144+) and HDR: test in Phase 2 with DayZ and two other titles.
- HDR capture needs a tone-mapping pass or a 10-bit HEVC path, deferred to a later phase.
