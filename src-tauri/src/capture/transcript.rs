use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::capture::CaptureError;
use crate::filename::sanitize_filename_part;
use crate::models::{ChatExportMessage, ChatExportResult};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaudeCodeSessionMetadata {
    session_id: Option<String>,
    cli_session_id: Option<String>,
    cwd: Option<String>,
    title: Option<String>,
    model: Option<String>,
    created_at: Option<u64>,
    last_focused_at: Option<u64>,
    last_activity_at: Option<u64>,
}

#[derive(Debug, Clone)]
struct SessionCandidate {
    metadata: ClaudeCodeSessionMetadata,
    metadata_path: PathBuf,
    modified_unix_ms: u64,
}

#[derive(Debug, Clone)]
pub struct LatestSessionMetadata {
    pub title: Option<String>,
}

#[derive(Debug, Serialize)]
struct TranscriptExportDocument {
    title: String,
    session_id: String,
    cli_session_id: String,
    cwd: Option<String>,
    model: Option<String>,
    created_at: Option<u64>,
    last_activity_at: Option<u64>,
    source_path: String,
    messages: Vec<ChatExportMessage>,
}

pub fn export_latest_session(store: SessionStore) -> Result<ChatExportResult, CaptureError> {
    let session = find_latest_session_candidate(store)?;
    let cli_session_id = session.metadata.cli_session_id.clone().ok_or_else(|| {
        CaptureError::Diagnostic(format!(
            "Claude session metadata at {} did not include a cliSessionId.",
            session.metadata_path.display()
        ))
    })?;
    let transcript_path = find_transcript_path(
        &session.metadata_path,
        session.metadata.cwd.as_deref(),
        &cli_session_id,
    )?;
    let mut warnings = Vec::new();
    let (messages, title_from_transcript) = parse_transcript(&transcript_path, &mut warnings)?;

    if messages.is_empty() {
        return Err(CaptureError::Diagnostic(format!(
            "Claude transcript was found at {}, but no human prompts or Claude text replies were readable.",
            transcript_path.display()
        )));
    }

    let title = session
        .metadata
        .title
        .clone()
        .filter(|value| !value.trim().is_empty())
        .or(title_from_transcript)
        .unwrap_or_else(|| title_from_first_user_message(&messages));

    let exports_dir = std::env::current_dir()
        .map_err(|error| CaptureError::Diagnostic(error.to_string()))?
        .join("exports");
    fs::create_dir_all(&exports_dir)
        .map_err(|error| CaptureError::Diagnostic(error.to_string()))?;

    let timestamp = unix_timestamp_secs()?;
    let filename_title = sanitize_filename_part(&title, "Claude Session");
    let basename = format!("{filename_title}-{timestamp}");
    let markdown_path = exports_dir.join(format!("{basename}.md"));
    let json_path = exports_dir.join(format!("{basename}.json"));

    let document = TranscriptExportDocument {
        title: title.clone(),
        session_id: session
            .metadata
            .session_id
            .clone()
            .unwrap_or_else(|| cli_session_id.clone()),
        cli_session_id: cli_session_id.clone(),
        cwd: session.metadata.cwd.clone(),
        model: session.metadata.model.clone(),
        created_at: session.metadata.created_at,
        last_activity_at: session.metadata.last_activity_at,
        source_path: transcript_path.display().to_string(),
        messages: messages.clone(),
    };

    let json = serde_json::to_string_pretty(&document)
        .map_err(|error| CaptureError::Diagnostic(error.to_string()))?;
    fs::write(&json_path, json).map_err(|error| CaptureError::Diagnostic(error.to_string()))?;
    fs::write(&markdown_path, render_markdown(&document))
        .map_err(|error| CaptureError::Diagnostic(error.to_string()))?;

    Ok(ChatExportResult {
        title,
        session_id: cli_session_id,
        source_type: store.label().to_string(),
        source_path: transcript_path.display().to_string(),
        markdown_path: markdown_path.display().to_string(),
        json_path: json_path.display().to_string(),
        message_count: messages.len(),
        warnings,
    })
}

