use crate::capture::{CaptureAdapter, CaptureError};
use crate::models::{
    AccessibilitySnapshot, ClaudeDetection, DiagnosticSaveResult, InspectorOptions,
    SessionMetadata, VisibleTextBlock,
};

pub struct MacOsCaptureAdapter;

impl CaptureAdapter for MacOsCaptureAdapter {
    fn detect_claude(&self) -> Result<ClaudeDetection, CaptureError> {
        Ok(ClaudeDetection {
            detected: false,
            platform: "macos-axuielement".to_string(),
            processes: vec![],
            windows: vec![],
            message: "macOS AXUIElement adapter is scaffolded but not implemented in Phase 1."
                .to_string(),
        })
    }

    fn get_active_session(&self) -> Result<SessionMetadata, CaptureError> {
        Ok(SessionMetadata {
            title: None,
            session_type: "unknown".to_string(),
        })
    }

    fn accessibility_snapshot(
        &self,
        options: InspectorOptions,
    ) -> Result<AccessibilitySnapshot, CaptureError> {
        Ok(AccessibilitySnapshot {
            platform: "macos-axuielement".to_string(),
            root_name: None,
            max_depth: options.max_depth.unwrap_or(12),
            max_elements: options.max_elements.unwrap_or(5_000),
            element_count: 0,
            truncated: false,
            nodes: vec![],
            conversation_candidates: vec![],
            visible_text_blocks: vec![],
            warnings: vec![
                "macOS requires Accessibility permission before Claude UI inspection can work."
                    .to_string(),
                "Visual fallback capture may require Screen Recording permission.".to_string(),
            ],
        })
    }

    fn visible_text_blocks(
        &self,
        _options: InspectorOptions,
    ) -> Result<Vec<VisibleTextBlock>, CaptureError> {
        Ok(vec![])
    }

    fn save_diagnostic_snapshot(
        &self,
        _options: InspectorOptions,
    ) -> Result<DiagnosticSaveResult, CaptureError> {
        Err(CaptureError::Diagnostic(
            "macOS runtime support is not implemented in Phase 2.".to_string(),
        ))
    }
}
