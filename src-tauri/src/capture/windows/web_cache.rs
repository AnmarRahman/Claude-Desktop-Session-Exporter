use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::capture::CaptureError;
use crate::filename::sanitize_filename_part;
use crate::models::{ChatExportMessage, ChatExportResult};

#[derive(Debug, Clone)]
pub struct LatestSessionMetadata {
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellMode {
    Chat,
    Code,
    Other(String),
}

#[derive(Debug, Clone)]
struct ConversationCandidate {
    conversation: WebConversation,
    source_path: PathBuf,
    modified_unix_ms: u64,
    hit_index: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct WebConversation {
    uuid: String,
    name: Option<String>,
    model: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
    chat_messages: Vec<WebMessage>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct WebMessage {
    sender: Option<String>,
    text: Option<String>,
    content: Option<Vec<WebContentBlock>>,
    created_at: Option<String>,
    updated_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct WebContentBlock {
    #[serde(rename = "type")]
    block_type: Option<String>,
    text: Option<String>,
}

#[derive(Debug, Serialize)]
struct WebExportDocument {
    title: String,
    session_id: String,
    model: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
    source_path: String,
    messages: Vec<ChatExportMessage>,
}

pub fn latest_session_metadata() -> Option<LatestSessionMetadata> {
    find_latest_web_conversation()
        .ok()
        .map(|candidate| LatestSessionMetadata {
            title: candidate.conversation.name,
        })
}

pub fn latest_shell_mode() -> Option<ShellMode> {
    for root in local_storage_roots().ok()? {
        let mut files = fs::read_dir(root)
            .ok()?
            .filter_map(Result::ok)
            .filter(|entry| entry.path().is_file())
            .collect::<Vec<_>>();
        files.sort_by_key(|entry| {
            entry
                .metadata()
                .ok()
                .and_then(|metadata| metadata.modified().ok())
                .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_millis() as u64)
                .unwrap_or(0)
        });
        files.reverse();

        for entry in files {
            if entry.file_name().to_string_lossy() == "LOCK" {
                continue;
            }
            let bytes = fs::read(entry.path()).ok()?;
            for text in [
                String::from_utf8_lossy(&bytes).to_string(),
                decode_utf16le_lossy(&bytes),
            ] {
                if let Some(mode) = extract_last_known_mode(&text) {
                    return Some(mode);
                }
            }
        }
    }

    None
}

pub fn export_latest_web_cache_transcript() -> Result<ChatExportResult, CaptureError> {
    let candidate = find_latest_web_conversation()?;
    let messages = web_messages_to_export_messages(&candidate.conversation.chat_messages);

    if messages.is_empty() {
        return Err(CaptureError::Diagnostic(format!(
            "Claude Home/Cowork cache was found at {}, but no user or Claude text messages were readable.",
            candidate.source_path.display()
        )));
    }

    let title = candidate
        .conversation
        .name
        .clone()
        .filter(|value| !value.trim().is_empty())
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

    let document = WebExportDocument {
        title: title.clone(),
        session_id: candidate.conversation.uuid.clone(),
        model: candidate.conversation.model.clone(),
        created_at: candidate.conversation.created_at.clone(),
        updated_at: candidate.conversation.updated_at.clone(),
        source_path: candidate.source_path.display().to_string(),
        messages: messages.clone(),
    };

    let json = serde_json::to_string_pretty(&document)
        .map_err(|error| CaptureError::Diagnostic(error.to_string()))?;
    fs::write(&json_path, json).map_err(|error| CaptureError::Diagnostic(error.to_string()))?;
    fs::write(&markdown_path, render_markdown(&document))
        .map_err(|error| CaptureError::Diagnostic(error.to_string()))?;

    Ok(ChatExportResult {
        title,
        session_id: candidate.conversation.uuid,
        source_type: "Claude Home/Cowork cache".to_string(),
        source_path: candidate.source_path.display().to_string(),
        markdown_path: markdown_path.display().to_string(),
        json_path: json_path.display().to_string(),
        message_count: messages.len(),
        warnings: vec![
            "Exported from Claude Desktop's local web cache. Open the target Home/Cowork chat first, then retry if this is not the chat you expected.".to_string(),
        ],
    })
}

fn find_latest_web_conversation() -> Result<ConversationCandidate, CaptureError> {
    let mut candidates = Vec::new();
    for root in local_storage_roots()? {
        collect_web_conversations(&root, &mut candidates)?;
    }

    let mut by_uuid: HashMap<String, ConversationCandidate> = HashMap::new();
    for candidate in candidates {
        by_uuid
            .entry(candidate.conversation.uuid.clone())
            .and_modify(|existing| {
                if candidate_sort_key(&candidate) > candidate_sort_key(existing) {
                    *existing = candidate.clone();
                }
            })
            .or_insert(candidate);
    }

    by_uuid
        .into_values()
        .max_by_key(candidate_sort_key)
        .ok_or_else(|| {
            CaptureError::Diagnostic(
                "No regular Claude Home/Cowork chat transcript was found in the local web cache. Open the chat in Claude Desktop, wait for it to load, then retry."
                    .to_string(),
            )
        })
}

fn candidate_sort_key(candidate: &ConversationCandidate) -> (u64, usize, usize) {
    (
        candidate.modified_unix_ms,
        candidate.hit_index,
        candidate.conversation.chat_messages.len(),
    )
}

fn local_storage_roots() -> Result<Vec<PathBuf>, CaptureError> {
    let mut roots = Vec::new();

    if let Some(appdata) = std::env::var_os("APPDATA") {
        roots.push(
            PathBuf::from(appdata)
                .join("Claude")
                .join("Local Storage")
                .join("leveldb"),
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
                            .join("Local Storage")
                            .join("leveldb"),
                    );
                }
            }
        }
    }