// Used by the Windows adapter's session summary.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn latest_session_metadata() -> Option<LatestSessionMetadata> {
    latest_session_metadata_for(SessionStore::Any)
}

pub fn latest_session_metadata_for(store: SessionStore) -> Option<LatestSessionMetadata> {
    find_latest_session_candidate(store)
        .ok()
        .map(|candidate| LatestSessionMetadata {
            title: candidate.metadata.title,
        })
}

fn find_latest_session_candidate(store: SessionStore) -> Result<SessionCandidate, CaptureError> {
    let mut candidates = Vec::new();
    for root in claude_code_session_roots(store)? {
        collect_session_candidates(&root, &mut candidates)?;
    }

    candidates
        .into_iter()
        .max_by_key(|candidate| {
            (
                candidate.metadata.last_focused_at.unwrap_or(0),
                candidate.metadata.last_activity_at.unwrap_or(0),
                candidate.modified_unix_ms,
            )
        })
        .ok_or_else(|| {
            CaptureError::Diagnostic(
                "No Claude Code or Cowork session metadata was found on this computer."
                    .to_string(),
            )
        })
}

/// Which local session store to read.
///
/// Both use the same `local_*.json` + JSONL format, but they are different
/// products: Cowork sessions appear in Claude Desktop's Home sidebar, Claude
/// Code sessions come from the CLI. They cannot be told apart after the fact, so
/// the caller says which one it wants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStore {
    Cowork,
    ClaudeCode,
    Any,
}

impl SessionStore {
    fn directory_names(self) -> &'static [&'static str] {
        match self {
            SessionStore::Cowork => &["local-agent-mode-sessions"],
            SessionStore::ClaudeCode => &["claude-code-sessions"],
            SessionStore::Any => &["local-agent-mode-sessions", "claude-code-sessions"],
        }
    }

    fn label(self) -> &'static str {
        match self {
            SessionStore::Cowork => "Cowork session (local JSONL)",
            SessionStore::ClaudeCode => "Claude Code JSONL",
            SessionStore::Any => "Claude local session (JSONL)",
        }
    }
}

fn claude_code_session_roots(store: SessionStore) -> Result<Vec<PathBuf>, CaptureError> {
    let mut roots = Vec::new();

    for data_dir in crate::capture::web_cache::paths::claude_data_dirs() {
        for name in store.directory_names() {
            roots.push(data_dir.join(name));
        }
    }

    roots.retain(|root| root.exists());
    Ok(roots)
}

/// Home directories holding the shared `.claude/projects` store.
///
/// Both variables are consulted, most authoritative first: on Windows
/// `USERPROFILE` is the real profile while `HOME` is often redirected by Git,
/// MSYS, or Cygwin to somewhere else entirely.
fn home_dirs() -> Vec<PathBuf> {
    let ordered = if cfg!(windows) {
        ["USERPROFILE", "HOME"]
    } else {
        ["HOME", "USERPROFILE"]
    };

    let mut dirs: Vec<PathBuf> = ordered
        .iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .collect();
    dirs.dedup();
    dirs
}

fn collect_session_candidates(
    directory: &Path,
    candidates: &mut Vec<SessionCandidate>,
) -> Result<(), CaptureError> {
    for entry in
        fs::read_dir(directory).map_err(|error| CaptureError::Diagnostic(error.to_string()))?
    {
        let entry = entry.map_err(|error| CaptureError::Diagnostic(error.to_string()))?;
        let path = entry.path();
        if path.is_dir() {
            let _ = collect_session_candidates(&path, candidates);
            continue;
        }

        let is_local_json = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("local_") && name.ends_with(".json"));
        if !is_local_json {
            continue;
        }

        // Claude Desktop rewrites these while running, so a torn or truncated
        // read is expected. Skip the file rather than failing the whole scan and
        // hiding every other session in the store.
        let Ok(contents) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(metadata) = serde_json::from_str::<ClaudeCodeSessionMetadata>(&contents) else {
            continue;
        };
        if metadata.cli_session_id.is_none() {
            continue;
        }
        let modified_unix_ms = entry
            .metadata()
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(0);
        candidates.push(SessionCandidate {
            metadata,
            metadata_path: path,
            modified_unix_ms,
        });
    }

    Ok(())
}

