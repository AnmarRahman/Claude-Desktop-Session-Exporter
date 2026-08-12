//! Reads regular Claude Home conversations out of Claude Desktop's local
//! Chromium profile.
//!
//! Claude Desktop fetches a conversation over
//! `GET /api/organizations/<org>/chat_conversations/<uuid>?tree=True&rendering_mode=messages`,
//! and the renderer's HTTP disk cache keeps the full JSON response. That cache is
//! the transcript source: the messages are not in Local Storage or IndexedDB.
//!
//! Everything here is platform-neutral in structure, but only **verified on
//! macOS**. It implements Chromium's simple-cache format, which is what Claude
//! Desktop uses there. Chromium's other HTTP backend, blockfile, has a different
//! layout; if a profile uses it the failure message says so instead of reporting
//! an empty cache (see [`paths::detect_backend`]). Whether Windows Claude uses
//! simple or blockfile has not been checked on a real installation.
//!
//! This reads files the user's own Claude Desktop wrote. Nothing is fetched or
//! uploaded.

mod conversation;
mod decode;
mod export;
pub(crate) mod paths;
mod session_state;
mod shell_mode;
mod simple_cache;

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

use crate::capture::CaptureError;
use crate::models::{ChatExportOptions, ChatExportResult, LocalSessionSummary};

pub const SOURCE_HOME: &str = "Claude Home Chat";

pub use conversation::Conversation;
pub use session_state::latest_active_drawer_session;
pub use shell_mode::{latest_shell_mode, ShellMode};

/// How many stored versions of one conversation to try before giving up.
/// Chromium keeps a handful as a conversation grows.
const MAX_VERSIONS_PER_CONVERSATION: usize = 5;

/// A cached conversation, located but not yet decoded.
#[derive(Debug, Clone)]
pub struct CachedConversationRef {
    pub uuid: String,
    pub entry_path: PathBuf,
    pub modified_unix_ms: u128,
    pub entry_bytes: u64,
    /// Status of the cached response. `None` when it could not be read.
    pub http_status: Option<u16>,
}

impl CachedConversationRef {
    /// A cached error response is a dead entry, not a transcript. Selecting one
    /// as the export target would hide every conversation behind it.
    fn is_usable(&self) -> bool {
        !matches!(self.http_status, Some(status) if status != 200)
    }
}

#[derive(Debug, Clone)]
pub struct LatestSessionMetadata {
    pub title: Option<String>,
    pub observed_at_unix_ms: u64,
}

/// One entry per conversation, most recently written first.
///
/// Opening a conversation in Claude Desktop refreshes its cache entry, so the
/// first element is the conversation the user most recently had open.
///
/// Export deliberately does not use this — it needs every stored version of one
/// conversation, not one entry per conversation. This is the listing a
/// conversation picker needs.
#[allow(dead_code)]
pub fn list_cached_conversations() -> Vec<CachedConversationRef> {
    let mut conversations = list_cache_entries();
    let mut seen = std::collections::HashSet::new();
    conversations.retain(|candidate| seen.insert(candidate.uuid.clone()));
    conversations
}

