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

pub fn export_latest_code_transcript() -> Result<ChatExportResult, CaptureError> {
    let session = find_latest_session_candidate()?;
    let cli_session_id = session.metadata.cli_session_id.clone().ok_or_else(|| {
        CaptureError::Diagnostic(format!(
            "Claude session metadata at {} did not include a cliSessionId.",
            session.metadata_path.display()
        ))
    })?;
    let transcript_path = find_transcript_path(session.metadata.cwd.as_deref(), &cli_session_id)?;
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
        source_type: "Claude Code JSONL".to_string(),
        source_path: transcript_path.display().to_string(),
        markdown_path: markdown_path.display().to_string(),
        json_path: json_path.display().to_string(),
        message_count: messages.len(),
        warnings,
    })
}

pub fn latest_session_metadata() -> Option<LatestSessionMetadata> {
    find_latest_session_candidate()
        .ok()
        .map(|candidate| LatestSessionMetadata {
            title: candidate.metadata.title,
        })
}

fn find_latest_session_candidate() -> Result<SessionCandidate, CaptureError> {
    let mut candidates = Vec::new();
    for root in claude_code_session_roots()? {
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
                "No Claude Code session metadata was found for the current Windows user."
                    .to_string(),
            )
        })
}

fn claude_code_session_roots() -> Result<Vec<PathBuf>, CaptureError> {
    let mut roots = Vec::new();

    if let Some(appdata) = std::env::var_os("APPDATA") {
        roots.push(
            PathBuf::from(appdata)
                .join("Claude")
                .join("claude-code-sessions"),
        );
    }

    if let Some(local_appdata) = std::env::var_os("LOCALAPPDATA") {
        let packages = PathBuf::from(local_appdata).join("Packages");
        if packages.exists() {
            for entry in fs::read_dir(&packages)
                .map_err(|error| CaptureError::Diagnostic(error.to_string()))?
            {
                let entry = entry.map_err(|error| CaptureError::Diagnostic(error.to_string()))?;
                let name = entry.file_name().to_string_lossy().to_lowercase();
                if name.starts_with("claude_") {
                    roots.push(
                        entry
                            .path()
                            .join("LocalCache")
                            .join("Roaming")
                            .join("Claude")
                            .join("claude-code-sessions"),
                    );
                }
            }
        }
    }

    roots.retain(|root| root.exists());
    Ok(roots)
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
            collect_session_candidates(&path, candidates)?;
            continue;
        }

        let is_local_json = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("local_") && name.ends_with(".json"));
        if !is_local_json {
            continue;
        }

        let contents = fs::read_to_string(&path)
            .map_err(|error| CaptureError::Diagnostic(error.to_string()))?;
        let metadata: ClaudeCodeSessionMetadata = serde_json::from_str(&contents)
            .map_err(|error| CaptureError::Diagnostic(error.to_string()))?;
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

fn find_transcript_path(cwd: Option<&str>, cli_session_id: &str) -> Result<PathBuf, CaptureError> {
    let projects_root = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .ok_or_else(|| CaptureError::Diagnostic("USERPROFILE is not set.".to_string()))?
        .join(".claude")
        .join("projects");

    if let Some(cwd) = cwd {
        let direct = projects_root
            .join(claude_project_directory_name(cwd))
            .join(format!("{cli_session_id}.jsonl"));
        if direct.exists() {
            return Ok(direct);
        }
    }

    let filename = format!("{cli_session_id}.jsonl");
    find_file_recursive(&projects_root, &filename)?.ok_or_else(|| {
        CaptureError::Diagnostic(format!(
            "Could not find Claude transcript {filename} under {}.",
            projects_root.display()
        ))
    })
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
                    messages.push(ChatExportMessage {
                        role: "user".to_string(),
                        text,
                        timestamp: value
                            .get("timestamp")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                    });
                }
            }
            Some("assistant") => {
                if let Some(text) = extract_message_text(value.get("message")) {
                    messages.push(ChatExportMessage {
                        role: "claude".to_string(),
                        text,
                        timestamp: value
                            .get("timestamp")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                    });
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
            ChatExportMessage {
                role: "claude".to_string(),
                text: "One".to_string(),
                timestamp: Some("t1".to_string()),
            },
            ChatExportMessage {
                role: "claude".to_string(),
                text: "Two".to_string(),
                timestamp: Some("t2".to_string()),
            },
        ];

        let coalesced = coalesce_adjacent_messages(messages);
        assert_eq!(coalesced.len(), 1);
        assert_eq!(coalesced[0].text, "One\n\nTwo");
        assert_eq!(coalesced[0].timestamp.as_deref(), Some("t1"));
    }
}
