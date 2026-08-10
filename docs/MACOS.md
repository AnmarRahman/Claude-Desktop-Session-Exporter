# macOS Capture Notes

macOS capture should use AXUIElement.

Expected permissions:

- Accessibility permission for inspecting and controlling Claude Desktop UI.
- Screen Recording permission if visual fallback screenshots are needed.

Current Phase 1 behavior:

- compiles a macOS adapter scaffold
- reports that AXUIElement implementation is pending
- documents permission risks

Future macOS work:

- detect Claude through running applications
- check accessibility trust
- inspect AX windows and children
- classify roles/attributes into the shared capture model
- implement scroll actions and visual fallback capture