/// Every readable Home conversation currently retained by Claude Desktop's
/// cache, one row per conversation UUID. This is best-effort because Chromium
/// may evict old responses; only device-local cached conversations are listed.
pub fn list_home_sessions() -> Vec<LocalSessionSummary> {
    let entries = list_cache_entries();
    let mut seen = std::collections::HashSet::new();
    let newest: Vec<&CachedConversationRef> = entries
        .iter()
        .filter(|entry| entry.is_usable() && seen.insert(entry.uuid.clone()))
        .collect();
    let workers = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(4)
        .clamp(1, 8);
    let chunk_size = newest.len().div_ceil(workers).max(1);

    std::thread::scope(|scope| {
        let handles: Vec<_> = newest
            .chunks(chunk_size)
            .map(|chunk| {
                let entries = &entries;
                scope.spawn(move || {
                    chunk
                        .iter()
                        .filter_map(|newest| home_summary(newest, entries))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().unwrap_or_else(|_| Vec::new()))
            .collect()
    })
}

fn home_summary(
    newest: &CachedConversationRef,
    entries: &[CachedConversationRef],
) -> Option<LocalSessionSummary> {
    let conversation = entries
        .iter()
        .filter(|entry| entry.uuid == newest.uuid && entry.is_usable())
        .take(MAX_VERSIONS_PER_CONVERSATION)
        .find_map(|entry| load_expected_conversation(entry).ok())?;
    let title = conversation
        .display_title()
        .unwrap_or_else(|| conversation.uuid.clone());
    Some(LocalSessionSummary {
        // Kept as the canonical local ID for compatibility with the JSONL
        // session picker. Home export interprets it as a conversation UUID.
        cli_session_id: conversation.uuid,
        desktop_session_id: None,
        title,
        source_type: SOURCE_HOME.to_string(),
        transcript_available: true,
        transcript_path: Some(newest.entry_path.display().to_string()),
        metadata_path: None,
        metadata_modified_at: None,
        cwd: None,
        origin_cwd: None,
        model: conversation.model,
        effort: None,
        created_at: None,
        last_activity_at: Some(newest.modified_unix_ms as u64),
        last_focused_at: Some(newest.modified_unix_ms as u64),
        is_archived: None,
        title_source: Some("Claude Desktop HTTP cache".to_string()),
        permission_mode: None,
    })
}

/// Every cache entry, most recently written first.
///
/// A conversation can have several stored versions as it grows. They are kept
/// distinct here so that an unreadable newest version can be retried against an
/// older version *of the same conversation*, rather than silently resolving to a
/// different conversation.
fn list_cache_entries() -> Vec<CachedConversationRef> {
    // Stream 0/1 live in `<hash>_0`; other suffixes hold side data.
    let candidates: Vec<PathBuf> = paths::http_cache_dirs()
        .iter()
        .filter_map(|cache_dir| std::fs::read_dir(cache_dir).ok())
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.to_string_lossy().ends_with("_0"))
        .collect();

    // A profile accumulates tens of thousands of entries, and identifying one
    // costs an open plus a small read. Done serially that is seconds of stall on
    // a synchronous command, so the scan is spread across cores. Nothing is
    // cached between calls: a stale index would mean exporting the wrong
    // conversation, which is the one failure this app must not have.
    let workers = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(4)
        .clamp(1, 8);
    let chunk_size = candidates.len().div_ceil(workers).max(1);

    let mut found: Vec<CachedConversationRef> = std::thread::scope(|scope| {
        let handles: Vec<_> = candidates
            .chunks(chunk_size)
            .map(|chunk| scope.spawn(move || chunk.iter().filter_map(read_reference).collect()))
            .collect();

        handles
            .into_iter()
            .flat_map(|handle| handle.join().unwrap_or_else(|_| Vec::new()))
            .collect()
    });

    // Newest first, breaking ties on size so a truncated response loses to a
    // full one.
    found.sort_by(|a, b| {
        (b.modified_unix_ms, b.entry_bytes).cmp(&(a.modified_unix_ms, a.entry_bytes))
    });
    found
}

/// Identifies one cache entry, without reading its body.
fn read_reference(path: &PathBuf) -> Option<CachedConversationRef> {
    let uuid = conversation_uuid_from_key(&simple_cache::read_entry_key(path)?)?;
    let metadata = std::fs::metadata(path).ok()?;

    Some(CachedConversationRef {
        uuid,
        entry_path: path.clone(),
        modified_unix_ms: metadata
            .modified()
            .ok()
            .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis())
            .unwrap_or(0),
        entry_bytes: metadata.len(),
        http_status: simple_cache::read_entry_status(path),
    })
}

/// The only origin and route a conversation transcript can come from.
const CONVERSATION_ROUTE_PREFIX: &str = "https://claude.ai/api/organizations/";

/// Isolates the requested resource URL from a Chromium cache key.
///
/// Keys wrap the URL in scheme-dependent framing: `1/0/<url>` for the HTTP
/// cache, and space-separated site prefixes such as
/// `_dk_<top-frame-site> <frame-site> <url>` for partitioned entries. The URL is
/// the last whitespace-separated field, starting at its *first* `https://` — a
/// later one belongs to a query parameter, not to the origin being fetched.
fn resource_url(key: &str) -> &str {
    let field = key.rsplit(char::is_whitespace).next().unwrap_or(key);
    match field.find("https://") {
        Some(at) => &field[at..],
        None => field,
    }
}

/// Extracts the conversation UUID from a cache key.
///
/// Keys look like
/// `1/0/https://claude.ai/api/organizations/<org>/chat_conversations/<uuid>?...`,
/// with the leading fields varying by cache-key scheme (partitioned keys carry
/// extra site prefixes). The whole origin and route are matched, not just the
/// `/chat_conversations/` fragment: the renderer cache holds third-party
/// resources too, and a foreign URL containing a lookalike path must never be
/// mistaken for a transcript.
///
/// Sub-resources such as `.../chat_conversations/<uuid>/title` are not
/// transcripts either.
fn conversation_uuid_from_key(key: &str) -> Option<String> {
    // Anchored at the start of the resource URL, never searched for inside it.
    // Searching would accept a foreign URL that merely embeds the route, e.g.
    // `https://evil.example/?next=https://claude.ai/api/organizations/...`.
    let rest = resource_url(key).strip_prefix(CONVERSATION_ROUTE_PREFIX)?;

    // <org>/chat_conversations/<uuid>, with no extra path segments before it.
    let (organization, rest) = rest.split_once('/')?;
    if organization.is_empty() || organization.contains(['?', '#']) {
        return None;
    }
    let rest = rest.strip_prefix("chat_conversations/")?;

    let uuid: String = rest
        .chars()
        .take_while(|character| *character != '?' && *character != '/')
        .collect();
    let terminator = rest[uuid.len()..].chars().next();

    (is_uuid(&uuid) && matches!(terminator, None | Some('?'))).then_some(uuid)
}

fn is_uuid(value: &str) -> bool {
    value.len() == 36
        && value.chars().enumerate().all(|(index, character)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                character == '-'
            } else {
                character.is_ascii_hexdigit()
            }
        })
}

