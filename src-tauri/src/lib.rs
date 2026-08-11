mod capture;
mod filename;
mod models;

use capture::CaptureAdapter;
use models::{
    AccessibilitySnapshot, ChatExportOptions, ChatExportResult, ClaudeDetection,
    DiagnosticSaveResult, InspectorOptions, SessionMetadata, VisibleContentCapture,
    VisibleTextBlock,
};

#[tauri::command]
fn detect_claude() -> Result<ClaudeDetection, String> {
    capture::platform_adapter()
        .detect_claude()
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn get_active_session() -> Result<SessionMetadata, String> {
    capture::platform_adapter()
        .get_active_session()
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn get_accessibility_snapshot(
    options: Option<InspectorOptions>,
) -> Result<AccessibilitySnapshot, String> {
    capture::platform_adapter()
        .accessibility_snapshot(options.unwrap_or(InspectorOptions {
            max_depth: None,
            max_elements: None,
            tree_view: None,
        }))
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn extract_visible_text(
    options: Option<InspectorOptions>,
) -> Result<Vec<VisibleTextBlock>, String> {
    capture::platform_adapter()
        .visible_text_blocks(options.unwrap_or(InspectorOptions {
            max_depth: None,
            max_elements: None,
            tree_view: None,
        }))
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn save_diagnostic_snapshot(
    options: Option<InspectorOptions>,
) -> Result<DiagnosticSaveResult, String> {
    capture::platform_adapter()
        .save_diagnostic_snapshot(options.unwrap_or(InspectorOptions {
            max_depth: None,
            max_elements: None,
            tree_view: None,
        }))
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn capture_visible_content() -> Result<VisibleContentCapture, String> {
    capture::platform_adapter()
        .capture_visible_content()
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn export_chat_transcript(options: Option<ChatExportOptions>) -> Result<ChatExportResult, String> {
    capture::platform_adapter()
        .export_chat_transcript(options.unwrap_or(ChatExportOptions {
            source: Some("auto".to_string()),
        }))
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn sanitize_filename_part(value: String) -> String {
    filename::sanitize_filename_part(&value, "Claude Session")
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            detect_claude,
            get_active_session,
            get_accessibility_snapshot,
            extract_visible_text,
            save_diagnostic_snapshot,
            capture_visible_content,
            export_chat_transcript,
            sanitize_filename_part
        ])
        .run(tauri::generate_context!())
        .expect("error while running Claude Session Exporter");
}
