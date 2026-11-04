# ADR-0001: Rust engine + Tauri 2 UI, FFmpeg (LGPL) for encode/mux

Status: **Accepted** (2026-11-04, approved by owner)

## Context
The resident process runs during every gaming session, so its CPU, RAM and GPU cost matters more than development speed. Capture, encoding and audio all have to use native Windows APIs.

## Options considered
| Option | Resident cost | Capture/encode control | Licence | Dev cost | Verdict |
|---|---|---|---|---|---|
| **Rust core + Tauri 2 (WebView2 UI on demand)** | Low: the core is native, and the UI is destroyed when closed | Full (windows-rs, FFmpeg C API) | MIT/Apache | Medium-high | **Chosen** |
| Embed libobs (OBS core) | Medium. Its replay buffer keeps encoded packets **in RAM** (~1.5 GB for 5 min at 40 Mb/s) | High, and proven | **GPL-2.0 for the whole app** | Medium: large, complex build | Rejected for v1. We'd reconsider if our own pipeline proves unreliable |
| C# / .NET + WinUI 3 | Medium (GC, ~60–100 MB baseline) | Good, through interop | MIT | Low-medium | A viable runner-up, with weaker hot-path guarantees |
| C++ + Qt/WinUI | Lowest | Full | Qt is LGPL/commercial | High; memory-safety burden | Rejected |
| Electron | High (~150–300 MB resident) | Needs a native add-on anyway | MIT | Low | Rejected |

## Decision
- The engine, tray and hotkeys are **Rust** (`windows` crate for WGC, D3D11, WASAPI and Win32).
- Encoding and muxing use **FFmpeg libavcodec/libavformat 7.x, LGPL shared build**, dynamically linked. It provides AMF, NVENC and QSV through one API and accepts D3D11 hardware frames, so there's no CPU copy. We don't use x264/x265 (GPL).
- The UI is **Tauri 2** with a TypeScript frontend. The window is created on demand and destroyed on close.
- Storage is **SQLite** (`rusqlite`, bundled) for the library index, plus TOML settings.

## Consequences
- We have to ship the FFmpeg DLLs and comply with the LGPL: dynamic linking, offering the source, and publishing our build config. The dev build (BtbN `n9.0 lgpl-shared`) is **LGPL v3** because it includes libvpl (`--enable-version3`). It has no GPL or non-free parts (checked 2026-11-04).
- AMF and QSV via FFmpeg are less tunable than the vendor SDKs. If quality or latency falls short, an ADR can move one vendor to its native SDK (the AMF SDK is MIT).
- Local verification covers AMD only. NVIDIA and Intel need test hardware or testers.
