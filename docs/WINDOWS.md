# Windows Capture Notes

Windows capture uses Microsoft UI Automation through Rust.

Current Phase 1 behavior:

- scans running processes for Claude-like names
- scans top-level UI Automation windows for Claude-like titles or process IDs
- returns a bounded control-view tree snapshot for diagnostics

Current Phase 2 source behavior:

- detects Claude using process and UI Automation window signals
- captures a recursive UI Automation tree with configurable depth and element limits
- records control type, localized control type, name, automation ID, class name, framework ID, bounds, enabled/focus/offscreen state, value/text, and supported pattern names where available
- analyzes likely conversation containers without treating the result as authoritative
- extracts currently rendered visible text blocks in screen/tree order
- saves local diagnostic snapshots under `diagnostics/`

Build requirement:

- Rust MSVC builds require both Visual Studio C++ Build Tools and a Windows SDK containing libraries such as `kernel32.lib`.
- If `link.exe` exists but `kernel32.lib` is missing, install the Windows 10/11 SDK component for the Build Tools installation.

Future Windows work:

- identify Claude's conversation scroll container
- read text/value/range/grid patterns
- perform controlled scroll actions
- capture visual fallback regions when UI Automation is incomplete

Permissions should be limited to what Windows UI Automation and screenshot fallback require.
