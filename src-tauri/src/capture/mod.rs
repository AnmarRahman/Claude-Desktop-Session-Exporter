use crate::models::{
    AccessibilitySnapshot, ChatExportOptions, ChatExportResult, ClaudeDetection,
    DiagnosticSaveResult, InspectorOptions, SessionMetadata, VisibleContentCapture,
    VisibleTextBlock,
};
use thiserror::Error;

pub mod claude;
/// Filesystem-first Claude Desktop Cowork discovery and JSONL correlation.
pub mod cowork;
#[cfg(target_os = "macos")]
mod macos;
/// Resolves and opens transcript export destinations.
pub mod output;
/// Native PDF rendering for normalized transcript exports.
pub mod pdf;
/// Reads Claude Code and Cowork sessions from local JSONL transcripts.
pub mod transcript;
#[cfg(not(any(windows, target_os = "macos")))]
mod unsupported;
/// Reads Claude Home conversations from the local Chromium profile.
/// Platform-neutral: the cache format is the same everywhere.
pub mod web_cache;
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
    fn capture_visible_content(&self) -> Result<VisibleContentCapture, CaptureError>;
    fn export_chat_transcript(
        &self,
        options: ChatExportOptions,
    ) -> Result<ChatExportResult, CaptureError>;
}

/// `auto` could not read the mode, so the *store* was a guess.
pub const UNREADABLE_MODE_CAVEAT: &str = "Claude Desktop's current mode could not be read, so Auto picked this source without knowing which session is on screen. Check the title below; if you wanted a Cowork or Claude Code session, choose that source explicitly and retry.";

/// `auto` knew the store but not the session inside it.
pub const NEWEST_IN_STORE_CAVEAT: &str = "Auto exported the most recently active session in this store, which is not necessarily the one on screen — a Cowork or Claude Code session's files only change when it is worked in. Check the title below before relying on this export.";

/// How `auto` may resolve a source, given the mode it read.
///
/// This exists so both platform adapters run the *same* routing. They drifted
/// once already: Windows fell back to the web cache when a local-store export
/// failed while macOS propagated the error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoPlan {
    /// Home chat — the web cache is the only place it lives.
    WebCacheOnly,
    /// Cowork or Claude Code, with **no web-cache fallback**. Those sessions are
    /// not in the web cache at all, so falling back cannot find the session that
    /// failed to load — it can only succeed with an unrelated Home chat, under a
    /// caveat that claims this store. Failing is the honest outcome.
    LocalStoreOnly(transcript::SessionStore),
    /// Mode unreadable or unrecognized, so nothing rules either store out.
    WebCacheThenAnyLocal,
}

/// Routes a shell mode to the store(s) `auto` is allowed to read.
pub fn auto_plan(mode: Option<&web_cache::ShellMode>) -> AutoPlan {
    // Derived from the same mapping `session_type` uses, so the exported store
    // and the reported session type cannot disagree.
    if let Some((store, _)) = transcript::local_store_for_mode(mode) {
        return AutoPlan::LocalStoreOnly(store);
    }
    match mode {
        Some(web_cache::ShellMode::Chat) => AutoPlan::WebCacheOnly,
        _ => AutoPlan::WebCacheThenAnyLocal,
    }
}

/// The caveat `auto` owes the user for how it chose, given the mode it read.
///
/// Two distinct gaps, and conflating them helps nobody:
///
/// - An unreadable or unrecognized mode means the *store* was guessed.
/// - A readable `code`/`cowork` mode names a store but never a session. Those
///   stores are ordered by recency, and a local session's files change only when
///   it is worked in, so the newest is not reliably the one on screen.
///
/// `Chat` alone earns silence: opening a Home chat rewrites its web-cache entry,
/// so within that store recency really does track what the user has open. See
/// docs/CLAUDE_UI_ASSUMPTIONS.md.
pub fn auto_source_caveat(mode: Option<&web_cache::ShellMode>) -> Option<&'static str> {
    match mode {
        None | Some(web_cache::ShellMode::Other(_)) => Some(UNREADABLE_MODE_CAVEAT),
        Some(web_cache::ShellMode::Code) | Some(web_cache::ShellMode::Cowork) => {
            Some(NEWEST_IN_STORE_CAVEAT)
        }
        Some(web_cache::ShellMode::Chat) => None,
    }
}

