# Architecture

Claude Session Exporter separates native capture from structured rendering.

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

The frontend calls Tauri commands. Rust selects a platform adapter at compile time:

- Windows: UI Automation.
- macOS: AXUIElement.
- Other platforms: unsupported adapter.

The conversation scanner, normalized session model, HTML renderer, and PDF generator should remain platform-independent.
