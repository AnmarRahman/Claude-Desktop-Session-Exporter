# Claude Session Exporter - Mac Handoff Notes

This file is a handoff for continuing the existing Claude Session Exporter project on another machine.

## Project Goal

Claude Session Exporter is a production-quality desktop app built with Tauri, React, TypeScript, and Rust. Its goal is to export the currently open Claude Desktop conversation, including:

- conversation title
- user messages
- Claude responses
- code blocks
- tables
- images or file-card metadata where available
- tool / cowork activity where available

The app is currently focused on Claude Desktop. Windows support has active implementation work. macOS support is still mostly scaffolded.

## Current Workspace

Original Windows path:

```text
C:\Users\Anmar Abdelrahman\Desktop\CE\Claude Extractor
```

Important generated folders are intentionally ignored:

```text
src-tauri/diagnostics/
src-tauri/exports/
```

## How To Run

On macOS:

```bash
npm install
npm run tauri dev
```

Useful checks:

```bash
npm test
npm run build
cd src-tauri && cargo test
```

On Windows, the equivalent commands used during development were:

```powershell
npm.cmd run tauri dev
npm.cmd test
npm.cmd run build
cd src-tauri
cargo test
```

Latest known verification before this handoff:

- `cargo test` passed with 18 tests.
- `npm.cmd run build` passed.
- Earlier frontend tests also passed with `npm.cmd test`.

## Current State Summary

The Home/Cowork blocker described in the original handoff is **resolved**. The
transcripts were never in Local Storage or IndexedDB — they are in Claude
Desktop's Chromium **HTTP disk cache**, as the JSON response to
`chat_conversations/<uuid>?tree=True&rendering_mode=messages`. See
[docs/WEB_CACHE.md](docs/WEB_CACHE.md).

Verified on macOS, 2026-08-11: 10,690 cache entries indexed, 43 conversations
recovered, one 242-message conversation exported end to end with tool activity
and thinking intact.

Both platforms now share one reader, `src-tauri/src/capture/web_cache/`. It
implements Chromium's simple-cache format, verified against a real macOS profile.
Chromium's other HTTP backend (blockfile) is **not** implemented; whether Windows
Claude uses it has not been checked, so the reader detects the backend and says
so rather than reporting an empty cache. The old string-scanning
`capture/windows/web_cache.rs` was removed.

## What Works

### Home/Cowork transcript export (verified on macOS)

`src-tauri/src/capture/web_cache/` — indexes the HTTP cache by entry key,
deduplicates by conversation UUID keeping the newest, decodes the zstd/gzip/br
body, normalizes `chat_messages` into ordered blocks, and writes Markdown + JSON
to `src-tauri/exports/`.

Export options: `source`, `conversation_id`, `include_thinking` (default off),
`include_tools` (default on).

### Cowork and Claude Code export (all platforms)

`src-tauri/src/capture/transcript.rs`, promoted out of `windows/` and made
platform-neutral. It reads two stores of the same JSONL format:
`local-agent-mode-sessions` (Cowork — the sessions in Claude Desktop's Home
sidebar) and `claude-code-sessions` (Claude Code). A Cowork session keeps its
transcript in its own `.claude` home:
`local_<id>/.claude/projects/<sanitized-cwd>/<cliSessionId>.jsonl`.

Verified on macOS by exporting a 211-message Cowork session.

### Claude detection

Windows: `capture/claude/detection.rs`, `capture/windows/{process,uia}.rs`,
unchanged. macOS: a process scan for the `/Claude.app/Contents/MacOS/` bundle
executable. The export button is gated on detection, so without this the macOS
export was unreachable from the UI.

### Source selector

Unchanged behavior, now backed by the shared reader. `auto` still refuses to fall
back to a stale Claude Code session while Claude Desktop reports Home/Chat mode,
using `lastKnownMode` from the `dframe-store` Local Storage blob — which the
shared module reads on macOS too.

## Known Limits

The HTTP cache is best-effort. A conversation never opened on this machine is not
cached, Chromium evicts entries under pressure, and a cached copy is only as
fresh as the last time the conversation was opened. If that proves too lossy, the
fallback is an authenticated refetch using the local session cookie, which would
require Keychain-decrypting `Cookies`. Not attempted.

## Next Steps

1. **Conversation picker UI.** Now the top priority: Cowork sessions and Home
   chats live in separate stores whose timestamps update on different triggers,
   so nothing on disk reliably says which session is on screen. Letting the user
   pick removes the guesswork entirely.
1. **(was) Conversation picker UI.** The backend already lists every cached
   conversation with real titles and accepts `conversation_id`; the frontend
   still only exports the most recent one.
2. **Port Claude Code export to macOS** — the data is at `~/.claude/projects` and
   `~/Library/Application Support/Claude/claude-code-sessions`.
3. **HTML and PDF rendering** from the normalized model.
4. **Reconsider gating export on detection** — export reads the cache and works
   whether or not Claude Desktop is running.

## Files To Read First

Start with these:

```text
docs/WEB_CACHE.md
src-tauri/src/capture/web_cache/mod.rs
src-tauri/src/capture/web_cache/conversation.rs
src-tauri/src/capture/windows/transcript.rs
src-tauri/src/models.rs
src/App.tsx
src/types.ts
```

Then read these for detection and diagnostics:

```text
src-tauri/src/capture/claude/detection.rs
src-tauri/src/capture/windows/process.rs
src-tauri/src/capture/windows/uia.rs
src-tauri/src/capture/windows/visual.rs
```

macOS adapter:

```text
src-tauri/src/capture/macos.rs
```

## Key UX Requirement

Do not let the app export a Claude Code transcript when the user has a regular Claude Home/Cowork chat open.

If the Home/Cowork transcript cannot be found, show a clear failure and explain which source paths were checked. Do not silently fall back to Claude Code.

## Verification Status

- `cargo test` — 41 pass, plus the ignored real-profile test which passes on demand.
- `cargo check --target x86_64-pc-windows-msvc` — the Windows-only modules
  type-check. The full Windows build could not be run here: `tauri-winres` needs
  `llvm-rc`, which is not installed on this Mac.
- The Windows Chromium cache backend is unconfirmed. If Claude Desktop on Windows
  uses the blockfile backend, Home/Cowork export will fail there with a message
  naming the backend, and a blockfile reader would be needed.
- `npm test` — 5 pass. `npm run build` — clean.
- The Windows *runtime* path has not been exercised since the refactor.

