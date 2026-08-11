# macOS Capture Notes

## Working today

Home/Cowork transcript export works on macOS. It reads Claude Desktop's Chromium
profile at `~/Library/Application Support/Claude` — see
[WEB_CACHE.md](WEB_CACHE.md). This needs no special permission: the profile
belongs to the user running the app, and the files are only read.

Cowork and Claude Code session export work too, from local JSONL transcripts —
see the Cowork notes in [CLAUDE_UI_ASSUMPTIONS.md](CLAUDE_UI_ASSUMPTIONS.md).
`capture/transcript.rs` is now platform-neutral and reads both
`local-agent-mode-sessions` (Cowork) and `claude-code-sessions` (Claude Code).

Claude Desktop detection works too, via a process scan for the bundle executable
`/Claude.app/Contents/MacOS/`. That path is what separates Claude Desktop from
its own helper and framework processes and from the Claude Code CLI, which is a
different program that also answers to `claude`. It needs no permission.

The shell-mode read (`lastKnownMode`, `sidebar-selected-mode`) is shared with
Windows, but both keys are frequently compacted out of local storage, so `None`
is the common answer. The session type falls back to reporting what is actually
exportable rather than "unknown".

## Not implemented yet

- **Window enumeration.** `detect_claude` reports processes but no windows;
  listing windows needs Accessibility permission, and nothing on the export path
  requires it.
- **Selecting *which* Cowork/Claude Code session to export.** The reader takes
  the most recently touched one in the chosen store. That is correct for Cowork
  in practice, but there is no reliable signal for which session is on screen.
- **AXUIElement inspection and visual fallback.** Still scaffolded.

## Permissions

Only the accessibility and screenshot paths need permission, and neither is on
the transcript export path any more:

- Accessibility permission for inspecting or controlling Claude Desktop's UI.
- Screen Recording permission for visual fallback screenshots.

Missing permission should stay a recoverable diagnostic state, not a hard error.
