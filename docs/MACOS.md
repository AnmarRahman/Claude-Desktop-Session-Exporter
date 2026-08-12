# macOS Capture Notes

## Working today

Home transcript export works on macOS. It reads Claude Desktop's Chromium
profile at `~/Library/Application Support/Claude` — see
[WEB_CACHE.md](WEB_CACHE.md). This needs no special permission: the profile
belongs to the user running the app, and the files are only read.

Cowork and Claude Code session export work too, from local JSONL transcripts.
`capture/cowork.rs` reads Cowork metadata from `local-agent-mode-sessions` and
joins each record by exact `cliSessionId` to its session-nested JSONL. It also
reads Desktop Code metadata from `claude-code-sessions` and indexes the shared
`~/.claude/projects` tree. Nested `subagents` transcripts are excluded.
`capture/transcript.rs` reuses that classified discovery for export.

Claude Desktop detection works too, via a process scan for the bundle executable
`/Claude.app/Contents/MacOS/`. That path is what separates Claude Desktop from
its own helper and framework processes and from the Claude Code CLI, which is a
different program that also answers to `claude`. It needs no permission.

The shell-mode read (`lastKnownMode`, `sidebar-selected-mode`) is shared with
Windows, but both keys may be missing or stale. It cannot reliably distinguish
a Home conversation from a Cowork conversation currently displayed in the Home
surface. The searchable session picker therefore controls exact export.

## Not implemented yet

- **AXUIElement inspection and visual fallback.** Still scaffolded.

CoreGraphics window enumeration is best effort and permission-free. macOS may
redact the title; an exposed title is matched to a stored session, while a blank
title falls back to Claude's Session Storage `chat-drawer-snapshot-store`. Its
newest snapshot maps the visible Home UUID or Cowork `local_*` ID to the picker.
If neither exact signal is available, selection remains manual.

## Permissions

Only the accessibility and screenshot paths need permission, and neither is on
the transcript export path any more:

- Accessibility permission for inspecting or controlling Claude Desktop's UI.
- Screen Recording permission for visual fallback screenshots.

Missing permission should stay a recoverable diagnostic state, not a hard error.
