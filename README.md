<img src="public/logo.svg" alt="" width="88" height="88" align="left" hspace="12" />

# Claude Session Exporter

Claude Session Exporter is a local-first desktop application for discovering and exporting Claude Desktop conversations and, eventually, preserving them as complete, well-formatted PDFs.

The project is built with Tauri 2, React, TypeScript, and Rust. The long-term goal is to preserve Claude chats and Claude Cowork sessions with searchable text, code blocks, tables, images, file cards, and activity cards whenever the operating system exposes them.

> Current status: transcript export works on macOS and produces Markdown, JSON, and a paginated PDF. Regular Claude Home conversations use the local Chromium cache. Cowork sessions use `local-agent-mode-sessions` metadata and session-nested JSONL transcripts; Desktop Code sessions use `claude-code-sessions` metadata and the shared `~/.claude/projects` tree. All three sources are searchable and selectable in the app. HTML preview is not implemented yet.

## Why This Exists

Claude Desktop conversations can contain useful work: plans, decisions, code, diagrams, files, screenshots, and Cowork activity. Claude Session Exporter turns a session into a durable local artifact without Claude's built-in print flow, without credentials, and without contacting any server.

It does this by reading the data Claude Desktop already stored on your own machine: the Chromium HTTP response cache for Home chats, Cowork's `local-agent-mode-sessions`, and Desktop Code's `claude-code-sessions` plus `~/.claude/projects`. Those formats are undocumented and can change without notice — see [docs/WEB_CACHE.md](docs/WEB_CACHE.md) for the cache reader's limits.

The target workflow is:

```text
Open Claude Desktop
Open the conversation you want to preserve
Open Claude Session Exporter
Export the session
```

Choose Markdown, JSON, PDF, or any combination of those formats before exporting. The selected files are written to the extraction directory configured in the app; without a custom directory, the app uses its local `exports/` folder. HTML preview is a later phase.

## Features Implemented So Far

- Tauri 2 desktop shell with React and TypeScript.
- Home conversation export from Claude Desktop's local response cache: title, both roles, message order, timestamps, code, tool activity, attachment and file metadata. **Verified on macOS only** — the Windows cache may use a different Chromium backend, which is detected and reported rather than misread.
- Filesystem-first Cowork discovery on macOS from `~/Library/Application Support/Claude/local-agent-mode-sessions/**/local_*.json`, joined by exact `cliSessionId` to the same session's nested `.claude/projects/**/<cliSessionId>.jsonl`.
- Desktop Code discovery from `claude-code-sessions` metadata and shared `~/.claude/projects/**/*.jsonl`; nested `subagents` transcripts are excluded from the session list.
- Unified, searchable Home, Cowork, and Claude Code picker and exact-session export, independent of whether Claude Desktop is running.
- Active conversation matching from Claude's Session Storage chat-drawer snapshots, with macOS window-title matching as a fallback.
- Cowork diagnostics for roots, record counts, matches, missing transcripts, duplicates, and selected-session paths/metadata.
- Selectable Markdown, JSON, and polished A4 PDF output (one, two, or all three), with a dedicated PDF cover, conversation-style user/Claude layout, rendered Markdown, tables, code panels, optional thinking blocks, and optional tool activity.
- Persistent extraction-directory picker, default-folder reset, and one-click opening in Finder or File Explorer.
- Source selector: Auto, Home / Cowork, Claude Code — where Auto refuses to fall back to a stale Claude Code session while Claude Desktop is showing Home/Chat.
- Windows-native capture boundary written in Rust.
- Claude process detection.
- Win32 top-level window enumeration using HWND, PID, visibility, title, class name, and bounds.
- Windows UI Automation attachment path using the detected window handle.
- Developer Mode Accessibility Inspector.
- Raw, Control, and Content UI Automation tree selectors.
- Recursive, bounded accessibility traversal with configurable depth and element count.
- UIA node serialization for control type, name, automation ID, class name, framework ID, bounds, state, values, and supported patterns.
- Preliminary conversation-region candidate scoring.
- Preliminary visible text block extraction from currently rendered accessibility nodes.
- Local diagnostic snapshot export.
- macOS adapter scaffold and documentation, with runtime support intentionally deferred.

## Not Implemented Yet

- HTML preview.
- Images: only file and attachment metadata is exported, not image bytes.

## Privacy Model

Claude Session Exporter is designed to operate locally.

It should not:

- upload conversation contents
- call external AI APIs
- require Anthropic or Claude credentials
- scrape Anthropic servers
- read browser cookies
- modify Claude Desktop files

