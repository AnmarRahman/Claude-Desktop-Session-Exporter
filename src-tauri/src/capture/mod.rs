use crate::models::{
    AccessibilitySnapshot, ClaudeDetection, DiagnosticSaveResult, InspectorOptions,
    SessionMetadata, VisibleTextBlock,
};
use thiserror::Error;

pub mod claude;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(windows, target_os = "macos")))]
mod unsupported;
#[cfg(windows)]
mod windows;

pub trait CaptureAdapter {
    fn detect_claude(&self) -> Result<ClaudeDetection, CaptureError>;
    fn get_active_session(&self) -> Result<SessionMetadata, CaptureError>;
    fn accessibility_snapshot(
        &self,
        options: InspectorOptions,
    ) -> Result<AccessibilitySnapshot, CaptureError>;
    fn visible_text_blocks(
        &self,
        options: InspectorOptions,
    ) -> Result<Vec<VisibleTextBlock>, CaptureError>;
    fn save_diagnostic_snapshot(
        &self,
        options: InspectorOptions,
    ) -> Result<DiagnosticSaveResult, CaptureError>;
}

#[derive(Debug, Error)]
pub enum CaptureError {
    #[error("Native accessibility query failed: {0}")]
    Native(String),
    #[error("Diagnostic snapshot failed: {0}")]
    Diagnostic(String),
}

#[cfg(windows)]
pub fn platform_adapter() -> windows::WindowsCaptureAdapter {
    windows::WindowsCaptureAdapter
}

#[cfg(target_os = "macos")]
pub fn platform_adapter() -> macos::MacOsCaptureAdapter {
    macos::MacOsCaptureAdapter
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn platform_adapter() -> unsupported::UnsupportedCaptureAdapter {
    unsupported::UnsupportedCaptureAdapter
}
