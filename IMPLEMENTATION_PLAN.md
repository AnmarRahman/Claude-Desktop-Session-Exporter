# Claude Session Exporter Implementation Plan

## Repository Status

The repository was empty at inspection time. There was no existing project, package manifest, source tree, or git repository metadata in the workspace root.

## Current Technical Research

Tauri 2 is a realistic fit for the requested desktop shell because it supports a TypeScript/React frontend with Rust commands for native OS integration. Tauri's current prerequisites still require Rust, Node.js for a framework frontend, WebView2 on Windows, and Xcode or command-line tools on macOS.

For Windows capture, the primary route should be Microsoft UI Automation from Rust. The `uiautomation` crate wraps Windows UI Automation and exposes control patterns, tree walking, process filtering, and optional input support. For lower-level or missing APIs, the `windows` crate can call generated Win32, COM, and UI Automation bindings directly.

For macOS capture, the primary route should be Apple's Accessibility API via `AXUIElement`. The Rust `axuielement` crate provides safe bindings for AX elements, attributes, actions, observers, geometry, and process trust helpers. macOS will require the app to be granted Accessibility permission before it can inspect another app's UI, and visual fallback capture may require Screen Recording permission.

Research references:

- Tauri prerequisites: https://v2.tauri.app/start/prerequisites/
- Windows UI Automation Rust wrapper: https://docs.rs/crate/uiautomation/latest/source/README.md
- Rust for Windows bindings: https://github.com/microsoft/windows-rs
- macOS AXUIElement Rust bindings: https://docs.rs/axuielement/latest/axuielement/
- Apple AXUIElement reference: https://developer.apple.com/documentation/applicationservices/axuielement_h

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

The app should keep Claude-specific assumptions in one native capture area:

```text
src-tauri/src/capture/
    mod.rs
    macos.rs
    unsupported.rs
    claude/            detection + accessibility-tree analysis
    windows/           UIA, process scan, visual capture, Claude Code transcripts
    web_cache/         Home/Cowork transcripts (platform-neutral)
        mod.rs             index, select, export
        paths.rs           per-platform Chromium profile roots
        simple_cache.rs    Chromium cache entry format
        decode.rs          zstd / gzip / deflate / br
        conversation.rs    payload schema -> normalized model
        export.rs          Markdown + JSON writers
        shell_mode.rs      Home vs Code, from Local Storage
```

The TypeScript frontend should talk to Rust through Tauri commands only. The frontend should not know whether the active adapter is Windows UI Automation or macOS AXUIElement.

## Phase 1 Scope

Implement only:

- Tauri 2 + React + TypeScript application shell.
- Main screen with Claude detection status, current session placeholder metadata, capture options, and a disabled/non-destructive capture action.
- Rust capture abstraction with platform-specific modules.
- Windows process/window detection for Claude Desktop.
- macOS placeholder adapter compiled only on macOS, with documented AXUIElement strategy and permission status shape.
- Simple developer diagnostics view that can display a shallow accessibility/window snapshot.
- Basic tests/checks for pure frontend utility logic and Rust filename/session helpers if introduced.

Phase 1 explicitly does not claim conversation extraction, scrolling, deduplication, image capture, preview rendering, or PDF export.

## Phase Roadmap

> **Revised 2026-08-11.** Phases 2-5 were written on the assumption that
> transcripts had to be scraped from the accessibility tree. They do not:
> Claude Desktop's Chromium HTTP cache holds the complete conversation JSON
> (see [docs/WEB_CACHE.md](docs/WEB_CACHE.md)). Scroll-driven scanning,
> fingerprinting, and deduplication are no longer on the critical path.

### Phase 1 - Shell, Detection, Inspector (done on Windows)

Build the app shell and native adapter boundary. Detect whether Claude Desktop
appears to be running and expose enough diagnostic information to validate native
access assumptions. macOS detection is still outstanding.

### Phase 2 - Local Transcript Readers (done)

Read conversations from Claude Desktop's own local data. Claude Code sessions
from JSONL transcripts; Home/Cowork conversations from the Chromium HTTP cache.
Normalize both into one platform-independent model of ordered blocks: text,
thinking, tool use, tool results, attachments, files. Export Markdown and JSON.

### Phase 3 - Conversation Selection