/// Locates the JSONL transcript for a session.
///
/// A Cowork session running in host-loop mode has its own `.claude` home beside
/// its metadata file — `local_<id>.json` sits next to `local_<id>/.claude/…` —
/// so that is searched before the shared `~/.claude/projects` store used by
/// Claude Code.
fn find_transcript_path(
    metadata_path: &Path,
    cwd: Option<&str>,
    cli_session_id: &str,
) -> Result<PathBuf, CaptureError> {
    let filename = format!("{cli_session_id}.jsonl");
    let mut projects_roots = Vec::new();

    if let Some(session_dir) = metadata_path
        .file_stem()
        .map(|stem| metadata_path.with_file_name(stem))
    {
        projects_roots.push(session_dir.join(".claude").join("projects"));
    }
    for home in home_dirs() {
        projects_roots.push(home.join(".claude").join("projects"));
    }
    projects_roots.retain(|root| root.exists());
    projects_roots.dedup();

    for projects_root in &projects_roots {
        if let Some(cwd) = cwd {
            let direct = projects_root
                .join(claude_project_directory_name(cwd))
                .join(&filename);
            if direct.exists() {
                return Ok(direct);
            }
        }
        if let Some(found) = find_file_recursive(projects_root, &filename)? {
            return Ok(found);
        }
    }

    Err(CaptureError::Diagnostic(format!(
        "Could not find Claude transcript {filename}. Checked: {}.",
        if projects_roots.is_empty() {
            "no .claude/projects directory exists".to_string()
        } else {
            projects_roots
                .iter()
                .map(|root| root.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        }
    )))
}

fn find_file_recursive(directory: &Path, filename: &str) -> Result<Option<PathBuf>, CaptureError> {
    if !directory.exists() {
        return Ok(None);
    }

    for entry in
        fs::read_dir(directory).map_err(|error| CaptureError::Diagnostic(error.to_string()))?
    {
        let entry = entry.map_err(|error| CaptureError::Diagnostic(error.to_string()))?;
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_file_recursive(&path, filename)? {
                return Ok(Some(found));
            }
        } else if path.file_name().and_then(|name| name.to_str()) == Some(filename) {
            return Ok(Some(path));
        }
    }

    Ok(None)
}

fn parse_transcript(
    transcript_path: &Path,
    warnings: &mut Vec<String>,
) -> Result<(Vec<ChatExportMessage>, Option<String>), CaptureError> {
    let file =
        File::open(transcript_path).map_err(|error| CaptureError::Diagnostic(error.to_string()))?;
    let reader = BufReader::new(file);
    let mut messages = Vec::new();
    let mut title = None;

    for (line_index, line) in reader.lines().enumerate() {
        let line = line.map_err(|error| CaptureError::Diagnostic(error.to_string()))?;
        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(error) => {
                warnings.push(format!(
                    "Skipped unreadable transcript line {}: {}",
                    line_index + 1,
                    error
                ));
                continue;
            }
        };

        match value.get("type").and_then(Value::as_str) {
            Some("custom-title") => {
                title = value
                    .get("customTitle")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or(title);
            }
            Some("ai-title") if title.is_none() => {
                title = value
                    .get("aiTitle")
                    .and_then(Value::as_str)
                    .map(str::to_string);
            }
            Some("user") if is_human_user_message(&value) => {
                if let Some(text) = extract_message_text(value.get("message")) {
                    messages.push(ChatExportMessage::plain(
                        "user",
                        text,
                        value
                            .get("timestamp")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                    ));
                }
            }
            Some("assistant") => {
                if let Some(text) = extract_message_text(value.get("message")) {
                    messages.push(ChatExportMessage::plain(
                        "claude",
                        text,
                        value
                            .get("timestamp")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                    ));
                }
            }
            _ => {}
        }
    }

    Ok((coalesce_adjacent_messages(messages), title))
}

