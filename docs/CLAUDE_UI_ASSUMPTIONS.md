# Claude UI Assumptions

This file records assumptions about Claude Desktop that must be validated against real app versions.

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
