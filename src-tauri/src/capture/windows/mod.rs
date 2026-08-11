mod process;
mod transcript;
mod uia;
mod visual;
mod web_cache;

use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::capture::{claude, CaptureAdapter, CaptureError};
use crate::models::{
    AccessibilitySnapshot, ChatExportOptions, ChatExportResult, ClaudeDetection,
    DiagnosticSaveResult, InspectorOptions, SessionMetadata, VisibleContentCapture,
    VisibleTextBlock,
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
        let window_title = uia::find_claude_windows(&processes)?
            .into_iter()
            .find_map(|window| claude::detection::clean_claude_window_title(&window.title));
        let shell_mode = web_cache::latest_shell_mode();
        let web_session = web_cache::latest_session_metadata();
        let local_session = transcript::latest_session_metadata();
        let is_chat_mode = shell_mode == Some(web_cache::ShellMode::Chat);

        Ok(SessionMetadata {
            title: window_title.or_else(|| {
                web_session
                    .as_ref()
                    .and_then(|session| session.title.clone())
                    .or_else(|| {
                        if is_chat_mode {
                            None
                        } else {
                            local_session
                                .as_ref()
                                .and_then(|session| session.title.clone())
                        }
                    })
            }),
            session_type: if is_chat_mode || web_session.is_some() {
                "chat".to_string()
            } else if local_session.is_some() {
                "cowork".to_string()
            } else {
                "unknown".to_string()
            },
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
        let processes = process::find_claude_processes();
        let windows = uia::find_claude_windows(&processes)?;
        let snapshot = claude::conversation::analyze_snapshot(uia::inspect_first_claude_window(
            &processes,
            options.clone(),
        )?);
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
        let diagnostic = serde_json::json!({
            "captured_at_unix": timestamp,
            "platform": "windows-uiautomation",
            "inspector_options": options,
            "processes": processes,
            "windows": windows,
            "snapshot": snapshot,
        });
        let json = serde_json::to_string_pretty(&diagnostic)
            .map_err(|error| CaptureError::Diagnostic(error.to_string()))?;
        fs::write(&path, json).map_err(|error| CaptureError::Diagnostic(error.to_string()))?;

        Ok(DiagnosticSaveResult {
            path: path.display().to_string(),
            warning: "Diagnostic snapshots can contain Claude window metadata and any accessible conversation text. They remain on this computer.".to_string(),
        })
    }

    fn capture_visible_content(&self) -> Result<VisibleContentCapture, CaptureError> {
        let processes = process::find_claude_processes();
        let Some(window) = uia::find_claude_windows(&processes)?.into_iter().next() else {
            return Err(CaptureError::Native(
                "Claude Desktop window was not found.".to_string(),
            ));
        };
        let diagnostics_dir = std::env::current_dir()
            .map_err(|error| CaptureError::Diagnostic(error.to_string()))?
            .join("diagnostics");
        fs::create_dir_all(&diagnostics_dir)
            .map_err(|error| CaptureError::Diagnostic(error.to_string()))?;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| CaptureError::Diagnostic(error.to_string()))?
            .as_secs();
        let path = diagnostics_dir.join(format!("claude-visible-content-{timestamp}.bmp"));

        visual::capture_window_to_bmp(&window, &path)
    }

    fn export_chat_transcript(
        &self,
        options: ChatExportOptions,
    ) -> Result<ChatExportResult, CaptureError> {
        let shell_mode = web_cache::latest_shell_mode();
        match options.source.as_deref().unwrap_or("auto") {
            "home" => web_cache::export_latest_web_cache_transcript(),
            "code" => transcript::export_latest_code_transcript(),
            _ if shell_mode == Some(web_cache::ShellMode::Chat) => {
                web_cache::export_latest_web_cache_transcript()
            }
            _ if shell_mode == Some(web_cache::ShellMode::Code) => {
                transcript::export_latest_code_transcript()
                    .or_else(|_| web_cache::export_latest_web_cache_transcript())
            }
            _ => web_cache::export_latest_web_cache_transcript()
                .or_else(|_| transcript::export_latest_code_transcript()),
        }
    }
}
