# Claude Session Exporter

Claude Session Exporter is a local-first desktop application for capturing the currently open Claude Desktop conversation and, eventually, exporting it as a complete, well-formatted PDF.

The project is built with Tauri 2, React, TypeScript, and Rust. The long-term goal is to preserve Claude chats and Claude Cowork sessions with searchable text, code blocks, tables, images, file cards, and activity cards whenever the operating system exposes them.

> Current status: early Phase 2. The app can build on Windows and includes Windows process/window detection, UI Automation inspection, a Developer Mode accessibility inspector, visible-text extraction scaffolding, and diagnostic snapshot export. Full conversation capture, automatic scrolling, preview, and PDF export are not implemented yet.

## Why This Exists

Claude Desktop conversations can contain useful work: plans, decisions, code, diagrams, files, screenshots, and Cowork activity. Claude Session Exporter is intended to turn that visible session into a durable local artifact without relying on Claude's built-in print flow, undocumented Claude databases, cookies, credentials, or server-side scraping.

The target workflow is:

```text
Open Claude Desktop
Open the conversation you want to preserve
Open Claude Session Exporter
Capture the current session
Preview the reconstructed conversation
Export a PDF
```

Only the inspection and diagnostics pieces exist today. PDF generation is part of a later phase.

## Features Implemented So Far

- Tauri 2 desktop shell with React and TypeScript.
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

- Automatic conversation scrolling.
- Full long-session reconstruction.
- PDF export.
- HTML preview.
- Semantic extraction of code blocks, tables, images, and Cowork activity cards.
- Reliable user-vs-Claude message classification.
- macOS runtime capture.

## Privacy Model

Claude Session Exporter is designed to operate locally.

It should not:

- upload conversation contents
- call external AI APIs
- require Anthropic or Claude credentials
- scrape Anthropic servers
- read browser cookies
- modify Claude Desktop files
- depend on Claude's internal databases as the primary capture path

Developer diagnostic snapshots can contain visible conversation text. They are saved locally and should be treated as private files.

## Architecture

```text
Claude Desktop
      |
OS Accessibility / UI Automation
      |
Platform Capture Adapter
      |
Conversation Scanner
      |
Normalized Session Model
      |
HTML Renderer
      |
PDF Generator
```

Only the first platform-capture layer is currently under active validation.

Important source areas:

- `src/` - React/TypeScript frontend
- `src-tauri/src/` - Rust native application code
- `src-tauri/src/capture/windows/` - Windows process, window, and UIA inspection
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

## Documentation

- [Architecture](docs/ARCHITECTURE.md)
- [Capture Engine](docs/CAPTURE_ENGINE.md)
- [Windows Notes](docs/WINDOWS.md)
- [macOS Notes](docs/MACOS.md)
- [Claude UI Assumptions](docs/CLAUDE_UI_ASSUMPTIONS.md)
- [Implementation Plan](IMPLEMENTATION_PLAN.md)

## Roadmap

1. Finish live Windows Phase 2 validation against Claude Desktop.
2. Implement Phase 3 automatic scrolling and deduplication.
3. Add semantic extraction for headings, lists, code blocks, tables, and links.
4. Add image and visual fallback capture.
5. Add HTML preview and PDF export.
6. Add Cowork-specific extraction.
7. Package polished Windows and macOS builds.

## Disclaimer

This project is independent and is not affiliated with Anthropic or Claude. It inspects the already-open desktop application through local operating system accessibility APIs.

## License

No open-source license has been selected yet.
