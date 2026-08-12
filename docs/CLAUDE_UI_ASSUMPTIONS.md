# Claude UI Assumptions

This file records assumptions about Claude Desktop that must be validated against real app versions.

## Validated Against Live Claude Desktop

macOS, 2026-08-11, Electron profile at `~/Library/Application Support/Claude`:

- Regular Home transcripts are in the Chromium HTTP disk cache as the
  JSON response to `chat_conversations/<uuid>?tree=True&rendering_mode=messages`.
  They are **not** in Local Storage, Session Storage, or IndexedDB. See
  [WEB_CACHE.md](WEB_CACHE.md) for the format and the payload shape.
- Shell mode comes from Local Storage on macOS as well as Windows, so the
  Home-vs-Code routing is shared. Two keys carry it — `lastKnownMode` inside the
  `dframe-store` blob, and the bare `sidebar-selected-mode` — and **both are
  frequently compacted away while Claude Desktop runs**. One profile carried
  `sidebar-selected-mode` only, then neither, within an hour. Routing must treat
  "unknown mode" as the normal case.
- Claude Desktop's main process is the executable under
  `/Claude.app/Contents/MacOS/`; helpers, frameworks, and the Claude Code CLI
  must not be matched by name alone.
- Opening a conversation refreshes its cache entry, which makes "most recently
  written cache entry" a dependable stand-in for "the conversation the user has
  open" **within the Home chat store**. No accessibility or DOM access is needed.
- **Cowork sessions are not in the web cache.** Verified 2026-08-12: metadata
  lives under `~/Library/Application Support/Claude/local-agent-mode-sessions`
  and each session owns a nested `.claude/projects` tree. Metadata
  `cliSessionId` is the authoritative join key for
  `**/<cliSessionId>.jsonl`; nested `subagents` JSONLs are not standalone
  sessions.
- **Desktop Code is a different layout.** Its metadata is under
  `~/Library/Application Support/Claude/claude-code-sessions` and its main
  transcripts are in shared `~/.claude/projects`.
- Shell state recognizes `chat` / `home` / `task`, `code`, and `cowork`, but it
  is not an authoritative active-session signal. In live builds, a visible
  Cowork conversation has also been observed while the stored mode still read
  as Home/task.
- Recency cannot arbitrate between the two stores. Merely *viewing* a Home chat
  rewrites its cache entry, whereas a Cowork session's files change only when it
  is worked in — so a Cowork session that is open on screen can look "older"
  than a Home chat the user only glanced at. The source must be chosen
  explicitly; the device-session picker is the authoritative export target.
- CoreGraphics can enumerate Claude windows without Accessibility permission,
  but macOS may redact `kCGWindowName`. When a title is exposed it can select an
  exact matching session; when it is blank, the app must not claim to know which
  conversation is visible.
- Session Storage contains `chat-drawer-snapshot-store`, keyed by a regular Home
  UUID or Cowork `local_*` ID. The snapshot with the greatest `at` timestamp
  matched the visible Cowork conversation in live testing and is the primary
  active-session signal. If it is missing or cannot be correlated, selection
  remains explicit rather than guessed from unrelated file mtimes.

This removes the dependency on accessibility trees for transcript extraction on
both platforms. The UIA/AX work below still matters only for visual fallback and
diagnostics.

## Confirmed In Source Implementation

- Claude detection now combines multiple Windows signals: process name, top-level UI Automation window title, and process ID linkage between UIA elements and known Claude-like processes.
- Windows UI Automation traversal is bounded by configurable `maxDepth` and `maxElements` values.
- Claude-specific detection and conversation-region analysis are separated from the low-level Windows UIA adapter.
- Visible text extraction is preliminary and uses UIA element name/value/text pattern data when exposed.
- User/Claude author classification remains conservative and returns `unknown` unless a structural signal is available.

## Unverified In Live Claude Desktop

- Whether Claude exposes a top-level UI Automation window in this desktop environment.
- Whether conversation text is exposed as Text controls, Document text patterns, Value patterns, or only flattened names.
- Whether the likely conversation container exposes the Scroll pattern.
- Whether user and assistant messages have stable, distinct parent hierarchies or layout bounds.
- Whether Cowork activity cards are visible to UI Automation.
- Whether code blocks preserve whitespace through UI Automation.
- Whether tables expose row/column structure or only text.
- Whether generated previews expose useful accessibility metadata.

## Phase 1 Assumptions Retained Until Live Validation

- Claude Desktop has a running process whose name is likely `Claude`, `Claude.exe`, or `Claude Desktop.exe`.
- Claude Desktop has at least one top-level application window whose title may contain `Claude`.
- Conversation title may be recoverable from a top-level window title, but this is not reliable enough to infer Chat vs Cowork.
- The Windows control-view UI Automation tree can expose at least a shallow hierarchy for the Claude window.
- The accessibility tree may be incomplete or flattened because Claude Desktop is likely web technology inside a desktop shell.

Every new Claude UI assumption should be added here with the Claude Desktop version and platform used during validation.
