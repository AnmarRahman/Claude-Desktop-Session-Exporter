# Capture Engine

The capture engine will be built vertically across phases.

Phase 1 exposes only detection and shallow diagnostics. Later phases should add:

- visible text extraction
- scroll container detection
- upward and downward timeline scanning
- stable block fingerprinting
- deduplication
- semantic reconstruction
- image and visual fallback capture

The engine must never silently drop unsupported content. Unknown elements should become warnings and, when possible, visual fallback blocks.
