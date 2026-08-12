# Capture Engine

Transcript extraction reads Claude Desktop's local data rather than scanning its
UI. See [WEB_CACHE.md](WEB_CACHE.md) for the Home source and
[MACOS.md](MACOS.md) for Cowork metadata/JSONL correlation.

Implemented:

- Home conversations from the Chromium HTTP cache (all platforms)
- Cowork sessions from `local-agent-mode-sessions`, joined by exact metadata
  `cliSessionId` to the session-nested JSONL tree
- Claude Code sessions from `claude-code-sessions` and shared
  `~/.claude/projects` JSONL transcripts
- a unified searchable picker for cached Home, Cowork, and Claude Code sessions
- normalization into ordered blocks: text, thinking, tool use, tool results,
  attachments, files
- Markdown, JSON, and native paginated PDF export

Still to come:

- HTML preview rendering
- images and visual fallback capture

The scroll-driven scanning, fingerprinting, and deduplication once planned for
timeline extraction are no longer needed: the cached payload is the complete,
already-ordered conversation. They would only return if a UI-scraping fallback
becomes necessary.

The engine must never silently drop unsupported content. Unknown elements should become warnings and, when possible, visual fallback blocks.

PDF output is generated locally from the normalized message/block model. It
uses A4 pages, wrapped text, continuation headers, page numbering, distinct
user/Claude colors, and indented thinking/tool/reference sections. A Unicode
system font is embedded when available, with a named warning if the renderer
must fall back to a built-in PDF font.
