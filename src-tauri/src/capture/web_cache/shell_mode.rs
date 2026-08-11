//! Reads which shell Claude Desktop is currently showing.
//!
//! This is what stops `auto` from exporting a stale Claude Code transcript while
//! a Home chat is open, so it reads two independent signals from renderer local
//! storage rather than one:
//!
//! - `lastKnownMode`, inside the `dframe-store` JSON blob.
//! - `sidebar-selected-mode`, a bare JSON string.
//!
//! Neither is guaranteed to be present, and often neither is. Both keys are
//! written to LevelDB and compacted out again as Claude Desktop runs: a profile
//! observed on 2026-08-11 carried `sidebar-selected-mode` but not
//! `dframe-store`, then dropped both within the hour. Reading both is strictly
//! better than reading one, but callers must treat `None` as the common case and
//! degrade to a safe default rather than assuming a mode.

use std::fs;
use std::time::UNIX_EPOCH;

use super::paths;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellMode {
    Chat,
    Code,
    Other(String),
}

pub fn latest_shell_mode() -> Option<ShellMode> {
    for root in paths::local_storage_dirs() {
        let mut files = fs::read_dir(&root)
            .ok()?
            .filter_map(Result::ok)
            .filter(|entry| entry.path().is_file())
            .collect::<Vec<_>>();
        // LevelDB rotates its files, so the newest write wins.
        files.sort_by_key(|entry| {
            entry
                .metadata()
                .ok()
                .and_then(|metadata| metadata.modified().ok())
                .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_millis())
                .unwrap_or(0)
        });
        files.reverse();

        for entry in files {
            if entry.file_name() == "LOCK" {
                continue;
            }
            let Ok(bytes) = fs::read(entry.path()) else {
                continue;
            };
            // LevelDB stores local storage values as either UTF-8 or UTF-16LE.
            for text in [
                String::from_utf8_lossy(&bytes).to_string(),
                decode_utf16le_lossy(&bytes),
            ] {
                if let Some(mode) = extract_shell_mode(&text) {
                    return Some(mode);
                }
            }
        }
    }

    None
}

fn decode_utf16le_lossy(bytes: &[u8]) -> String {
    let units = bytes
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]));
    char::decode_utf16(units)
        .map(|result| result.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
}

fn extract_shell_mode(text: &str) -> Option<ShellMode> {
    extract_last_known_mode(text).or_else(|| extract_sidebar_selected_mode(text))
}

/// A LevelDB file keeps superseded copies of a key, and the last occurrence is
/// often a block-index entry with no value behind it. Both extractors therefore
/// walk every occurrence newest-first rather than trusting `rfind` alone.
fn extract_last_known_mode(text: &str) -> Option<ShellMode> {
    // `str` match iterators are not double-ended, so collect then walk back.
    let matches: Vec<_> = text.match_indices("\"lastKnownMode\":\"").collect();
    matches.into_iter().rev().find_map(|(at, marker)| {
        let start = at + marker.len();
        let end = text[start..].find('"')? + start;
        (end > start).then(|| classify(&text[start..end]))
    })
}

/// Reads the bare `sidebar-selected-mode` value.
///
/// Its value is a JSON string, but LevelDB's record framing sits between the key
/// and the value and can clip the quotes, so this scans a short window after the
/// key for a known mode word instead of parsing a quoted token.
fn extract_sidebar_selected_mode(text: &str) -> Option<ShellMode> {
    const WINDOW_CHARS: usize = 32;

    let matches: Vec<_> = text.match_indices("sidebar-selected-mode").collect();
    matches.into_iter().rev().find_map(|(at, marker)| {
        let window: String = text[at + marker.len()..].chars().take(WINDOW_CHARS).collect();
        ["chat", "home", "task", "code"]
            .into_iter()
            .filter_map(|mode| find_json_string_value(&window, mode).map(|found_at| (found_at, mode)))
            .min_by_key(|(found_at, _)| *found_at)
            .map(|(_, mode)| classify(mode))
    })
}

/// Locates `mode` as an actual JSON string value inside `window`.
///
/// A bare substring search is not safe here: the window is raw LevelDB bytes, so
/// an adjacent key such as `code-session-watch`, or a word like `"encoded"`,
/// would otherwise register as a mode. Requiring the opening quote of the value
/// and a non-word character after it rules both out. The closing quote is not
/// required — record framing frequently clips it.
fn find_json_string_value(window: &str, mode: &str) -> Option<usize> {
    let needle = format!("\"{mode}");
    let mut from = 0;

    while let Some(relative) = window[from..].find(&needle) {
        let at = from + relative;
        let after = window[at + needle.len()..].chars().next();
        if after.is_none_or(|character| !character.is_alphanumeric() && !"-_".contains(character)) {
            return Some(at);
        }
        from = at + needle.len();
    }

    None
}

