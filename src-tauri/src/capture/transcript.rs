use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::Value;

use crate::capture::output::{
    prepare_export_directory, reserve_export_paths, ExportFormats, ExportPaths,
};
use crate::capture::pdf::{self, PdfTranscript};
use crate::capture::{cowork, CaptureError};
use crate::filename::sanitize_filename_part;
use crate::models::{ChatExportMessage, ChatExportOptions, ChatExportResult, LocalSessionSummary};

#[derive(Debug, Clone)]
pub struct LatestSessionMetadata {
    pub title: Option<String>,
    pub observed_at_unix_ms: u64,
}

#[derive(Debug, Serialize)]
struct TranscriptExportDocument {
    title: String,
    session_id: String,
    cli_session_id: String,
    cwd: Option<String>,
    model: Option<String>,
    effort: Option<String>,
    created_at: Option<u64>,
    last_activity_at: Option<u64>,
    source_type: String,
    desktop_metadata_path: Option<String>,
    source_path: String,
    messages: Vec<ChatExportMessage>,
}

pub fn export_latest_session(
    store: SessionStore,
    options: &ChatExportOptions,
) -> Result<ChatExportResult, CaptureError> {
    export_session(store, None, options)
}

/// Export one discovered local session by its canonical JSONL filename stem, or
/// the newest session in the requested classification when no ID is supplied.
pub fn export_session(
    store: SessionStore,
    cli_session_id: Option<&str>,
    options: &ChatExportOptions,
) -> Result<ChatExportResult, CaptureError> {
    let discovery = cowork::discover_default();
    let session = discovery
        .sessions
        .iter()
        .filter(|session| store.includes(session))
        .find(|session| cli_session_id.is_none_or(|id| session.cli_session_id == id))
        .ok_or_else(|| missing_session_error(store, cli_session_id, &discovery))?;
    let transcript_path_string = session.transcript_path.as_deref().ok_or_else(|| {
        CaptureError::Diagnostic(format!(
            "Transcript unavailable for local session {}.",
            session.cli_session_id
        ))
    })?;
    let transcript_path = Path::new(transcript_path_string);
    let mut warnings = Vec::new();
    let (messages, _) = parse_transcript(transcript_path, &mut warnings)?;

    if messages.is_empty() {
        return Err(CaptureError::Diagnostic(format!(
            "Claude transcript was found at {}, but no human prompts or Claude text replies were readable.",
            transcript_path.display()
        )));
    }

    // Discovery already applies Desktop title -> transcript title -> cwd
    // basename -> cliSessionId. Keep the first-user fallback for legacy or
    // externally supplied transcripts whose summary has no useful title.
    let title = if session.title.trim().is_empty() {
        title_from_first_user_message(&messages)
    } else {
        session.title.clone()
    };

    let formats = ExportFormats::from_options(options)?;
    let exports_dir = prepare_export_directory(options.output_directory.as_deref())?;

    let timestamp = unix_timestamp_secs()?;
    let filename_title = sanitize_filename_part(&title, "Claude Session");
    let document = TranscriptExportDocument {
        title: title.clone(),
        session_id: session
            .desktop_session_id
            .clone()
            .unwrap_or_else(|| session.cli_session_id.clone()),
        cli_session_id: session.cli_session_id.clone(),
        cwd: session.cwd.clone(),
        model: session.model.clone(),
        effort: session.effort.clone(),
        created_at: session.created_at,
        last_activity_at: session.last_activity_at,
        source_type: session.source_type.clone(),
        desktop_metadata_path: session.metadata_path.clone(),
        source_path: transcript_path.display().to_string(),
        messages: messages.clone(),
    };

    let json = formats
        .json
        .then(|| serde_json::to_string_pretty(&document))
        .transpose()
        .map_err(|error| CaptureError::Diagnostic(error.to_string()))?;
    let markdown = formats.markdown.then(|| render_markdown(&document));
    let (pdf_bytes, pdf_warnings) = if formats.pdf {
        let (bytes, warnings) = pdf::render_pdf(&PdfTranscript {
            title: &document.title,
            source_type: &document.source_type,
            session_id: &document.session_id,
            model: document.model.as_deref(),
            messages: &document.messages,
        })?;
        (Some(bytes), warnings)
    } else {
        (None, Vec::new())
    };
    let paths = reserve_export_paths(&exports_dir, &filename_title, timestamp, formats)?;
    let written = write_selected_files(
        &paths,
        markdown.as_deref(),
        json.as_deref(),
        pdf_bytes.as_deref(),
    );
    if let Err(error) = written {
        paths.remove_files();
        return Err(CaptureError::Diagnostic(error.to_string()));
    }
    warnings.extend(pdf_warnings);

    Ok(ChatExportResult {
        title,
        session_id: session.cli_session_id.clone(),
        source_type: session.source_type.clone(),
        source_path: transcript_path.display().to_string(),
        markdown_path: optional_path_string(paths.markdown),
        json_path: optional_path_string(paths.json),
        pdf_path: optional_path_string(paths.pdf),
        output_directory: exports_dir.display().to_string(),
        message_count: messages.len(),
        warnings,
    })
}

