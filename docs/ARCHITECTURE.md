# Architecture

Claude Session Exporter separates native capture from structured rendering.

Transcripts come from Claude Desktop's own local data, not from its UI. There are
two readers, and both produce the same normalized model:

```text
Claude Desktop
      |
      +-- Chromium profile (HTTP cache)  --> Home/Cowork reader   [macOS verified]
      |
      +-- claude-code-sessions + JSONL   --> Claude Code reader   [Windows today]
      |
Normalized Session Model
      |
Markdown / JSON export   (HTML + PDF still to come)
```

Accessibility is a separate, secondary path used for detection, diagnostics, and
visual fallback — not for extracting transcripts:

```text
Claude Desktop  ->  OS Accessibility / UI Automation  ->  Platform Capture Adapter
```

The frontend calls Tauri commands. Rust selects a platform adapter at compile time:

- Windows: UI Automation.
- macOS: AXUIElement.
- Other platforms: unsupported adapter.

The adapters share `capture::web_cache`. It is written to be platform-neutral,
but only the macOS profile has been read for real; Chromium has a second HTTP
cache backend that this reader detects and reports rather than misreading.
The normalized session model and the renderers must stay platform-independent
too.