fn is_human_user_message(value: &Value) -> bool {
    value
        .get("origin")
        .and_then(|origin| origin.get("kind"))
        .and_then(Value::as_str)
        == Some("human")
}

fn extract_message_text(message: Option<&Value>) -> Option<String> {
    let message = message?;
    let content = message.get("content")?;
    let text = extract_content_text(content);
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

fn extract_content_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.trim().to_string(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(extract_content_block_text)
            .collect::<Vec<_>>()
            .join("\n\n"),
        _ => String::new(),
    }
}

fn extract_content_block_text(block: &Value) -> Option<String> {
    let block_type = block.get("type").and_then(Value::as_str);
    match block_type {
        Some("text") => block
            .get("text")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string),
        _ => None,
    }
}

fn coalesce_adjacent_messages(messages: Vec<ChatExportMessage>) -> Vec<ChatExportMessage> {
    let mut coalesced: Vec<ChatExportMessage> = Vec::new();
    for message in messages {
        if let Some(previous) = coalesced.last_mut() {
            if previous.role == message.role {
                previous.text.push_str("\n\n");
                previous.text.push_str(&message.text);
                previous.blocks.extend(message.blocks);
                if previous.timestamp.is_none() {
                    previous.timestamp = message.timestamp;
                }
                continue;
            }
        }
        coalesced.push(message);
    }
    coalesced
}

fn render_markdown(document: &TranscriptExportDocument) -> String {
    let mut markdown = String::new();
    markdown.push_str("# ");
    markdown.push_str(&document.title);
    markdown.push_str("\n\n");
    markdown.push_str("| Field | Value |\n| --- | --- |\n");
    markdown.push_str(&format!(
        "| Session ID | {} |\n",
        escape_table_cell(&document.session_id)
    ));
    markdown.push_str(&format!(
        "| CLI Session ID | {} |\n",
        escape_table_cell(&document.cli_session_id)
    ));
    if let Some(cwd) = &document.cwd {
        markdown.push_str(&format!("| Project | {} |\n", escape_table_cell(cwd)));
    }
    if let Some(model) = &document.model {
        markdown.push_str(&format!("| Model | {} |\n", escape_table_cell(model)));
    }
    markdown.push_str(&format!(
        "| Source | {} |\n\n",
        escape_table_cell(&document.source_path)
    ));

    for message in &document.messages {
        let heading = if message.role == "user" {
            "User"
        } else {
            "Claude"
        };
        if let Some(timestamp) = &message.timestamp {
            markdown.push_str(&format!("## {heading} ({timestamp})\n\n"));
        } else {
            markdown.push_str(&format!("## {heading}\n\n"));
        }
        markdown.push_str(message.text.trim());
        markdown.push_str("\n\n");
    }

    markdown
}

fn escape_table_cell(value: &str) -> String {
    value.replace('|', "\\|").replace('\n', "<br>")
}

fn title_from_first_user_message(messages: &[ChatExportMessage]) -> String {
    messages
        .iter()
        .find(|message| message.role == "user")
        .map(|message| {
            let first_line = message
                .text
                .lines()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("Claude Session");
            first_line.chars().take(80).collect::<String>()
        })
        .unwrap_or_else(|| "Claude Session".to_string())
}

fn claude_project_directory_name(cwd: &str) -> String {
    cwd.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' || character == '-' {
                character
            } else {
                '-'
            }
        })
        .collect()
}

fn unix_timestamp_secs() -> Result<u64, CaptureError> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| CaptureError::Diagnostic(error.to_string()))?
        .as_secs())
}

#[cfg(test)]
mod store_tests {
    use super::{home_dirs, SessionStore};

    /// Each store must resolve to its own directory. "Claude Code" resolving to
    /// `Any` would let a Cowork session answer a Claude Code request.
    #[test]
    fn stores_map_to_distinct_directories() {
        assert_eq!(
            SessionStore::Cowork.directory_names(),
            ["local-agent-mode-sessions"]
        );
        assert_eq!(
            SessionStore::ClaudeCode.directory_names(),
            ["claude-code-sessions"]
        );
        assert_eq!(SessionStore::Any.directory_names().len(), 2);
    }

