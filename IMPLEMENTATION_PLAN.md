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
    windows.rs
    macos.rs
    unsupported.rs
    claude/
        mod.rs
        signals.rs
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

### Phase 1 - Shell, Detection, Inspector

Build the app shell and native adapter boundary. Detect whether Claude Desktop appears to be running and expose enough diagnostic information to validate native access assumptions. Use guarded, shallow snapshots to avoid expensive full-tree walks.

### Phase 2 - Visible Text Extraction

Extract visible text from one normal Claude chat using native accessibility data. Normalize user and assistant message candidates into a platform-independent model and log blocks for inspection.

### Phase 3 - Scrolling and Deduplication

Find the conversation scroll container, scan upward to the beginning, scan downward to the end, wait for lazy content, fingerprint blocks, and deduplicate stable content.

### Phase 4 - Structured Content

Detect headings, paragraphs, lists, links, code blocks, and tables. Preserve table rows/columns structurally whenever the accessibility tree supports it.

### Phase 5 - Images and Visual Fallback

Capture image elements and visual bounding regions for content that cannot be semantically reconstructed. Track source type, size, chronology, and duplicate image fingerprints.

### Phase 6 - HTML and PDF

Render normalized sessions to searchable HTML and generate PDF with print CSS, page numbers, page-break handling, repeatable table headers, code formatting, and filename sanitization.

### Phase 7 - Cowork Extraction

Add Cowork-specific classifiers for tool activity, file cards, task progress, generated previews, and other timeline cards.

### Phase 8 - Product Hardening

Add preview, settings, developer mode, warnings, partial export reporting, diagnostics export, packaging, and cross-platform release validation.

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

### Claude Accessibility Tree Quality

Claude Desktop may not expose every timeline element semantically. Electron/WebView-backed content can appear as coarse groups, flattened text, or unlabeled custom elements. Mitigation: build diagnostics early, collect multiple attributes per element, and design fallback screenshot blocks.

### Virtualized Conversation Rendering

Long conversations may only keep a subset of messages in the accessibility tree at any time. Mitigation: scanning must be scroll-driven, incremental, cancellable, and deduplicated by content fingerprint.

### Programmatic Scrolling

The scroll container may not expose a reliable scroll pattern/action. Mitigation: try native scroll patterns first, then accessibility actions, then controlled input wheel events targeted at the window. Track scroll position, content changes, and no-progress loops.

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
