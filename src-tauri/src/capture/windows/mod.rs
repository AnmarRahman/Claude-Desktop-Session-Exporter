mod process;
mod uia;
mod visual;

use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::capture::{claude, transcript, web_cache, CaptureAdapter, CaptureError};
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
        let is_chat_mode = shell_mode == Some(web_cache::ShellMode::Chat);
        // Cowork and Claude Code are separate stores under separate mode names.
        // Reading `Any` here would let a Cowork session answer for Claude Code
        // and the other way round, since neither store can be identified after
        // the fact.
        let local_store = transcript::local_store_for_mode(shell_mode.as_ref());
        let local_session = transcript::latest_session_metadata_for(
            local_store.map_or(transcript::SessionStore::Any, |(store, _)| store),
        );

        Ok(SessionMetadata {
            // The title must agree with the `session_type` below: when the mode
            // names a local store, a web-cache title would label a Home chat as
            // Cowork or Claude Code.
            title: crate::capture::active_session_title(
                window_title,
                web_session
                    .as_ref()
                    .and_then(|session| session.title.clone()),
                local_session
                    .as_ref()
                    .and_then(|session| session.title.clone()),
                local_store.is_some(),
                is_chat_mode,
            ),
            session_type: match local_store {
                Some((_, session_type)) => session_type.to_string(),
                None if is_chat_mode || web_session.is_some() => "chat".to_string(),
                None if local_session.is_some() => "cowork".to_string(),
                None => "unknown".to_string(),
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
        // `auto` must never fall back to a stale Claude Code transcript while
        // Claude Desktop is showing Home/Chat.
        let shell_mode = web_cache::latest_shell_mode();
        let mut result = match options.source.as_deref().unwrap_or("auto") {
            "home" => return web_cache::export_web_conversation(&options),
            "cowork" => {
                return transcript::export_session(
                    transcript::SessionStore::Cowork,
                    options.conversation_id.as_deref(),
                    &options,
                )
            }
            "code" => {
                return transcript::export_session(
                    transcript::SessionStore::ClaudeCode,
                    options.conversation_id.as_deref(),
                    &options,
                )
            }
            // Identical routing to macOS, by construction.
            _ => match crate::capture::auto_plan(shell_mode.as_ref()) {
                crate::capture::AutoPlan::WebCacheOnly => {
                    web_cache::export_web_conversation(&options)
                }
                crate::capture::AutoPlan::LocalStoreOnly(store) => {
                    transcript::export_latest_session(store, &options)
                }
                crate::capture::AutoPlan::WebCacheThenAnyLocal => {
                    web_cache::export_web_conversation(&options).or_else(|_| {
                        transcript::export_latest_session(transcript::SessionStore::Any, &options)
                    })
                }
            },
        }?;

        // Both mode keys are frequently compacted out of local storage, and even
        // a mode that reads cleanly names only a store. Say which of the two
        // applies rather than implying the choice was fully informed.
        if let Some(caveat) = crate::capture::auto_source_caveat(shell_mode.as_ref()) {
            result.warnings.insert(0, caveat.to_string());
        }

        Ok(result)
    }
}