fn classify(mode: &str) -> ShellMode {
    match mode {
        "chat" | "home" | "task" => ShellMode::Chat,
        "code" => ShellMode::Code,
        other => ShellMode::Other(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        decode_utf16le_lossy, extract_last_known_mode, extract_shell_mode,
        extract_sidebar_selected_mode, ShellMode,
    };

    /// Byte-for-byte from a live profile, LevelDB framing included: the closing
    /// quote of the JSON value is clipped by the next record's header.
    const LIVE_SIDEBAR_RECORD: &str =
        "\u{1d}Zsidebar-selected-mode\u{1}\u{94}h:\u{11}\u{4}\u{10}\"task!.";

    #[test]
    fn falls_back_to_sidebar_selected_mode() {
        assert_eq!(
            extract_sidebar_selected_mode(LIVE_SIDEBAR_RECORD),
            Some(ShellMode::Chat)
        );
        // The compacted profile that motivated this has no dframe-store at all.
        assert_eq!(extract_last_known_mode(LIVE_SIDEBAR_RECORD), None);
        assert_eq!(extract_shell_mode(LIVE_SIDEBAR_RECORD), Some(ShellMode::Chat));
    }

    #[test]
    fn prefers_dframe_store_when_both_signals_exist() {
        let text = format!("{LIVE_SIDEBAR_RECORD} {{\"lastKnownMode\":\"code\"}}");
        assert_eq!(extract_shell_mode(&text), Some(ShellMode::Code));
    }

    #[test]
    fn reads_code_from_the_sidebar_signal() {
        assert_eq!(
            extract_sidebar_selected_mode("sidebar-selected-mode\u{1}\u{94}\"code\""),
            Some(ShellMode::Code)
        );
    }

    /// The window is raw LevelDB bytes, so neighbouring keys and ordinary words
    /// containing a mode name must not register as the value. A false reading is
    /// worse than none: on macOS a spurious `Code` makes `auto` refuse to export.
    #[test]
    fn ignores_mode_words_that_are_not_json_values() {
        for text in [
            // An adjacent LevelDB key that happens to start with a mode word.
            "sidebar-selected-mode\u{1}\u{94}\u{11}code-session-watch",
            // A mode word inside a longer word.
            "sidebar-selected-mode\u{1}\u{94}\u{11}\"encoded-thing\"",
            "sidebar-selected-mode\u{1}\u{94}\u{11}\"chats\"",
            // Framing bytes only, no value at all.
            "sidebar-selected-mode\u{1}\u{94}h:\u{11}\u{4}\u{10}",
        ] {
            assert_eq!(
                extract_sidebar_selected_mode(text),
                None,
                "false reading from: {text:?}"
            );
        }
    }

    /// A trailing unrelated record must not be read as the mode.
    #[test]
    fn ignores_mode_words_beyond_the_value_window() {
        let text = format!("sidebar-selected-mode{}code", " ".repeat(64));
        assert_eq!(extract_sidebar_selected_mode(&text), None);
    }

    /// A compacted LevelDB file repeats the key; the final copy is usually a
    /// valueless index entry, so a live value earlier in the file must win.
    #[test]
    fn skips_valueless_copies_of_a_repeated_key() {
        let text = format!(
            "{LIVE_SIDEBAR_RECORD}{}sidebar-selected-mode{}",
            " ".repeat(80),
            " ".repeat(80)
        );
        assert_eq!(extract_sidebar_selected_mode(&text), Some(ShellMode::Chat));

        let dframe = format!(
            "{{\"lastKnownMode\":\"code\"}}{}\"lastKnownMode\":\"",
            " ".repeat(80)
        );
        assert_eq!(extract_last_known_mode(&dframe), Some(ShellMode::Code));
    }

    #[test]
    fn reads_the_most_recent_mode_in_a_log_file() {
        let text = r#"old {"lastKnownMode":"code"} new {"lastKnownMode":"chat"}"#;
        assert_eq!(extract_last_known_mode(text), Some(ShellMode::Chat));
    }

    #[test]
    fn maps_home_and_task_onto_chat() {
        assert_eq!(
            extract_last_known_mode(r#"{"lastKnownMode":"task"}"#),
            Some(ShellMode::Chat)
        );
        assert_eq!(
            extract_last_known_mode(r#"{"lastKnownMode":"code"}"#),
            Some(ShellMode::Code)
        );
    }

    #[test]
    fn preserves_unrecognized_modes() {
        assert_eq!(
            extract_last_known_mode(r#"{"lastKnownMode":"canvas"}"#),
            Some(ShellMode::Other("canvas".to_string()))
        );
        assert_eq!(extract_last_known_mode("no mode here"), None);
    }

    #[test]
    fn decodes_utf16le_values() {
        let utf16: Vec<u8> = r#"{"lastKnownMode":"chat"}"#
            .encode_utf16()
            .flat_map(|unit| unit.to_le_bytes())
            .collect();
        assert_eq!(
            extract_last_known_mode(&decode_utf16le_lossy(&utf16)),
            Some(ShellMode::Chat)
        );
    }
}