/// Decodes one cache entry and confirms it is the conversation it claims to be.
///
/// The payload UUID must match the one the cache key promised; a mismatch means
/// the entry is not what was requested and must not be exported under its name.
fn load_expected_conversation(entry: &CachedConversationRef) -> Result<Conversation, String> {
    let conversation = load_conversation(&entry.entry_path)?;
    if conversation.uuid != entry.uuid {
        return Err(format!(
            "cache key names conversation {} but the payload is {}",
            entry.uuid, conversation.uuid
        ));
    }

    Ok(conversation)
}

/// Decodes one cache entry into a conversation.
pub fn load_conversation(path: &Path) -> Result<Conversation, String> {
    let entry = simple_cache::read_entry(path)
        .ok_or_else(|| "cache entry could not be parsed".to_string())?;

    // A conversation deleted from the account leaves a cached 404 behind.
    match simple_cache::status_code(&entry.headers) {
        Some(200) | None => {}
        Some(status) => return Err(format!("cached response was HTTP {status}")),
    }

    let encoding = simple_cache::header_value(&entry.headers, "content-encoding");
    let body = decode::decode_body(&entry.body, encoding.as_deref())?;
    serde_json::from_slice(&body).map_err(|error| {
        format!(
            "cached response for {} was not a conversation: {error}",
            entry.key
        )
    })
}

