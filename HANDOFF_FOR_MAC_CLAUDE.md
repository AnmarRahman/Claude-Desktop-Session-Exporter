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

The app can now detect Claude Desktop and export Claude Code transcripts from local Claude data. This is working.

The current blocker is regular Claude Home / Cowork chat export. The app can sometimes correctly detect that Claude Desktop is in Home/Chat mode, but exporting the transcript fails with this message:

```text
Diagnostic snapshot failed. No regular Claude Home / Cowork chat transcript was found in the local cache. Open the chat in Claude Desktop, wait for it to load, and then retry.
```

This means the app knows Claude is in a regular Home/Cowork chat context, but the current cache reader cannot find the actual chat transcript in the local Claude Desktop cache.

## What Works

### Claude Code transcript export

Implemented in:

```text
src-tauri/src/capture/windows/transcript.rs
```

It reads Claude Desktop code-session metadata from locations like:

```text
%LOCALAPPDATA%\Packages\Claude_*\LocalCache\Roaming\Claude\claude-code-sessions
%APPDATA%\Claude\claude-code-sessions
```

Then it maps `cliSessionId` and cwd to:

```text
%USERPROFILE%\.claude\projects\<sanitized-cwd>\<cliSessionId>.jsonl
```

It parses the JSONL transcript and exports Markdown and JSON into:

```text
src-tauri/exports/
```

The parser intentionally keeps user prompts and assistant text while filtering noisy tool-result data.

### Claude window detection

Relevant files:

```text
src-tauri/src/capture/claude/detection.rs
src-tauri/src/capture/windows/process.rs
src-tauri/src/capture/windows/uia.rs
src-tauri/src/capture/windows/mod.rs
```

The Windows detector now avoids many false positives, including:

- the exporter app itself
- dev/editor windows
- Claude Code helper process paths
- stale Claude Code fallback when the current Claude shell mode says Home/Chat

### Source selector

The UI now has a source selector:

```text
Auto
Home / Cowork
Claude Code
```

Relevant files:

```text
src/App.tsx
src/types.ts
src/styles.css
src-tauri/src/models.rs
src-tauri/src/capture/windows/mod.rs
```

Important intended behavior:

- `Claude Code` should export only Claude Code transcripts.
- `Home / Cowork` should export only regular Claude Home/Cowork chats.
- `Auto` should not silently export a stale Claude Code session when Claude Desktop is currently in Home/Chat mode.

### Visible screenshot capture

Implemented in:

```text
src-tauri/src/capture/windows/visual.rs
```

It uses Win32 GDI capture and saves a BMP diagnostic image into:

```text
src-tauri/diagnostics/
```

This proved useful as a fallback diagnostic path, but it is not a full transcript export solution.

### UIA inspector

Relevant files:

```text
src-tauri/src/capture/windows/uia.rs
src/App.tsx
```

The UI Automation inspector was hardened so it does not freeze the app as badly:

- snapshot work moved off the UI thread
- timeout handling added
- `maxDepth=0` supported
- clearer warning messages

However, Claude Desktop on Windows often does not expose a readable UIA tree for the Chromium content root. This is an important Phase 2 finding, not just a UI bug.

## Current Blocker In Detail

The user opened a regular Claude Home chat, for example:

- `Multi-agent project proposal analyzer`
- possibly `SQL database check`

Before recent fixes, the app incorrectly showed a stale Claude Code title:

```text
Next.js npm installation errors
```

That stale fallback problem was addressed. The app now checks Claude Desktop shell mode from local storage. In the observed Windows data, the app found values like:

```text
LSS-sidebar-selected-mode = task
dframe-store.state.lastKnownMode = "chat"
```

Those values were found in Claude Desktop Chromium local storage, including a file similar to:

```text
Local Storage\leveldb\012924.log
```

After that fix, the app correctly refuses to fall back to Claude Code while Claude is in Home/Chat. But it still cannot find the actual Home/Cowork transcript.

The current regular-chat cache reader is here:

```text
src-tauri/src/capture/windows/web_cache.rs
```

It scans Claude Desktop Chromium storage files for UTF-8 and UTF-16LE JSON fragments containing things like:

```text
chat_messages
chat_conversation
react-query-cache-ls
dframe-store
```

Earlier in development, a local storage log file did contain a full regular conversation record:

```text
Local Storage\leveldb\012915.log
```

That record included:

```text
name: Multi-agent project proposal analyzer
uuid: 7289b876-bc06-4859-8c15-be33a89db50b
chat_messages: [...]
```

Later, Claude Desktop compacted or rotated the LevelDB files, and the current files no longer contained that transcript. Current local storage and session storage scans found shell mode metadata but not `chat_messages` or the visible chat title.

This suggests regular Home/Cowork messages may currently be one of:

- only in memory
- in IndexedDB rather than Local Storage
- in another Claude Desktop cache location
- behind an authenticated Claude API response
- serialized or encoded in a way the naive scanner does not parse
- available through DOM/DevTools rather than local cache

## Best Next Investigation Paths

### 1. Investigate macOS Claude Desktop storage

Find the actual Claude Desktop data directory on macOS. Likely candidates:

```text
~/Library/Application Support/Claude/
~/Library/Containers/*Claude*/Data/Library/Application Support/
```

Search for:

```text
Local Storage/leveldb
Session Storage
IndexedDB
claude-code-sessions
```

Copy the storage folders before inspecting them, because Claude may lock, compact, or rotate the files while running.

### 2. Parse IndexedDB properly

Do not rely only on raw string scanning of LevelDB logs. Investigate IndexedDB contents for regular Home/Cowork conversations.

Search copied cache data for:

```text
chat_messages
chat_conversation
conversation
artifact
dframe-store
react-query-cache-ls
organization_uuid
current_conversation
```

Also search by the visible conversation title.

### 3. Investigate authenticated Claude API/cache data

If the current conversation UUID can be found from route state or cache, look for a Claude endpoint that returns full conversation JSON.

The app should only use the user's local Claude Desktop authenticated state and should avoid uploading any conversation data anywhere.

The likely path is:

1. get current organization/account context from Claude Desktop cache
2. get current conversation id from route/cache
3. fetch or reconstruct the conversation detail
4. export locally

### 4. Investigate DOM or DevTools access

Check whether Claude Desktop can be launched with a remote debugging port or otherwise exposes WebView/Chromium DevTools access.

If available, DOM extraction may be more reliable than UIA on Windows. On macOS, accessibility APIs may work better if Claude Desktop is granted Accessibility permissions.

### 5. OCR fallback only if needed

OCR may be needed if no cache/API/DOM path is available, but it has limitations:

- visible content only
- needs controlled scrolling to capture a full transcript
- message role separation can be fragile
- the user previously said not to implement Phase 3 scrolling/PDF unless explicitly approved

OCR should be treated as a fallback, not the primary export method, unless the user explicitly approves that direction.

## Files To Read First

Start with these:

```text
src-tauri/src/capture/windows/web_cache.rs
src-tauri/src/capture/windows/transcript.rs
src-tauri/src/capture/windows/mod.rs
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

macOS scaffold:

```text
src-tauri/src/capture/macos.rs
```

## Key UX Requirement

Do not let the app export a Claude Code transcript when the user has a regular Claude Home/Cowork chat open.

If the Home/Cowork transcript cannot be found, show a clear failure and explain which source paths were checked. Do not silently fall back to Claude Code.

## User's Latest Request

The user does not want behavior changes right now. They only asked for this handoff file so Claude on a Mac can continue from the latest blocker tomorrow.

