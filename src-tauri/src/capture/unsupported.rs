use crate::capture::{CaptureAdapter, CaptureError};
use crate::models::{
    AccessibilitySnapshot, ClaudeDetection, DiagnosticSaveResult, InspectorOptions,
    SessionMetadata, VisibleTextBlock,
};

pub struct UnsupportedCaptureAdapter;

impl CaptureAdapter for UnsupportedCaptureAdapter {
    fn detect_claude(&self) -> Result<ClaudeDetection, CaptureError> {
        Ok(ClaudeDetection {
            detected: false,
            platform: "unsupported".to_string(),
            processes: vec![],
            windows: vec![],
            message: "This platform is not supported by Claude Session Exporter.".to_string(),
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
            platform: "unsupported".to_string(),
            root_name: None,
            max_depth: options.max_depth.unwrap_or(12),
            max_elements: options.max_elements.unwrap_or(5_000),
            element_count: 0,
            truncated: false,
            nodes: vec![],
            conversation_candidates: vec![],
            visible_text_blocks: vec![],
            warnings: vec!["Only Windows and macOS are planned targets.".to_string()],
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
            "This platform is not supported by Claude Session Exporter.".to_string(),
        ))
    }
}