pub fn export_web_conversation(
    options: &ChatExportOptions,
) -> Result<ChatExportResult, CaptureError> {
    let entries = list_cache_entries();
    if entries.is_empty() {
        return Err(CaptureError::Diagnostic(no_conversations_message()));
    }
    // Skip cached errors when picking the target: a deleted conversation leaves
    // a 404 behind, and it must not mask the conversations that do exist.
    let Some(newest) = entries.iter().find(|entry| entry.is_usable()) else {
        return Err(CaptureError::Diagnostic(format!(
            "Every cached Claude Home entry is an error response, so there is nothing to export. Open the chat in Claude Desktop, wait for it to load, then retry. Checked: {}",
            paths::describe_searched_locations()
        )));
    };

    // Exactly one conversation is targeted, and only its own stored versions are
    // tried. Falling through to a different conversation would turn an
    // unreadable entry into a successful export of the wrong chat.
    let target_uuid = options
        .conversation_id
        .clone()
        .unwrap_or_else(|| newest.uuid.clone());

    let attempts: Vec<CachedConversationRef> = entries
        .iter()
        .filter(|candidate| candidate.uuid == target_uuid && candidate.is_usable())
        .take(MAX_VERSIONS_PER_CONVERSATION)
        .cloned()
        .collect();
    if attempts.is_empty() {
        return Err(CaptureError::Diagnostic(format!(
            "Conversation {target_uuid} is not in Claude Desktop's local cache. Open it in Claude Desktop, wait for it to load, then retry."
        )));
    }

    let normalize = conversation::NormalizeOptions {
        include_thinking: options.include_thinking.unwrap_or(false),
        include_tools: options.include_tools.unwrap_or(true),
    };

    let mut failures = Vec::new();
    for candidate in &attempts {
        let conversation = match load_expected_conversation(candidate) {
            Ok(conversation) => conversation,
            Err(error) => {
                failures.push(format!("{}: {error}", candidate.entry_path.display()));
                continue;
            }
        };

        let messages = conversation.to_export_messages(normalize);
        if messages.is_empty() {
            failures.push(format!(
                "{}: no user or Claude messages were readable",
                candidate.entry_path.display()
            ));
            continue;
        }

        let title = conversation
            .display_title()
            .unwrap_or_else(|| title_from_first_user_message(&messages));

        let document = export::ExportDocument {
            title,
            session_id: conversation.uuid.clone(),
            source_type: "Claude Home web cache".to_string(),
            source_path: candidate.entry_path.display().to_string(),
            model: conversation.model.clone(),
            summary: conversation.summary.clone(),
            project_uuid: conversation.project_uuid.clone(),
            created_at: conversation.created_at.clone(),
            updated_at: conversation.updated_at.clone(),
            messages,
        };

        let mut warnings = vec![
            "Exported from Claude Desktop's local response cache. If this is not the chat you expected, open that chat in Claude Desktop, wait for it to load, then retry.".to_string(),
        ];
        if !normalize.include_thinking {
            warnings.push("Claude's thinking blocks were excluded.".to_string());
        }
        if !normalize.include_tools {
            warnings.push("Tool and Cowork activity was excluded.".to_string());
        }
        if !failures.is_empty() {
            warnings.push(format!(
                "Exported an older stored version of this conversation: {} newer cache entr{} could not be read ({}). It may be missing the most recent messages.",
                failures.len(),
                if failures.len() == 1 { "y" } else { "ies" },
                failures.join("; ")
            ));
        }

        return export::write_export(&document, warnings, options);
    }

    Err(CaptureError::Diagnostic(format!(
        "Conversation {target_uuid} is cached, but none of its {} stored version(s) could be read, so nothing was exported. Open the chat in Claude Desktop, wait for it to load, then retry. Details: {}",
        attempts.len(),
        failures.join("; ")
    )))
}

fn no_conversations_message() -> String {
    format!(
        "No regular Claude Home chat transcript was found in Claude Desktop's local cache. Open the chat in Claude Desktop, wait for it to load, then retry. Checked: {}",
        paths::describe_searched_locations()
    )
}

fn title_from_first_user_message(messages: &[crate::models::ChatExportMessage]) -> String {
    messages
        .iter()
        .find(|message| message.role == "user")
        .and_then(|message| {
            message
                .text
                .lines()
                .find(|line| !line.trim().is_empty())
                .map(|line| line.chars().take(80).collect::<String>())
        })
        .unwrap_or_else(|| "Claude Session".to_string())
}

