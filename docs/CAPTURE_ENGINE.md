# Capture Engine

Transcript extraction reads Claude Desktop's local data rather than scanning its
UI. See [WEB_CACHE.md](WEB_CACHE.md) for the Home/Cowork source.

Implemented:

- Home/Cowork conversations from the Chromium HTTP cache (all platforms)
- Claude Code sessions from JSONL transcripts (Windows)
- normalization into ordered blocks: text, thinking, tool use, tool results,
  attachments, files
- Markdown and JSON export

Still to come:

- HTML and PDF rendering
- images and visual fallback capture
- a conversation picker over the cached conversations

The scroll-driven scanning, fingerprinting, and deduplication once planned for
timeline extraction are no longer needed: the cached payload is the complete,
already-ordered conversation. They would only return if a UI-scraping fallback
becomes necessary.

The engine must never silently drop unsupported content. Unknown elements should become warnings and, when possible, visual fallback blocks.