    roots.retain(|root| root.exists());
    Ok(roots)
}

fn collect_web_conversations(
    directory: &Path,
    candidates: &mut Vec<ConversationCandidate>,
) -> Result<(), CaptureError> {
    for entry in
        fs::read_dir(directory).map_err(|error| CaptureError::Diagnostic(error.to_string()))?
    {
        let entry = entry.map_err(|error| CaptureError::Diagnostic(error.to_string()))?;
        let path = entry.path();
        if !path.is_file() || path.file_name().and_then(|name| name.to_str()) == Some("LOCK") {
            continue;
        }

        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(_) => continue,
        };
        let modified_unix_ms = entry
            .metadata()
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(0);

        collect_conversations_from_text(
            &String::from_utf8_lossy(&bytes),
            &path,
            modified_unix_ms,
            candidates,
        );
        collect_conversations_from_text(
            &decode_utf16le_lossy(&bytes),
            &path,
            modified_unix_ms,
            candidates,
        );
    }

    Ok(())
}

fn collect_conversations_from_text(
    text: &str,
    path: &Path,
    modified_unix_ms: u64,
    candidates: &mut Vec<ConversationCandidate>,
) {
    let mut search_start = 0;
    while let Some(relative_index) = text[search_start..].find("\"chat_messages\"") {
        let hit_index = search_start + relative_index;
        if let Some(conversation) = parse_conversation_near_hit(text, hit_index) {
            candidates.push(ConversationCandidate {
                conversation,
                source_path: path.to_path_buf(),
                modified_unix_ms,
                hit_index,
            });
        }
        search_start = hit_index + "\"chat_messages\"".len();
    }
}

fn parse_conversation_near_hit(text: &str, hit_index: usize) -> Option<WebConversation> {
    let window_start = hit_index.saturating_sub(200_000);
    let prefix = &text[window_start..hit_index];
    let mut starts = prefix
        .match_indices('{')
        .map(|(index, _)| window_start + index)
        .collect::<Vec<_>>();
    starts.reverse();

    for start in starts {
        let Some(end) = find_json_object_end(text, start) else {
            continue;
        };
        if end <= hit_index {
            continue;
        }
        let candidate = &text[start..end];
        if !(candidate.contains("\"uuid\"")
            && candidate.contains("\"name\"")
            && candidate.contains("\"chat_messages\""))
        {
            continue;
        }
        if let Ok(conversation) = serde_json::from_str::<WebConversation>(candidate) {
            return Some(conversation);
        }
    }

    None
}

fn find_json_object_end(text: &str, start: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (offset, character) in text[start..].char_indices() {
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
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(start + offset + character.len_utf8());
                }
            }
            _ => {}
        }
    }

    None
}

