use std::collections::BTreeMap;

use core_foundation::base::{CFType, TCFType};
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::string::{CFString, CFStringRef};
use core_graphics::window::{
    copy_window_info, kCGNullWindowID, kCGWindowListExcludeDesktopElements,
    kCGWindowListOptionOnScreenOnly, kCGWindowName, kCGWindowOwnerName,
};
use sysinfo::System;

use crate::capture::{transcript, web_cache, CaptureAdapter, CaptureError};
use crate::models::{
    AccessibilitySnapshot, ChatExportOptions, ChatExportResult, ClaudeDetection, DetectedProcess,
    DetectedWindow, DiagnosticSaveResult, InspectorOptions, SessionMetadata, VisibleContentCapture,
    VisibleTextBlock,
};

/// Claude Desktop's main executable inside its app bundle.
///
/// Matching the bundle path rather than the process name is what separates
/// Claude Desktop from its own helper and framework processes, and from the
/// Claude Code CLI — a different program that also answers to `claude`.
const APP_EXECUTABLE_FRAGMENT: &str = "/Claude.app/Contents/MacOS/";

pub struct MacOsCaptureAdapter;

/// Finds running Claude Desktop processes. Needs no Accessibility permission.
fn find_claude_processes() -> Vec<DetectedProcess> {
    let mut system = System::new_all();
    system.refresh_all();

    let mut processes = BTreeMap::new();
    for (pid, process) in system.processes() {
        let Some(path) = process.exe().map(|path| path.display().to_string()) else {
            continue;
        };
        if !is_claude_desktop_executable(&path) {
            continue;
        }

        processes.insert(
            pid.as_u32(),
            DetectedProcess {
                pid: pid.as_u32(),
                name: process.name().to_string_lossy().to_string(),
                path: Some(path),
            },
        );
    }

    processes.into_values().collect()
}

fn is_claude_desktop_executable(path: &str) -> bool {
    path.contains(APP_EXECUTABLE_FRAGMENT)
}

/// Permission-free best-effort window titles from CoreGraphics. Modern macOS
/// may redact `kCGWindowName` without Screen Recording permission, so an empty
/// result is expected and filesystem discovery remains authoritative.
fn find_claude_windows() -> Vec<DetectedWindow> {
    let Some(info) = copy_window_info(
        kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements,
        kCGNullWindowID,
    ) else {
        return Vec::new();
    };
    let owner_key = unsafe { CFString::wrap_under_get_rule(kCGWindowOwnerName as CFStringRef) };
    let name_key = unsafe { CFString::wrap_under_get_rule(kCGWindowName as CFStringRef) };
    let mut windows = Vec::new();

    for raw in info.get_all_values() {
        let dictionary: CFDictionary<CFString, CFType> =
            unsafe { TCFType::wrap_under_get_rule(raw as CFDictionaryRef) };
        let owner = dictionary
            .find(&owner_key)
            .and_then(|value| value.downcast::<CFString>())
            .map(|value| value.to_string());
        if owner.as_deref() != Some("Claude") {
            continue;
        }
        let title = dictionary
            .find(&name_key)
            .and_then(|value| value.downcast::<CFString>())
            .map(|value| value.to_string())
            .unwrap_or_default();
        if title.trim().is_empty() {
            continue;
        }
        windows.push(DetectedWindow {
            title,
            process_id: None,
            process_name: owner,
            hwnd: None,
            class_name: None,
            visible: true,
            bounds: None,
            detection_signals: vec!["CoreGraphics on-screen Claude window".to_string()],
        });
    }
    windows
}

impl CaptureAdapter for MacOsCaptureAdapter {
    fn detect_claude(&self) -> Result<ClaudeDetection, CaptureError> {
        let processes = find_claude_processes();
        let windows = find_claude_windows();
        let detected = !processes.is_empty();

        Ok(ClaudeDetection {
            detected,
            platform: "macos-process-scan".to_string(),
            processes,
            windows,
            message: if detected {
                "Claude Desktop is running.".to_string()
            } else {
                "Claude Desktop isn't running. Start it, open the chat you want, and let it load."
                    .to_string()
            },
        })
    }