    #[test]
    fn stores_are_labelled_distinctly() {
        let labels = [
            SessionStore::Cowork.label(),
            SessionStore::ClaudeCode.label(),
            SessionStore::Any.label(),
        ];
        assert_eq!(
            labels.iter().collect::<std::collections::HashSet<_>>().len(),
            3
        );
    }

    /// On Windows `HOME` is frequently redirected by Git/MSYS, so the real
    /// profile must be consulted first.
    #[test]
    fn home_directories_are_ordered_by_authority() {
        let dirs = home_dirs();
        if cfg!(windows) {
            if let (Some(profile), Some(first)) =
                (std::env::var_os("USERPROFILE"), dirs.first())
            {
                assert_eq!(first.as_os_str(), profile);
            }
        } else if let (Some(home), Some(first)) = (std::env::var_os("HOME"), dirs.first()) {
            assert_eq!(first.as_os_str(), home);
        }
    }
}

#[cfg(test)]
mod live_tests {
    /// Exports the newest Cowork session from this machine's Claude Desktop.
    /// Not hermetic: `cargo test -- --ignored --nocapture cowork`.
    #[test]
    #[ignore = "requires a local Cowork session"]
    fn exports_the_newest_cowork_session() {
        let metadata = super::latest_session_metadata_for(super::SessionStore::Cowork);
        eprintln!("newest Cowork session title: {:?}", metadata.and_then(|m| m.title));

        let result = super::export_latest_session(super::SessionStore::Cowork)
            .expect("Cowork export should succeed");
        eprintln!(
            "exported {:?} — {} messages\n  {}",
            result.title, result.message_count, result.markdown_path
        );
        assert!(result.message_count > 0);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        claude_project_directory_name, coalesce_adjacent_messages, extract_message_text,
        is_human_user_message,
    };
    use crate::models::ChatExportMessage;

    #[test]
    fn maps_windows_cwd_to_claude_project_directory_name() {
        assert_eq!(
            claude_project_directory_name(r"C:\Users\Anmar Abdelrahman\Desktop\Football-Game"),
            "C--Users-Anmar-Abdelrahman-Desktop-Football-Game"
        );
    }

    #[test]
    fn extracts_string_user_content() {
        let value = json!({
            "content": "What did I ask?"
        });
        assert_eq!(
            extract_message_text(Some(&value)),
            Some("What did I ask?".to_string())
        );
    }

    #[test]
    fn extracts_only_text_blocks_from_assistant_content() {
        let value = json!({
            "content": [
                { "type": "thinking", "thinking": "hidden" },
                { "type": "text", "text": "Visible reply" },
                { "type": "tool_use", "name": "Read" }
            ]
        });
        assert_eq!(
            extract_message_text(Some(&value)),
            Some("Visible reply".to_string())
        );
    }

    #[test]
    fn filters_out_tool_result_user_messages() {
        let tool_result = json!({
            "type": "user",
            "message": { "role": "user", "content": [{ "type": "tool_result", "content": "noise" }] }
        });
        let human = json!({
            "type": "user",
            "origin": { "kind": "human" },
            "message": { "role": "user", "content": "Keep this" }
        });

        assert!(!is_human_user_message(&tool_result));
        assert!(is_human_user_message(&human));
    }

    #[test]
    fn coalesces_adjacent_messages_with_same_role() {
        let messages = vec![
            ChatExportMessage::plain("claude", "One".to_string(), Some("t1".to_string())),
            ChatExportMessage::plain("claude", "Two".to_string(), Some("t2".to_string())),
        ];

        let coalesced = coalesce_adjacent_messages(messages);
        assert_eq!(coalesced.len(), 1);
        assert_eq!(coalesced[0].text, "One\n\nTwo");
        assert_eq!(coalesced[0].timestamp.as_deref(), Some("t1"));
    }
}