It does read Claude Desktop's own local data files, because that is where the transcripts are. Those files are opened read-only and belong to the user running the app.

Exports and developer diagnostic snapshots contain conversation text. Exports are written to the directory shown in the app, diagnostics use the app's local `diagnostics/` directory, and both stay on this computer. They should be treated as private files.

## Architecture

```text
Claude Desktop
      |
      +-- Chromium profile (HTTP cache)  --> Home reader          [macOS verified]
      |
      +-- local-agent-mode-sessions -- cliSessionId --> nested JSONL --> Cowork
      |
      +-- claude-code-sessions + ~/.claude/projects JSONL -------> Claude Code
      |
Normalized Session Model
      |
Markdown / JSON / PDF     (HTML preview still to come)
```

Accessibility (UI Automation on Windows, AXUIElement on macOS) is a separate path
used for detection, diagnostics, and visual fallback — not for transcripts.

Important source areas:

- `src/` - React/TypeScript frontend
- `src-tauri/src/` - Rust native application code
- `src-tauri/src/capture/web_cache/` - Home transcripts, platform-neutral
- `src-tauri/src/capture/windows/` - Windows process, window, UIA, Claude Code transcripts
- `src-tauri/src/capture/claude/` - Claude-specific detection and analysis rules
- `docs/` - architecture notes and platform assumptions

## Development Requirements

Windows:

- Node.js
- Rust stable
- Visual Studio Build Tools with C++ tools
- Windows 10/11 SDK
- WebView2 runtime

The Windows SDK must expose libraries such as `kernel32.lib`. In a Visual Studio developer environment, these should be set:

```bat
WindowsSdkDir
WindowsSDKVersion
LIB
```

macOS:

- Node.js
- Rust stable
- Xcode command-line tools

macOS runtime capture is not implemented yet.

## Getting Started

Install dependencies:

```powershell
npm.cmd install
```

Run frontend tests:

```powershell
npm.cmd test
```

Build the frontend:

```powershell
npm.cmd run build
```

Check the Rust/Tauri side from a Visual Studio developer environment:

```bat
cargo check
cargo test
```

Run the desktop app in development:

```powershell
npm.cmd run tauri dev
```

Build the Windows app:

```powershell
npm.cmd run tauri build
```

The built executable is generated under:

```text
src-tauri/target/release/
```

## Current Verification Status

Verified on Windows:

- `npm test`
- `npm run build`
- `cargo check`
- `cargo test`
- `npm run tauri build`

Still needs live manual validation:

- identifying Claude's visible HWND in an interactive desktop session
- attaching UI Automation to that HWND
- confirming whether visible Claude conversation text is exposed
- comparing Raw, Control, and Content UIA views
- identifying the likely conversation/timeline container
- checking whether Cowork activity cards appear in UIA

## Logo

The app mark is **Carbon** — a speech bubble with a download arrow knocked out
of it, for a conversation written to disk. Three shapes, so it still reads at
16x16.

`tools/generate_logo.py` is the source of truth. It holds the geometry and emits
both `public/logo.svg` and every raster size in `src-tauri/icons/`, so the vector
art and the bundled icons cannot drift apart. It is stdlib-only — `zlib` writes
the PNG and ICO containers, and macOS `iconutil` packs the `.icns` — so no image
toolchain is needed to rebuild the assets:

```bash
python3 tools/generate_logo.py
```

Edit the constants near the top of that file to change the mark, then re-run it.

## Documentation

- [Architecture](docs/ARCHITECTURE.md)
- [Capture Engine](docs/CAPTURE_ENGINE.md)
- [Home Transcript Source](docs/WEB_CACHE.md)
- [Windows Notes](docs/WINDOWS.md)
- [macOS Notes](docs/MACOS.md)
- [Claude UI Assumptions](docs/CLAUDE_UI_ASSUMPTIONS.md)
- [Implementation Plan](IMPLEMENTATION_PLAN.md)

## Roadmap

1. Add a picker for choosing which cached conversation to export.
2. Confirm the Windows cache backend on a real install, and re-validate the whole Windows runtime path after the shared-reader refactor.
3. Implement Claude Desktop detection on macOS, and port Claude Code export to macOS.
4. Add HTML preview and inline image rendering in PDF exports.
5. Add image capture for content the payload only references.
6. Present Cowork activity cards distinctly rather than as generic tool blocks.
7. Package polished Windows and macOS builds.

## Disclaimer

This project is independent and is not affiliated with Anthropic or Claude. It reads local Claude Desktop data files belonging to the user running it, and inspects the running application through local operating system accessibility APIs. The data formats it reads are undocumented and may change at any time.

## License

No open-source license has been selected yet.