Surface every cached conversation with its real title and date, and let the user
pick one instead of always taking the most recently opened. The reader already
supports this; the UI does not.

### Phase 4 - Platform Parity

macOS Claude detection via a running-application scan. Port the Claude Code
reader off Windows-only paths. Exercise the Windows runtime path after the shared
reader refactor.

### Phase 5 - HTML and PDF

Render normalized sessions to searchable HTML and generate PDF with print CSS,
page numbers, page-break handling, repeatable table headers, code formatting, and
filename sanitization.

### Phase 6 - Images and Visual Fallback

Capture images and bounding regions for content that cannot be reconstructed
from the payload. Track source type, size, chronology, and duplicate
fingerprints. This is also where UIA/AX scraping would return if a cache-less
fallback ever becomes necessary.

### Phase 7 - Cowork Classification

The payload already carries tool activity structurally. This phase is about
presenting it well: classifying known activity, file, and status cards rather
than rendering every `tool_use` identically.

### Phase 8 - Product Hardening

Add preview, settings, developer mode, warnings, partial export reporting,
diagnostics export, packaging, and cross-platform release validation.

## Normalized Model Direction

The native side should eventually produce serializable structures matching this shape:

```ts
type ClaudeSessionType = "chat" | "cowork" | "unknown";

interface ClaudeSession {
  id: string;
  title?: string;
  type: ClaudeSessionType;
  capturedAt: string;
  blocks: SessionBlock[];
  warnings: CaptureWarning[];
}
```

Extraction and rendering must remain separate. A failed semantic parse should produce a warning and a visual fallback block rather than silently dropping content.

## Technical Risks

### Cache Coverage

The HTTP cache is best-effort. A conversation never opened on this machine is not cached, Chromium evicts entries under pressure, and a cached copy is only as fresh as the last time the conversation was opened. Mitigation: report the source path and cache date on every export, and never present a partial export as complete. If coverage proves insufficient, the fallback is an authenticated refetch using the local session cookie, which requires Keychain-decrypting `Cookies`.

### Claude Accessibility Tree Quality

Confirmed on Windows: Claude Desktop often does not expose a readable UIA tree for the Chromium content root. This closed off UI scraping as a transcript source and is why the local-data readers exist. It still constrains the visual fallback and diagnostics paths.

### Image Capture

Accessibility APIs may expose image labels but not bytes. Mitigation: prefer recoverable image URLs or element data if available; otherwise capture the bounding rectangle with OS screenshot APIs and avoid duplicate thumbnails.

### Cowork Card Extraction

Cowork cards may be visually rich and structurally inconsistent. Mitigation: classify known activity/file/status patterns gradually, and preserve unknown cards as visual blocks with source metadata.

### macOS Permissions

Without Accessibility permission, AX calls may fail or return incomplete results. Visual fallback may additionally require Screen Recording permission. Mitigation: provide clear in-app permission status and instructions, avoid prompting until needed, and treat missing permission as a recoverable diagnostic state.

### Windows and macOS Differences

Windows UI Automation and macOS AXUIElement differ in roles, attribute names, bounds, event models, and scroll behavior. Mitigation: keep adapters platform-specific and convert to a shared internal tree/session model at the adapter boundary.

### Claude UI Changes

Claude Desktop may change process names, window titles, roles, hierarchy, and Cowork presentation. Mitigation: centralize signals, avoid single-selector dependence, maintain `docs/CLAUDE_UI_ASSUMPTIONS.md`, and keep diagnostics useful enough to refresh assumptions.

## Testing Strategy

Tests that do not require Claude should cover:

- filename sanitation
- block fingerprinting and deduplication
- ordering
- malformed or unknown block fallback
- HTML rendering of text, code, tables, and images once renderer exists
- fixture loading for chat, long chat, code chat, table chat, and Cowork sessions

Platform integration tests requiring Claude should remain manual at first and must be documented separately. No code should claim successful Claude extraction until tested against a real Claude Desktop session.

## Phase 1 Exit Criteria

- The Tauri app compiles on the current development machine.
- The frontend renders the simple main screen.
- The Rust command boundary returns Claude detection status.
- Developer Mode can show a shallow native diagnostic snapshot.
- Checks/tests that are available in Phase 1 have been run.
- Unverified areas are listed explicitly.
