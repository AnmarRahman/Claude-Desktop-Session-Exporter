mod process;
mod uia;

use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::capture::{claude, CaptureAdapter, CaptureError};
use crate::models::{
    AccessibilitySnapshot, ClaudeDetection, DiagnosticSaveResult, InspectorOptions,
    SessionMetadata, VisibleTextBlock,
};

pub struct WindowsCaptureAdapter;

impl CaptureAdapter for WindowsCaptureAdapter {
    fn detect_claude(&self) -> Result<ClaudeDetection, CaptureError> {
        let processes = process::find_claude_processes();
        let windows = uia::find_claude_windows(&processes)?;
        let detected = !processes.is_empty() || !windows.is_empty();
        let message = if detected {
            "Claude detected".to_string()
        } else {
            "Claude Desktop isn't running.".to_string()
        };

        Ok(ClaudeDetection {
            detected,
            platform: "windows-uiautomation".to_string(),
            processes,
            windows,
            message,
        })
    }

    fn get_active_session(&self) -> Result<SessionMetadata, CaptureError> {
        let processes = process::find_claude_processes();
        let title = uia::find_claude_windows(&processes)?
            .into_iter()
            .find_map(|window| claude::detection::clean_claude_window_title(&window.title));

        Ok(SessionMetadata {
            title,
            session_type: "unknown".to_string(),
        })
    }

    fn accessibility_snapshot(
        &self,
        options: InspectorOptions,
    ) -> Result<AccessibilitySnapshot, CaptureError> {
        let processes = process::find_claude_processes();
        let root = uia::inspect_first_claude_window(&processes, options)?;
        Ok(claude::conversation::analyze_snapshot(root))
    }

    fn visible_text_blocks(
        &self,
        options: InspectorOptions,
    ) -> Result<Vec<VisibleTextBlock>, CaptureError> {
        Ok(self.accessibility_snapshot(options)?.visible_text_blocks)
    }

    fn save_diagnostic_snapshot(
        &self,
        options: InspectorOptions,
    ) -> Result<DiagnosticSaveResult, CaptureError> {
        let snapshot = self.accessibility_snapshot(options)?;
        let diagnostics_dir = std::env::current_dir()
            .map_err(|error| CaptureError::Diagnostic(error.to_string()))?
            .join("diagnostics");
        fs::create_dir_all(&diagnostics_dir)
            .map_err(|error| CaptureError::Diagnostic(error.to_string()))?;

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| CaptureError::Diagnostic(error.to_string()))?
            .as_secs();
        let path = diagnostics_dir.join(format!("claude-accessibility-{timestamp}.json"));
        let json = serde_json::to_string_pretty(&snapshot)
            .map_err(|error| CaptureError::Diagnostic(error.to_string()))?;
        fs::write(&path, json).map_err(|error| CaptureError::Diagnostic(error.to_string()))?;

        Ok(DiagnosticSaveResult {
            path: path.display().to_string(),
            warning: "Diagnostic snapshots can contain text from your Claude conversation. They remain on this computer.".to_string(),
        })
    }
}