fn write_selected_files(
    paths: &ExportPaths,
    markdown: Option<&str>,
    json: Option<&str>,
    pdf: Option<&[u8]>,
) -> std::io::Result<()> {
    if let (Some(path), Some(contents)) = (&paths.markdown, markdown) {
        fs::write(path, contents)?;
    }
    if let (Some(path), Some(contents)) = (&paths.json, json) {
        fs::write(path, contents)?;
    }
    if let (Some(path), Some(contents)) = (&paths.pdf, pdf) {
        fs::write(path, contents)?;
    }
    Ok(())
}

fn optional_path_string(path: Option<std::path::PathBuf>) -> Option<String> {
    path.map(|path| path.display().to_string())
}

// Used by the Windows adapter's session summary.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn latest_session_metadata() -> Option<LatestSessionMetadata> {
    latest_session_metadata_for(SessionStore::Any)
}

pub fn latest_session_metadata_for(store: SessionStore) -> Option<LatestSessionMetadata> {
    cowork::discover_default()
        .sessions
        .into_iter()
        .find(|session| store.includes(session))
        .map(|session| LatestSessionMetadata {
            title: Some(session.title),
            observed_at_unix_ms: session
                .metadata_modified_at
                .unwrap_or(0)
                .max(session.last_focused_at.unwrap_or(0))
                .max(session.last_activity_at.unwrap_or(0)),
        })
}

/// Which local session store to read.
///
/// Cowork and Claude Code come from separate metadata/transcript layouts.
/// `Any` is their already-deduplicated union.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStore {
    Cowork,
    ClaudeCode,
    Any,
}

impl SessionStore {
    fn includes(self, session: &LocalSessionSummary) -> bool {
        match self {
            SessionStore::Cowork => session.source_type == cowork::SOURCE_COWORK,
            SessionStore::ClaudeCode => session.source_type == cowork::SOURCE_CLAUDE_CODE,
            SessionStore::Any => true,
        }
    }

    fn label(self) -> &'static str {
        match self {
            SessionStore::Cowork => cowork::SOURCE_COWORK,
            SessionStore::ClaudeCode => cowork::SOURCE_CLAUDE_CODE,
            SessionStore::Any => "Claude local session",
        }
    }
}

fn missing_session_error(
    store: SessionStore,
    cli_session_id: Option<&str>,
    discovery: &crate::models::LocalSessionDiscovery,
) -> CaptureError {
    if let Some(id) = cli_session_id {
        if discovery
            .unmatched_metadata
            .iter()
            .any(|metadata| metadata.cli_session_id == id)
        {
            return CaptureError::Diagnostic(format!(
                "Transcript unavailable for Cowork session {id}. Its Desktop metadata still exists, but no matching {id}.jsonl was found under {}.",
                discovery.diagnostics.claude_projects_root
            ));
        }
        return CaptureError::Diagnostic(format!(
            "No {} session with cliSessionId {id} was found.",
            store.label()
        ));
    }
    CaptureError::Diagnostic(format!(
        "No extractable {} transcript was found. Cowork metadata: {} record(s); JSONL transcripts: {}; matches: {}; missing transcripts: {}.",
        store.label(),
        discovery.diagnostics.metadata_records_discovered,
        discovery.diagnostics.jsonl_transcripts_discovered,
        discovery.diagnostics.cowork_matches,
        discovery.diagnostics.unmatched_cowork_metadata,
    ))
}

