mod capture;
mod filename;
mod models;

use tauri::Emitter;

use capture::CaptureAdapter;
use models::{
    AccessibilitySnapshot, ChatExportOptions, ChatExportResult, ClaudeDetection,
    DiagnosticSaveResult, InspectorOptions, LocalSessionDiscovery, SessionMetadata,
    VisibleContentCapture, VisibleTextBlock,
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

/// Filesystem discovery is independent of whether Claude Desktop is running or
/// which conversation is currently visible.
#[tauri::command]
fn discover_local_sessions() -> LocalSessionDiscovery {
    let mut discovery = capture::cowork::discover_default();
    discovery
        .sessions
        .extend(capture::web_cache::list_home_sessions());
    discovery.sessions.sort_by(|left, right| {
        right
            .last_focused_at
            .unwrap_or(0)
            .max(right.last_activity_at.unwrap_or(0))
            .max(right.metadata_modified_at.unwrap_or(0))
            .cmp(
                &left
                    .last_focused_at
                    .unwrap_or(0)
                    .max(left.last_activity_at.unwrap_or(0))
                    .max(left.metadata_modified_at.unwrap_or(0)),
            )
    });
    if let Some(active) = capture::web_cache::latest_active_drawer_session() {
        if let Some(session) = discovery.sessions.iter().find(|session| {
            session.cli_session_id == active.session_id
                || session.desktop_session_id.as_deref() == Some(active.session_id.as_str())
        }) {
            discovery.active_session_id = Some(session.cli_session_id.clone());
            discovery.active_session_signal = Some(format!(
                "Claude chat drawer snapshot at {}",
                active.observed_at_unix_ms
            ));
        }
    }
    discovery
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
        .export_chat_transcript(options.unwrap_or_default())
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn open_export_directory(output_directory: Option<String>) -> Result<String, String> {
    let directory = capture::output::prepare_export_directory(output_directory.as_deref())
        .map_err(|error| error.to_string())?;
    capture::output::open_in_file_manager(&directory).map_err(|error| error.to_string())?;
    Ok(directory.display().to_string())
}

/// Lets the UI name the real default destination instead of a relative path
/// that means nothing once the app is installed.
#[tauri::command]
fn get_default_export_directory() -> String {
    capture::output::default_export_directory()
        .display()
        .to_string()
}

#[tauri::command]
fn sanitize_filename_part(value: String) -> String {
    filename::sanitize_filename_part(&value, "Claude Session")
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // A large export takes minutes, so the pipeline's progress is
            // forwarded to the window as it happens.
            let handle = app.handle().clone();
            capture::progress::set_sink(Box::new(move |update| {
                let _ = handle.emit("export-progress", update);
            }));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            detect_claude,
            get_active_session,
            discover_local_sessions,
            get_accessibility_snapshot,
            extract_visible_text,
            save_diagnostic_snapshot,
            capture_visible_content,
            export_chat_transcript,
            open_export_directory,
            get_default_export_directory,
            sanitize_filename_part
        ])
        .run(tauri::generate_context!())
        .expect("error while running Claude Session Exporter");
}