/// Picks the title to show for the session currently on screen.
///
/// Precedence is not fixed: when the mode names a local store, that store's
/// title is the only one consistent with the `session_type` reported alongside
/// it. Preferring the web cache there pairs a Home chat's title with a
/// `"cowork"`/`"code"` type, which is worse than no title at all.
///
/// Only Windows can supply `window_title`; it wins when present because it is
/// read from the window actually on screen.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn active_session_title(
    window_title: Option<String>,
    web_title: Option<String>,
    local_title: Option<String>,
    names_local_store: bool,
    is_chat_mode: bool,
) -> Option<String> {
    window_title.or({
        if names_local_store {
            local_title
        } else if is_chat_mode {
            web_title
        } else {
            web_title.or(local_title)
        }
    })
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

#[cfg(test)]
mod auto_routing_tests {
    use super::transcript::SessionStore;
    use super::web_cache::ShellMode;
    use super::{
        active_session_title, auto_plan, auto_source_caveat, AutoPlan, NEWEST_IN_STORE_CAVEAT,
        UNREADABLE_MODE_CAVEAT,
    };

    /// The regression this guards: Windows used to fall back to the web cache
    /// when a Cowork/Claude Code export failed. Those sessions are not in the
    /// web cache, so the fallback could only ever succeed with an unrelated Home
    /// chat — reported under a caveat naming the local store.
    #[test]
    fn local_store_modes_have_no_web_cache_fallback() {
        assert_eq!(
            auto_plan(Some(&ShellMode::Cowork)),
            AutoPlan::LocalStoreOnly(SessionStore::Cowork)
        );
        assert_eq!(
            auto_plan(Some(&ShellMode::Code)),
            AutoPlan::LocalStoreOnly(SessionStore::ClaudeCode)
        );
    }

    /// A Home chat lives only in the web cache, so a local store cannot stand in
    /// for it either.
    #[test]
    fn chat_mode_reads_only_the_web_cache() {
        assert_eq!(auto_plan(Some(&ShellMode::Chat)), AutoPlan::WebCacheOnly);
    }

    /// Nothing is ruled out when the mode is unreadable, and the caveat says so.
    #[test]
    fn unreadable_mode_may_try_either_store() {
        assert_eq!(auto_plan(None), AutoPlan::WebCacheThenAnyLocal);
        assert_eq!(
            auto_plan(Some(&ShellMode::Other("canvas".to_string()))),
            AutoPlan::WebCacheThenAnyLocal
        );
    }

    /// The plan and the caveat are read together, so they must agree: any mode
    /// whose caveat asserts a specific store must be routed to that store alone.
    #[test]
    fn a_store_asserting_caveat_implies_a_single_store_plan() {
        for mode in [
            ShellMode::Chat,
            ShellMode::Code,
            ShellMode::Cowork,
            ShellMode::Other("canvas".to_string()),
            ShellMode::Other(String::new()),
        ] {
            let asserts_a_store = auto_source_caveat(Some(&mode)) == Some(NEWEST_IN_STORE_CAVEAT);
            let single_store = matches!(auto_plan(Some(&mode)), AutoPlan::LocalStoreOnly(_));
            assert_eq!(
                asserts_a_store, single_store,
                "{mode:?} promises one store but may read another"
            );
        }
    }

    /// Reading `cowork`/`code` identifies the store, never the session inside
    /// it, so `auto` must still tell the user to check the title. Dropping this
    /// warning when the mode became routable is what this guards against.
    #[test]
    fn local_store_modes_still_warn_about_which_session() {
        assert_eq!(
            auto_source_caveat(Some(&ShellMode::Cowork)),
            Some(NEWEST_IN_STORE_CAVEAT)
        );
        assert_eq!(
            auto_source_caveat(Some(&ShellMode::Code)),
            Some(NEWEST_IN_STORE_CAVEAT)
        );
    }

    /// A missing key and a mode this build does not know are equally blind.
    #[test]
    fn unreadable_and_unknown_modes_warn_about_the_store() {
        assert_eq!(auto_source_caveat(None), Some(UNREADABLE_MODE_CAVEAT));
        assert_eq!(
            auto_source_caveat(Some(&ShellMode::Other("canvas".to_string()))),
            Some(UNREADABLE_MODE_CAVEAT)
        );
    }

    /// Opening a Home chat rewrites its cache entry, so recency does track the
    /// open conversation there. A warning that always fires teaches users to
    /// ignore it.
    #[test]
    fn chat_mode_needs_no_caveat() {
        assert_eq!(auto_source_caveat(Some(&ShellMode::Chat)), None);
    }

    /// The two caveats say different things and must not be interchanged.
    #[test]
    fn the_two_caveats_are_distinct() {
        assert_ne!(UNREADABLE_MODE_CAVEAT, NEWEST_IN_STORE_CAVEAT);
    }

    fn titles() -> (Option<String>, Option<String>) {
        (
            Some("Home chat".to_string()),
            Some("Local session".to_string()),
        )
    }

    /// A `cowork`/`code` session type paired with a Home chat title is the bug
    /// this ordering exists to prevent.
    #[test]
    fn local_store_modes_never_show_a_web_cache_title() {
        let (web, local) = titles();
        assert_eq!(
            active_session_title(None, web.clone(), local.clone(), true, false),
            local
        );
        // Even with no local title, the Home chat title must not stand in.
        assert_eq!(active_session_title(None, web, None, true, false), None);
    }

    #[test]
    fn chat_mode_shows_the_web_cache_title() {
        let (web, local) = titles();
        assert_eq!(
            active_session_title(None, web.clone(), local, false, true),
            web
        );
    }

    /// Unknown mode: the web cache is the better guess, but a local session is
    /// better than nothing.
    #[test]
    fn unknown_mode_prefers_web_then_falls_back_to_local() {
        let (web, local) = titles();
        assert_eq!(
            active_session_title(None, web.clone(), local.clone(), false, false),
            web
        );
        assert_eq!(
            active_session_title(None, None, local.clone(), false, false),
            local
        );
    }

    /// The window on screen outranks every stored guess.
    #[test]
    fn a_window_title_wins_over_every_store() {
        let (web, local) = titles();
        let window = Some("On screen".to_string());
        for names_local_store in [true, false] {
            assert_eq!(
                active_session_title(
                    window.clone(),
                    web.clone(),
                    local.clone(),
                    names_local_store,
                    false
                ),
                window
            );
        }
    }
}