    fn get_active_session(&self) -> Result<SessionMetadata, CaptureError> {
        // Mirrors the `auto` routing below, so the title on screen names the
        // session an export would actually produce.
        if let Some((store, session_type)) =
            transcript::local_store_for_mode(web_cache::latest_shell_mode().as_ref())
        {
            return Ok(SessionMetadata {
                title: transcript::latest_session_metadata_for(store)
                    .and_then(|session| session.title),
                session_type: session_type.to_string(),
            });
        }

        let web_session = web_cache::latest_session_metadata();
        // Claude's Home tab hosts both regular chats and Cowork. Current builds
        // often persist that tab as `chat`/`task`, not `cowork`, so shell mode
        // alone mislabels the open Cowork session as the newest Home cache
        // entry. Both stores carry a focus-time filesystem signal; use the more
        // recent one when the shell says Home.
        if web_cache::latest_shell_mode() == Some(web_cache::ShellMode::Chat) {
            let cowork = transcript::latest_session_metadata_for(transcript::SessionStore::Cowork);
            if cowork.as_ref().is_some_and(|cowork| {
                cowork.observed_at_unix_ms
                    > web_session
                        .as_ref()
                        .map_or(0, |home| home.observed_at_unix_ms)
            }) {
                return Ok(SessionMetadata {
                    title: cowork.and_then(|session| session.title),
                    session_type: "cowork".to_string(),
                });
            }
        }
        Ok(SessionMetadata {
            session_type: if web_session.is_some() {
                "chat".to_string()
            } else {
                "unknown".to_string()
            },
            title: web_session.and_then(|session| session.title),
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

    fn capture_visible_content(&self) -> Result<VisibleContentCapture, CaptureError> {
        Err(CaptureError::Native(
            "macOS visible content capture is not implemented in Phase 2.".to_string(),
        ))
    }

    fn export_chat_transcript(
        &self,
        options: ChatExportOptions,
    ) -> Result<ChatExportResult, CaptureError> {
        // Three distinct stores, and the user can name one directly. Cowork
        // sessions are neither Home chats nor ordinary Claude Code: Desktop
        // metadata correlates them to their own nested JSONL transcripts.
        match options.source.as_deref().unwrap_or("auto") {
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
            _ => {}
        }

        // `auto`. Shell mode decides when it is readable; recency cannot
        // arbitrate between the stores, because viewing a Home chat rewrites its
        // cache entry while a Cowork session's files change only when worked in.
        let shell_mode = web_cache::latest_shell_mode();
        let mut result = match crate::capture::auto_plan(shell_mode.as_ref()) {
            crate::capture::AutoPlan::WebCacheOnly => web_cache::export_web_conversation(&options)?,
            crate::capture::AutoPlan::LocalStoreOnly(store) => {
                transcript::export_latest_session(store, &options)?
            }
            crate::capture::AutoPlan::WebCacheThenAnyLocal => {
                web_cache::export_web_conversation(&options).or_else(|_| {
                    transcript::export_latest_session(transcript::SessionStore::Any, &options)
                })?
            }
        };

        // An unreadable mode is the common case — Claude compacts both mode keys
        // out of local storage — and no permission-free signal replaces them.
        // Failing closed would block the only export macOS can perform, so this
        // exports and states plainly what Auto did and did not know.
        if let Some(caveat) = crate::capture::auto_source_caveat(shell_mode.as_ref()) {
            result.warnings.insert(0, caveat.to_string());
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::is_claude_desktop_executable;

    #[test]
    fn matches_the_claude_desktop_bundle_executable() {
        assert!(is_claude_desktop_executable(
            "/Applications/Claude.app/Contents/MacOS/Claude"
        ));
    }

    /// What the main screen receives on mount, against the Claude Desktop
    /// installed on this machine. Not hermetic:
    ///
    /// ```text
    /// cargo test -- --ignored --nocapture live_claude_desktop
    /// ```
    #[test]
    #[ignore = "requires Claude Desktop running on this machine"]
    fn reports_live_claude_desktop_status() {
        use crate::capture::CaptureAdapter;

        let detection = super::MacOsCaptureAdapter.detect_claude().unwrap();
        eprintln!(
            "detected={} platform={} message={:?}",
            detection.detected, detection.platform, detection.message
        );
        for process in &detection.processes {
            eprintln!("  pid {} {:?}", process.pid, process.path);
        }
        for window in &detection.windows {
            eprintln!("  window {:?}", window.title);
        }
        assert!(
            detection.detected,
            "Claude Desktop must be running for this check"
        );

        let session = super::MacOsCaptureAdapter.get_active_session().unwrap();
        let home = crate::capture::web_cache::latest_session_metadata();
        let cowork = crate::capture::transcript::latest_session_metadata_for(
            crate::capture::transcript::SessionStore::Cowork,
        );
        eprintln!(
            "session_type={} title={:?} home_seen={:?} cowork_seen={:?}",
            session.session_type,
            session.title,
            home.map(|session| session.observed_at_unix_ms),
            cowork.map(|session| session.observed_at_unix_ms),
        );
    }

    /// Helper, framework, and CLI processes must not count as Claude Desktop.
    #[test]
    fn rejects_helpers_frameworks_and_the_claude_code_cli() {
        for path in [
            "/Applications/Claude.app/Contents/Frameworks/Claude",
            "/Applications/Claude.app/Contents/Frameworks/Electron",
            "/Applications/Claude.app/Contents/Helpers/chrome-native-host",
            "/opt/homebrew/bin/claude",
            "/Users/someone/.local/share/claude-code/claude",
        ] {
            assert!(!is_claude_desktop_executable(path), "accepted: {path}");
        }
    }
}