fn web_messages_to_export_messages(messages: &[WebMessage]) -> Vec<ChatExportMessage> {
    messages
        .iter()
        .filter_map(|message| {
            let role = match message.sender.as_deref() {
                Some("human") => "user",
                Some("assistant") => "claude",
                _ => return None,
            };
            let text = extract_web_message_text(message)?;
            Some(ChatExportMessage {
                role: role.to_string(),
                text,
                timestamp: message.created_at.clone(),
            })
        })
        .collect()
}

fn extract_web_message_text(message: &WebMessage) -> Option<String> {
    let mut blocks = Vec::new();
    if let Some(content) = &message.content {
        for block in content {
            if block.block_type.as_deref() == Some("text") {
                if let Some(text) = block.text.as_deref().map(str::trim) {
                    if !text.is_empty() {
                        blocks.push(text.to_string());
                    }
                }
            }
        }
    }

    if blocks.is_empty() {
        message
            .text
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
    } else {
        Some(blocks.join("\n\n"))
    }
}

fn render_markdown(document: &WebExportDocument) -> String {
    let mut markdown = String::new();
    markdown.push_str("# ");
    markdown.push_str(&document.title);
    markdown.push_str("\n\n");
    markdown.push_str("| Field | Value |\n| --- | --- |\n");
    markdown.push_str(&format!(
        "| Session ID | {} |\n",
        escape_table_cell(&document.session_id)
    ));
    if let Some(model) = &document.model {
        markdown.push_str(&format!("| Model | {} |\n", escape_table_cell(model)));
    }
    if let Some(created_at) = &document.created_at {
        markdown.push_str(&format!(
            "| Created | {} |\n",
            escape_table_cell(created_at)
        ));
    }
    if let Some(updated_at) = &document.updated_at {
        markdown.push_str(&format!(
            "| Updated | {} |\n",
            escape_table_cell(updated_at)
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

fn decode_utf16le_lossy(bytes: &[u8]) -> String {
    let units = bytes
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]));
    char::decode_utf16(units)
        .map(|result| result.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
}

fn extract_last_known_mode(text: &str) -> Option<ShellMode> {
    let marker = "\"lastKnownMode\":\"";
    let start = text.rfind(marker)? + marker.len();
    let end = text[start..].find('"')? + start;
    match &text[start..end] {
        "chat" | "home" | "task" => Some(ShellMode::Chat),
        "code" => Some(ShellMode::Code),
        other => Some(ShellMode::Other(other.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        extract_last_known_mode, find_json_object_end, parse_conversation_near_hit,
        web_messages_to_export_messages, ShellMode,
    };

    #[test]
    fn extracts_conversation_object_from_noisy_storage_text() {
        let text = r#"noise {"uuid":"abc","name":"Regular Chat","model":"claude","chat_messages":[{"sender":"human","content":[{"type":"text","text":"Hello"}],"created_at":"t1"},{"sender":"assistant","content":[{"type":"thinking","text":"hidden"},{"type":"text","text":"Hi"}],"created_at":"t2"}]} trailing"#;
        let hit = text.find("\"chat_messages\"").unwrap();
        let conversation = parse_conversation_near_hit(text, hit).unwrap();
        assert_eq!(conversation.uuid, "abc");
        assert_eq!(conversation.name.as_deref(), Some("Regular Chat"));
        let messages = web_messages_to_export_messages(&conversation.chat_messages);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].text, "Hello");
        assert_eq!(messages[1].role, "claude");
        assert_eq!(messages[1].text, "Hi");
    }

    #[test]
    fn finds_json_end_with_braces_inside_strings() {
        let text = r#"{"text":"brace } in string","nested":{"ok":true}} trailing"#;
        let end = find_json_object_end(text, 0).unwrap();
        assert_eq!(
            &text[..end],
            r#"{"text":"brace } in string","nested":{"ok":true}}"#
        );
    }

    #[test]
    fn reads_latest_shell_mode_from_dframe_store() {
        let text = r#"old {"lastKnownMode":"code"} new {"lastKnownMode":"chat"}"#;
        assert_eq!(extract_last_known_mode(text), Some(ShellMode::Chat));
    }
}
