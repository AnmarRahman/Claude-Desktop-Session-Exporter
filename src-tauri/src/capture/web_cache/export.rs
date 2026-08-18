//! Renders a normalized session to the selected Markdown, JSON, and PDF formats.

use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::capture::output::{
    prepare_export_directory, reserve_export_paths, ExportFormats, ExportPaths,
};
use crate::capture::pdf::{self, PdfTranscript};
use crate::capture::progress::{self, ProgressStage};
use crate::capture::CaptureError;
use crate::filename::sanitize_filename_part;
use crate::models::{ChatExportBlock, ChatExportMessage, ChatExportOptions, ChatExportResult};

#[derive(Debug, Clone, Serialize)]
pub struct ExportDocument {
    pub title: String,
    pub session_id: String,
    pub source_type: String,
    pub source_path: String,
    pub model: Option<String>,
    pub summary: Option<String>,
    pub project_uuid: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub messages: Vec<ChatExportMessage>,
}

pub fn write_export(
    document: &ExportDocument,
    warnings: Vec<String>,
    options: &ChatExportOptions,
) -> Result<ChatExportResult, CaptureError> {
    let formats = ExportFormats::from_options(options)?;
    let exports_dir = prepare_export_directory(options.output_directory.as_deref())?;

    progress::report(ProgressStage::ReadingTranscript, 0, 0);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| CaptureError::Diagnostic(error.to_string()))?
        .as_secs();
    let title = sanitize_filename_part(&document.title, "Claude Session");
    let json = formats
        .json
        .then(|| serde_json::to_string_pretty(document))
        .transpose()
        .map_err(|error| CaptureError::Diagnostic(error.to_string()))?;
    let markdown = formats.markdown.then(|| render_markdown(document));
    let mut formats = formats;
    let (pdf_bytes, pdf_warnings) = if formats.pdf {
        match pdf::render_pdf(&PdfTranscript {
            title: &document.title,
            source_type: &document.source_type,
            session_id: &document.session_id,
            model: document.model.as_deref(),
            messages: &document.messages,
        }) {
            Ok((bytes, warnings)) => (Some(bytes), warnings),
            // An oversized transcript must not cost the user the formats that
            // did succeed, so the PDF alone is dropped.
            Err(CaptureError::PdfTooLarge(reason)) => {
                formats.pdf = false;
                (None, vec![reason])
            }
            Err(error) => return Err(error),
        }
    } else {
        (None, Vec::new())
    };
    let formats = formats;
    let paths = reserve_export_paths(&exports_dir, &title, timestamp, formats)?;

    // The selected files describe one export, so a partial set is worse than no
    // export. If any write fails, none of the selected files is left behind.
    progress::report(ProgressStage::WritingFiles, 0, 0);
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

    let mut warnings = warnings;
    warnings.extend(pdf_warnings);

    Ok(ChatExportResult {
        title: document.title.clone(),
        session_id: document.session_id.clone(),
        source_type: document.source_type.clone(),
        source_path: document.source_path.clone(),
        markdown_path: optional_path_string(paths.markdown),
        json_path: optional_path_string(paths.json),
        pdf_path: optional_path_string(paths.pdf),
        output_directory: exports_dir.display().to_string(),
        message_count: document.messages.len(),
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

pub fn render_markdown(document: &ExportDocument) -> String {
    let mut markdown = format!("# {}\n\n", document.title);

    markdown.push_str("| Field | Value |\n| --- | --- |\n");
    let mut row = |label: &str, value: &str| {
        markdown.push_str(&format!("| {label} | {} |\n", escape_table_cell(value)));
    };
    row("Session ID", &document.session_id);
    row("Source", &document.source_type);
    if let Some(model) = &document.model {
        row("Model", model);
    }
    if let Some(project_uuid) = &document.project_uuid {
        row("Project", project_uuid);
    }
    if let Some(summary) = &document.summary {
        row("Summary", summary);
    }
    if let Some(created_at) = &document.created_at {
        row("Created", created_at);
    }
    if let Some(updated_at) = &document.updated_at {
        row("Updated", updated_at);
    }
    row("Source path", &document.source_path);
    markdown.push('\n');

    for message in &document.messages {
        let heading = if message.role == "user" {
            "User"
        } else {
            "Claude"
        };
        match &message.timestamp {
            Some(timestamp) => markdown.push_str(&format!("## {heading} ({timestamp})\n\n")),
            None => markdown.push_str(&format!("## {heading}\n\n")),
        }
        for block in &message.blocks {
            markdown.push_str(&render_block(block));
        }
    }

    markdown
}

fn render_block(block: &ChatExportBlock) -> String {
    let text = block.text.as_deref().unwrap_or("").trim();

    let mut rendered = match block.kind.as_str() {
        "text" => format!("{text}\n\n"),
        "thinking" => format!("> **Thinking**\n{}\n\n", blockquote(text)),
        "tool_use" => {
            let mut section = format!(
                "**Tool — {}**\n\n",
                block.tool_name.as_deref().unwrap_or("unknown")
            );
            if !text.is_empty() {
                section.push_str(&format!("_{text}_\n\n"));
            }
            if let Some(input) = &block.tool_input {
                section.push_str(&fenced(input, "json"));
            }
            section
        }
        "tool_result" => {
            let status = if block.is_error == Some(true) {
                " (error)"
            } else {
                ""
            };
            let mut section = format!(
                "**Tool result — {}{status}**\n\n",
                block.tool_name.as_deref().unwrap_or("unknown")
            );
            if !text.is_empty() {
                section.push_str(&fenced(text, ""));
            }
            if let Some(raw) = &block.raw {
                section.push_str("_Result items this build cannot render:_\n\n");
                section.push_str(&fenced(raw, "json"));
            }
            section
        }
        "attachment" => {
            let label = block
                .references
                .first()
                .and_then(|reference| reference.label.as_deref())
                .unwrap_or("attachment");
            let mut section = format!("**Attachment — {label}**\n\n");
            if !text.is_empty() {
                section.push_str(&fenced(text, ""));
            }
            if let Some(raw) = &block.raw {
                section.push_str(&fenced(raw, "json"));
            }
            section
        }
        "file" => String::new(),
        _ => {
            let mut section = format!("**{}**\n\n", block.kind);
            if !text.is_empty() {
                section.push_str(&format!("{text}\n\n"));
            }
            if let Some(raw) = &block.raw {
                section.push_str(&fenced(raw, "json"));
            }
            section
        }
    };

    // An attachment's header already names its single reference.
    if block.kind != "attachment" && !block.references.is_empty() {
        for reference in &block.references {
            let label = reference
                .label
                .as_deref()
                .or(reference.url.as_deref())
                .unwrap_or(&reference.kind);
            match &reference.url {
                Some(url) => {
                    rendered.push_str(&format!("- [{}]({url})\n", escape_link_text(label)))
                }
                None => rendered.push_str(&format!("- {label}\n")),
            }
        }
        rendered.push('\n');
    }

    rendered
}

fn blockquote(text: &str) -> String {
    text.lines()
        .map(|line| format!("> {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Fences `text` with enough backticks to survive backticks inside it.
fn fenced(text: &str, language: &str) -> String {
    let longest_run = text
        .split(|character| character != '`')
        .map(str::len)
        .max()
        .unwrap_or(0);
    let fence = "`".repeat(longest_run.max(2) + 1);
    format!("{fence}{language}\n{}\n{fence}\n\n", text.trim_end())
}

fn escape_table_cell(value: &str) -> String {
    value.replace('|', "\\|").replace('\n', "<br>")
}

fn escape_link_text(value: &str) -> String {
    value.replace('[', "\\[").replace(']', "\\]")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ChatExportReference;

    fn document(messages: Vec<ChatExportMessage>) -> ExportDocument {
        ExportDocument {
            title: "Test | Session".to_string(),
            session_id: "conv-1".to_string(),
            source_type: "Claude Home web cache".to_string(),
            source_path: "/tmp/entry_0".to_string(),
            model: Some("claude-opus-5".to_string()),
            summary: None,
            project_uuid: None,
            created_at: None,
            updated_at: None,
            messages,
        }
    }

    fn all_formats() -> ExportFormats {
        ExportFormats {
            markdown: true,
            json: true,
            pdf: true,
        }
    }

    /// Two exports of one conversation in the same second must not collide.
    #[test]
    fn reserves_a_distinct_basename_per_export() {
        let dir = std::env::temp_dir().join("cse-export-reserve");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let first = reserve_export_paths(&dir, "Chat", 1786455779, all_formats()).unwrap();
        let second = reserve_export_paths(&dir, "Chat", 1786455779, all_formats()).unwrap();

        assert_ne!(first.markdown, second.markdown);
        assert_ne!(first.json, second.json);
        assert_ne!(first.pdf, second.pdf);
        assert!(
            first.markdown.as_ref().unwrap().exists(),
            "basename should be reserved on disk"
        );
        assert!(second
            .markdown
            .as_ref()
            .unwrap()
            .to_string_lossy()
            .contains("-2."));
    }

    /// A pre-existing JSON file must not leave an empty Markdown reservation.
    #[test]
    fn releases_the_reservation_when_only_the_json_name_is_taken() {
        let dir = std::env::temp_dir().join("cse-export-json-collision");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Chat-1786455779.json"), b"{}").unwrap();

        let paths = reserve_export_paths(&dir, "Chat", 1786455779, all_formats()).unwrap();

        assert!(paths
            .markdown
            .as_ref()
            .unwrap()
            .to_string_lossy()
            .contains("-2."));
        let orphans: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name() == "Chat-1786455779.md")
            .collect();
        assert!(orphans.is_empty(), "left an orphaned empty Markdown file");
        assert_eq!(
            std::fs::read(dir.join("Chat-1786455779.json")).unwrap(),
            b"{}"
        );
    }

    #[test]
    fn renders_metadata_table_with_escaped_pipes() {
        let markdown = render_markdown(&document(vec![]));
        assert!(markdown.starts_with("# Test | Session\n"));
        assert!(markdown.contains("| Session ID | conv-1 |"));
        assert!(markdown.contains("| Model | claude-opus-5 |"));
    }

    #[test]
    fn renders_roles_and_prose() {
        let markdown = render_markdown(&document(vec![
            ChatExportMessage::plain("user", "Hello".to_string(), Some("t1".to_string())),
            ChatExportMessage::plain("claude", "Hi there".to_string(), None),
        ]));
        assert!(markdown.contains("## User (t1)\n\nHello\n"));
        assert!(markdown.contains("## Claude\n\nHi there\n"));
    }

    #[test]
    fn writes_all_formats_to_the_selected_directory() {
        let dir = std::env::temp_dir().join(format!(
            "cse-selected-export-directory-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let export = write_export(
            &document(vec![ChatExportMessage::plain(
                "user",
                "Store this in my chosen folder.".to_string(),
                None,
            )]),
            vec![],
            &ChatExportOptions {
                output_directory: Some(dir.display().to_string()),
                ..ChatExportOptions::default()
            },
        )
        .unwrap();

        assert_eq!(export.output_directory, dir.display().to_string());
        for path in [&export.markdown_path, &export.json_path, &export.pdf_path] {
            let path = std::path::Path::new(path.as_ref().unwrap());
            assert_eq!(path.parent(), Some(dir.as_path()));
            assert!(path.is_file());
        }

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn writes_only_the_selected_formats() {
        let dir = std::env::temp_dir().join(format!(
            "cse-selected-export-formats-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let export = write_export(
            &document(vec![ChatExportMessage::plain(
                "user",
                "PDF only, please.".to_string(),
                None,
            )]),
            vec![],
            &ChatExportOptions {
                output_directory: Some(dir.display().to_string()),
                export_markdown: Some(false),
                export_json: Some(false),
                export_pdf: Some(true),
                ..ChatExportOptions::default()
            },
        )
        .unwrap();

        assert!(export.markdown_path.is_none());
        assert!(export.json_path.is_none());
        assert!(std::path::Path::new(export.pdf_path.as_ref().unwrap()).is_file());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn renders_tool_activity_with_input_and_citations() {
        let message = ChatExportMessage {
            role: "claude".to_string(),
            text: String::new(),
            timestamp: None,
            blocks: vec![
                ChatExportBlock {
                    text: Some("Searching the web".to_string()),
                    tool_name: Some("web_search".to_string()),
                    tool_input: Some("{\n  \"query\": \"odoo\"\n}".to_string()),
                    ..ChatExportBlock::empty("tool_use")
                },
                ChatExportBlock {
                    text: Some("result body".to_string()),
                    tool_name: Some("web_search".to_string()),
                    is_error: Some(true),
                    references: vec![ChatExportReference {
                        kind: "knowledge".to_string(),
                        label: Some("Odoo [docs]".to_string()),
                        url: Some("https://odoo.com/x".to_string()),
                    }],
                    ..ChatExportBlock::empty("tool_result")
                },
            ],
        };
        let markdown = render_markdown(&document(vec![message]));
        assert!(markdown.contains("**Tool — web_search**"));
        assert!(markdown.contains("_Searching the web_"));
        assert!(markdown.contains("```json\n{\n  \"query\": \"odoo\"\n}\n```"));
        assert!(markdown.contains("**Tool result — web_search (error)**"));
        assert!(markdown.contains("- [Odoo \\[docs\\]](https://odoo.com/x)"));
    }

    #[test]
    fn thinking_is_quoted_line_by_line() {
        let message = ChatExportMessage {
            role: "claude".to_string(),
            text: String::new(),
            timestamp: None,
            blocks: vec![ChatExportBlock {
                text: Some("first\nsecond".to_string()),
                ..ChatExportBlock::empty("thinking")
            }],
        };
        let markdown = render_markdown(&document(vec![message]));
        assert!(markdown.contains("> **Thinking**\n> first\n> second"));
    }

    /// A code fence inside tool output must not terminate the wrapper fence.
    #[test]
    fn fences_grow_past_backticks_in_content() {
        let fenced_output = fenced("a ``` b", "");
        assert!(fenced_output.starts_with("````\n"));
        assert!(fenced_output.ends_with("````\n\n"));
    }
}
