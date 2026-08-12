# Architecture

Claude Session Exporter separates native capture from structured rendering.

Transcripts come from Claude Desktop's own local data, not from its UI. There are
three storage layouts, and all produce the same normalized model:

```text
Claude Desktop
      |
      +-- Chromium profile (HTTP cache)  --> Home reader          [macOS verified]
      |
      +-- local-agent-mode-sessions/local_*.json -- cliSessionId --> nested JSONL
      |
      +-- claude-code-sessions + ~/.claude/projects/**/*.jsonl --> Desktop Code
      |
Normalized Session Model
      |
Markdown / JSON / PDF export   (HTML preview still to come)
```

Accessibility is a separate, secondary path used for detection, diagnostics, and
visual fallback — not for extracting transcripts:

```text
Claude Desktop  ->  OS Accessibility / UI Automation  ->  Platform Capture Adapter
```

Claude's Session Storage `chat-drawer-snapshot-store` provides the primary
active-conversation hint. Its newest per-conversation `at` timestamp is mapped
from a Home UUID or Cowork `local_*` ID to the canonical picker session. An
exposed top-level window title is a second exact-match signal.

The frontend calls Tauri commands. Rust selects a platform adapter at compile time:

- Windows: UI Automation.
- macOS: AXUIElement.
- Other platforms: unsupported adapter.

The adapters share `capture::web_cache` and `capture::cowork`. The latter indexes
the shared Desktop Code tree and each Cowork session's nested transcript tree.
Cowork is emitted only for an exact metadata `cliSessionId` / JSONL filename
match, and `subagents` directories are excluded. The readers are
platform-neutral, though these Desktop metadata paths are verified only on
macOS. The web cache is written to be platform-neutral,
but only the macOS profile has been read for real; Chromium has a second HTTP
cache backend that this reader detects and reports rather than misreading.
The normalized session model and the renderers must stay platform-independent
too.