/// Metadata for the most recently cached conversation that actually decodes.
///
/// Returns `None` when nothing decodes, because callers read `Some` as proof
/// that a Home chat is the active session. A cached 404 or a half-written
/// entry as the newest candidate must not answer that question.
///
/// Memoized on the newest entry's path and mtime: decoding a long conversation
/// means decompressing megabytes.
pub fn latest_session_metadata() -> Option<LatestSessionMetadata> {
    static MEMO: Mutex<Option<(PathBuf, u128, Option<LatestSessionMetadata>)>> = Mutex::new(None);

    let entries = list_cache_entries();
    let newest = entries.iter().find(|entry| entry.is_usable())?;

    let mut memo = MEMO.lock().ok()?;
    if let Some((path, modified, metadata)) = memo.as_ref() {
        if path == &newest.entry_path && *modified == newest.modified_unix_ms {
            return metadata.clone();
        }
    }

    // Only the newest conversation's own versions, matching what an export would
    // target — reporting one chat's title while export produces another would be
    // worse than reporting nothing. A conversation legitimately may have no
    // title, so this keys off decoding rather than off finding a name.
    let metadata = entries
        .iter()
        .filter(|candidate| candidate.uuid == newest.uuid && candidate.is_usable())
        .take(MAX_VERSIONS_PER_CONVERSATION)
        .find_map(|candidate| load_expected_conversation(candidate).ok())
        .map(|conversation| LatestSessionMetadata {
            title: conversation.display_title(),
            observed_at_unix_ms: newest.modified_unix_ms as u64,
        });
    *memo = Some((
        newest.entry_path.clone(),
        newest.modified_unix_ms,
        metadata.clone(),
    ));

    metadata
}

#[cfg(test)]
mod tests {
    use super::{conversation_uuid_from_key, is_uuid, resource_url};
    use crate::models::ChatExportOptions;

    const UUID: &str = "29f9667b-3882-476b-9926-91f536473a83";

    #[test]
    fn reads_uuid_from_a_real_cache_key() {
        let key = format!(
            "1/0/https://claude.ai/api/organizations/9900def5-47b5-4ce8-9818-42d218785e26/chat_conversations/{UUID}?tree=True&rendering_mode=messages"
        );
        assert_eq!(conversation_uuid_from_key(&key).as_deref(), Some(UUID));
    }

    #[test]
    fn reads_uuid_when_the_url_has_no_query() {
        let key = format!("1/0/https://claude.ai/api/organizations/org/chat_conversations/{UUID}");
        assert_eq!(conversation_uuid_from_key(&key).as_deref(), Some(UUID));
    }

    /// A foreign URL that merely *embeds* the Claude route must be rejected.
    /// Searching the key for the route instead of anchoring at the resource URL
    /// accepted these.
    #[test]
    fn rejects_urls_that_embed_the_claude_route() {
        for key in [
            format!("1/0/https://evil.example/?next=https://claude.ai/api/organizations/org/chat_conversations/{UUID}"),
            format!("1/0/https://evil.example/redirect#https://claude.ai/api/organizations/org/chat_conversations/{UUID}"),
            format!("1/0/https://evil.example/https://claude.ai/api/organizations/org/chat_conversations/{UUID}"),
        ] {
            assert_eq!(conversation_uuid_from_key(&key), None, "accepted: {key}");
        }
    }

    #[test]
    fn resource_url_takes_the_last_field_from_its_first_scheme() {
        assert_eq!(
            resource_url("1/0/https://claude.ai/api/x"),
            "https://claude.ai/api/x"
        );
        assert_eq!(
            resource_url("_dk_https://claude.ai https://claude.ai https://claude.ai/api/y"),
            "https://claude.ai/api/y"
        );
        assert_eq!(
            resource_url("1/0/https://evil.example/?next=https://claude.ai/api/z"),
            "https://evil.example/?next=https://claude.ai/api/z"
        );
    }

