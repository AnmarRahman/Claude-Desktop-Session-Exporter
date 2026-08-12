//! Reads Claude's most recently focused chat-drawer conversation.
//!
//! Claude persists `chat-drawer-snapshot-store` in Chromium Session Storage.
//! Each snapshot is keyed by a Home conversation UUID or Cowork `local_*` ID
//! and carries an `at` timestamp. The greatest timestamp is a substantially
//! stronger active-session signal than cache or transcript mtimes.

use std::collections::HashMap;
use std::fs;
use std::time::UNIX_EPOCH;

use serde::Deserialize;

use super::paths;

const STORE_MARKER: &str = "chat-drawer-snapshot-store";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveDrawerSession {
    pub session_id: String,
    pub observed_at_unix_ms: u64,
}

#[derive(Debug, Deserialize)]
struct SnapshotEnvelope {
    state: SnapshotState,
}

#[derive(Debug, Deserialize)]
struct SnapshotState {
    snapshots: HashMap<String, DrawerSnapshot>,
}

#[derive(Debug, Deserialize)]
struct DrawerSnapshot {
    at: u64,
}

pub fn latest_active_drawer_session() -> Option<ActiveDrawerSession> {
    for root in paths::session_storage_dirs() {
        let mut files = fs::read_dir(root)
            .ok()?
            .filter_map(Result::ok)
            .filter(|entry| entry.path().is_file() && entry.file_name() != "LOCK")
            .collect::<Vec<_>>();
        files.sort_by_key(file_modified_unix_ms);
        files.reverse();

        for entry in files {
            let Ok(bytes) = fs::read(entry.path()) else {
                continue;
            };
            // Session Storage values observed on macOS are UTF-16LE embedded in
            // LevelDB records. Removing NUL bytes also leaves ordinary UTF-8
            // records intact, and avoids depending on record alignment.
            let collapsed: Vec<u8> = bytes.into_iter().filter(|byte| *byte != 0).collect();
            let text = String::from_utf8_lossy(&collapsed);
            if let Some(session) = extract_active_drawer_session(&text) {
                return Some(session);
            }
        }
    }
    None
}

fn file_modified_unix_ms(entry: &fs::DirEntry) -> u128 {
    entry
        .metadata()
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

fn extract_active_drawer_session(text: &str) -> Option<ActiveDrawerSession> {
    let occurrences: Vec<_> = text.match_indices(STORE_MARKER).collect();
    occurrences.into_iter().rev().find_map(|(at, marker)| {
        let after_marker = &text[at + marker.len()..];
        let object_start = after_marker.find('{')?;
        let json = balanced_json_object(&after_marker[object_start..])?;
        let envelope: SnapshotEnvelope = serde_json::from_str(json).ok()?;
        envelope
            .state
            .snapshots
            .into_iter()
            .filter(|(id, _)| !id.trim().is_empty())
            .max_by_key(|(_, snapshot)| snapshot.at)
            .map(|(session_id, snapshot)| ActiveDrawerSession {
                session_id,
                observed_at_unix_ms: snapshot.at,
            })
    })
}

fn balanced_json_object(text: &str) -> Option<&str> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (index, character) in text.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }
        match character {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(&text[..=index]);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::extract_active_drawer_session;

    #[test]
    fn picks_the_snapshot_with_the_greatest_focus_timestamp() {
        let text = r#"chat-drawer-snapshot-store framing {"state":{"snapshots":{"home-id":{"at":10},"local_cowork":{"at":20}}}}"#;
        let session = extract_active_drawer_session(text).unwrap();
        assert_eq!(session.session_id, "local_cowork");
        assert_eq!(session.observed_at_unix_ms, 20);
    }

    #[test]
    fn skips_a_newer_valueless_leveldb_index_entry() {
        let text = r#"chat-drawer-snapshot-store {"state":{"snapshots":{"local_one":{"at":30}}}} trailing chat-drawer-snapshot-store"#;
        assert_eq!(
            extract_active_drawer_session(text).unwrap().session_id,
            "local_one"
        );
    }

    #[test]
    fn respects_braces_inside_json_strings() {
        let text = r#"chat-drawer-snapshot-store {"state":{"snapshots":{"strange}id":{"at":40}}}}"#;
        assert_eq!(
            extract_active_drawer_session(text).unwrap().session_id,
            "strange}id"
        );
    }

    #[test]
    #[ignore = "requires a running Claude Desktop profile"]
    fn reads_the_real_active_drawer_session() {
        let session = super::latest_active_drawer_session()
            .expect("Claude did not persist a chat-drawer snapshot");
        eprintln!(
            "active drawer session={} observed_at={}",
            session.session_id, session.observed_at_unix_ms
        );
        assert!(!session.session_id.is_empty());
        assert!(session.observed_at_unix_ms > 0);
    }
}