/// Maps Claude Desktop's shell mode onto the local JSONL store it reads from.
///
/// Cowork and Claude Code are separate products with separate stores under
/// separate mode names, and neither is the web cache. `None` means the mode does
/// not name a local store — Home chat, or a mode too new for this build — which
/// leaves the caller to fall back to the web cache.
///
/// The second element is the `session_type` the UI shows for that store.
pub fn local_store_for_mode(
    mode: Option<&crate::capture::web_cache::ShellMode>,
) -> Option<(SessionStore, &'static str)> {
    use crate::capture::web_cache::ShellMode;

    match mode? {
        ShellMode::Code => Some((SessionStore::ClaudeCode, "code")),
        ShellMode::Cowork => Some((SessionStore::Cowork, "cowork")),
        ShellMode::Chat | ShellMode::Other(_) => None,
    }
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
    if let Some(effort) = &document.effort {
        markdown.push_str(&format!("| Effort | {} |\n", escape_table_cell(effort)));
    }
    markdown.push_str(&format!(
        "| Session source | {} |\n",
        escape_table_cell(&document.source_type)
    ));
    if let Some(metadata_path) = &document.desktop_metadata_path {
        markdown.push_str(&format!(
            "| Desktop metadata | {} |\n",
            escape_table_cell(metadata_path)
        ));
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

fn unix_timestamp_secs() -> Result<u64, CaptureError> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| CaptureError::Diagnostic(error.to_string()))?
        .as_secs())
}

#[cfg(test)]
mod store_tests {
    use super::SessionStore;
    use crate::capture::cowork::{SOURCE_CLAUDE_CODE, SOURCE_COWORK};
    use crate::models::LocalSessionSummary;

    fn session(source_type: &str) -> LocalSessionSummary {
        LocalSessionSummary {
            cli_session_id: "id".to_string(),
            desktop_session_id: None,
            title: "Title".to_string(),
            source_type: source_type.to_string(),
            transcript_available: true,
            transcript_path: None,
            metadata_path: None,
            metadata_modified_at: None,
            cwd: None,
            origin_cwd: None,
            model: None,
            effort: None,
            created_at: None,
            last_activity_at: None,
            last_focused_at: None,
            is_archived: None,
            title_source: None,
            permission_mode: None,
        }
    }

    /// Each store must resolve to its own directory. "Claude Code" resolving to
    /// `Any` would let a Cowork session answer a Claude Code request.
    #[test]
    fn stores_filter_the_unified_discovery_by_classification() {
        let cowork = session(SOURCE_COWORK);
        let code = session(SOURCE_CLAUDE_CODE);
        assert!(SessionStore::Cowork.includes(&cowork));
        assert!(!SessionStore::Cowork.includes(&code));
        assert!(SessionStore::ClaudeCode.includes(&code));
        assert!(!SessionStore::ClaudeCode.includes(&cowork));
        assert!(SessionStore::Any.includes(&cowork));
        assert!(SessionStore::Any.includes(&code));
    }

    #[test]
    fn stores_are_labelled_distinctly() {
        let labels = [
            SessionStore::Cowork.label(),
            SessionStore::ClaudeCode.label(),
            SessionStore::Any.label(),
        ];
        assert_eq!(
            labels
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            3
        );
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
        eprintln!(
            "newest Cowork session title: {:?}",
            metadata.and_then(|m| m.title)
        );

        let result = super::export_latest_session(
            super::SessionStore::Cowork,
            &crate::models::ChatExportOptions::default(),
        )
        .expect("Cowork export should succeed");
        eprintln!(
            "exported {:?} — {} messages\n  {}",
            result.title,
            result.message_count,
            result.markdown_path.as_deref().unwrap_or("not generated")
        );
        assert!(result.message_count > 0);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{coalesce_adjacent_messages, extract_message_text, is_human_user_message};
    use crate::models::ChatExportMessage;

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