    /// The renderer cache holds third-party resources; only claude.ai's own API
    /// route may be read as a transcript.
    #[test]
    fn rejects_foreign_origins_and_lookalike_routes() {
        for key in [
            format!("1/0/https://evil.example/api/organizations/org/chat_conversations/{UUID}"),
            format!("1/0/http://claude.ai/api/organizations/org/chat_conversations/{UUID}"),
            format!("1/0/https://claude.ai.evil.example/api/organizations/org/chat_conversations/{UUID}"),
            format!("1/0/https://claude.ai/chat_conversations/{UUID}"),
            format!("1/0/https://claude.ai/api/organizations/org/projects/p/chat_conversations/{UUID}"),
            format!("1/0/https://claude.ai/api/organizations//chat_conversations/{UUID}"),
        ] {
            assert_eq!(conversation_uuid_from_key(&key), None, "accepted: {key}");
        }
    }

    /// Partitioned cache keys carry extra site prefixes before the resource URL.
    #[test]
    fn reads_uuid_from_a_partitioned_cache_key() {
        let key = format!(
            "_dk_https://claude.ai https://claude.ai https://claude.ai/api/organizations/org/chat_conversations/{UUID}?tree=True"
        );
        assert_eq!(conversation_uuid_from_key(&key).as_deref(), Some(UUID));
    }

    #[test]
    fn ignores_sub_resources_and_list_endpoints() {
        let sub =
            format!("1/0/https://claude.ai/api/organizations/org/chat_conversations/{UUID}/title");
        assert_eq!(conversation_uuid_from_key(&sub), None);
        assert_eq!(
            conversation_uuid_from_key(
                "1/0/https://claude.ai/api/organizations/org/chat_conversations_v2?limit=5"
            ),
            None
        );
    }

    /// Exercises the whole path against the Claude Desktop profile installed on
    /// this machine. Not hermetic, so it does not run by default:
    ///
    /// ```text
    /// cargo test -- --ignored --nocapture real_profile
    /// ```
    #[test]
    #[ignore = "requires a local Claude Desktop profile with a cached Home chat"]
    fn lists_home_sessions_from_the_real_profile() {
        let sessions = super::list_home_sessions();
        assert!(!sessions.is_empty(), "no cached Home conversations found");
        assert!(sessions
            .iter()
            .all(|session| session.source_type == super::SOURCE_HOME));
        let unique: std::collections::HashSet<_> = sessions
            .iter()
            .map(|session| &session.cli_session_id)
            .collect();
        assert_eq!(unique.len(), sessions.len());
    }

    #[test]
    #[ignore = "requires a local Claude Desktop profile with a cached Home chat"]
    fn exports_a_conversation_from_the_real_profile() {
        let started = std::time::Instant::now();
        let cached = super::list_cached_conversations();
        let index_time = started.elapsed();
        assert!(
            !cached.is_empty(),
            "no cached conversations found; open a Home chat in Claude Desktop first"
        );
        eprintln!(
            "indexed {} cached conversation(s) in {index_time:?}",
            cached.len()
        );

        let mut decoded = 0;
        for candidate in &cached {
            match super::load_conversation(&candidate.entry_path) {
                Ok(conversation) => {
                    decoded += 1;
                    eprintln!(
                        "  {} — {} message(s) — {}",
                        candidate.uuid,
                        conversation.chat_messages.as_ref().map_or(0, Vec::len),
                        conversation.display_title().unwrap_or_default()
                    );
                }
                Err(error) => eprintln!("  {} — skipped: {error}", candidate.uuid),
            }
        }
        assert!(decoded > 0, "no cached conversation could be decoded");

        let result = super::export_web_conversation(&ChatExportOptions {
            include_thinking: Some(true),
            ..ChatExportOptions::default()
        })
        .expect("export should succeed");
        eprintln!(
            "exported {:?} — {} messages\n  {}\n  {}",
            result.title,
            result.message_count,
            result.markdown_path.as_deref().unwrap_or("not generated"),
            result.json_path.as_deref().unwrap_or("not generated")
        );
        assert!(result.message_count > 0);
        assert!(std::path::Path::new(result.markdown_path.as_ref().unwrap()).exists());
        assert!(std::path::Path::new(result.json_path.as_ref().unwrap()).exists());
    }

    #[test]
    fn validates_uuid_shape() {
        assert!(is_uuid(UUID));
        assert!(!is_uuid("29f9667b38824"));
        assert!(!is_uuid("29f9667b-3882-476b-9926-91f536473aZZ"));
    }
}
