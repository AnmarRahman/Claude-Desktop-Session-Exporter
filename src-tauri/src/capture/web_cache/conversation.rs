//! The `chat_conversations` payload Claude Desktop caches, and its mapping onto
//! the exporter's normalized message model.
//!
//! Field names mirror the cached JSON. Everything is optional on purpose: the
//! payload is a moving target, and a renamed field should degrade one block
//! rather than fail the whole export.

use serde::Deserialize;

use crate::models::{ChatExportBlock, ChatExportMessage, ChatExportReference};

#[derive(Debug, Clone, Deserialize)]
pub struct Conversation {
    pub uuid: String,
    pub name: Option<String>,
    pub summary: Option<String>,
    pub model: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub project_uuid: Option<String>,
    pub chat_messages: Option<Vec<Message>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Message {
    pub sender: Option<String>,
    /// Historically the whole message text; empty on current payloads, which
    /// carry everything in `content`.
    pub text: Option<String>,
    /// Left untyped so that one block whose shape changed upstream degrades to a
    /// preserved `unknown:` block instead of failing the whole conversation.
    pub content: Option<Vec<serde_json::Value>>,
    pub created_at: Option<String>,
    pub attachments: Option<Vec<serde_json::Value>>,
    pub files: Option<Vec<serde_json::Value>>,
}

/// One entry of a message's `content` array.
///
/// The array is heterogeneous (`text`, `thinking`, `tool_use`, `tool_result`),
/// so this is a flat superset rather than a tagged enum: an unrecognized `type`
/// still round-trips whatever text it carries.
#[derive(Debug, Clone, Deserialize)]
pub struct ContentBlock {
    #[serde(rename = "type")]
    pub block_type: Option<String>,
    pub text: Option<String>,
    pub thinking: Option<String>,
    /// Tool name on both `tool_use` and `tool_result`.
    pub name: Option<String>,
    pub input: Option<serde_json::Value>,
    /// The status line Claude shows on a tool card, e.g. "Searching the web".
    pub message: Option<String>,
    pub is_error: Option<bool>,
    /// Present on `tool_result`; the tool's returned items. Untyped so an item
    /// this build cannot represent keeps its payload instead of vanishing.
    pub content: Option<Vec<serde_json::Value>>,
    /// Fields this build does not know about. A schema addition must not
    /// disappear just because the rest of the block still parsed.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolResultItem {
    #[serde(rename = "type")]
    pub item_type: Option<String>,
    pub text: Option<String>,
    pub title: Option<String>,
    pub url: Option<String>,
    pub name: Option<String>,
    pub file_path: Option<String>,
    pub mime_type: Option<String>,
    /// Fields this build does not know about. A schema addition must not
    /// disappear just because the rest of the block still parsed.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Attachment {
    pub file_name: Option<String>,
    pub file_size: Option<u64>,
    pub file_type: Option<String>,
    pub extracted_content: Option<String>,
    /// Fields this build does not know about. A schema addition must not
    /// disappear just because the rest of the block still parsed.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FileRef {
    pub file_name: Option<String>,
    pub file_kind: Option<String>,
    pub path: Option<String>,
    /// Fields this build does not know about. A schema addition must not
    /// disappear just because the rest of the block still parsed.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Copy)]
pub struct NormalizeOptions {
    pub include_thinking: bool,
    pub include_tools: bool,
}

impl Default for NormalizeOptions {
    fn default() -> Self {
        Self {
            include_thinking: false,
            include_tools: true,
        }
    }
}

impl Conversation {
    pub fn display_title(&self) -> Option<String> {
        self.name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string)
    }

    pub fn to_export_messages(&self, options: NormalizeOptions) -> Vec<ChatExportMessage> {
        self.chat_messages
            .as_deref()
            .unwrap_or_default()
            .iter()
            .filter_map(|message| message.to_export_message(options))
            .collect()
    }
}

impl Message {
    fn to_export_message(&self, options: NormalizeOptions) -> Option<ChatExportMessage> {
        let role = match self.sender.as_deref() {
            Some("human") | Some("user") => "user",
            Some("assistant") => "claude",
            _ => return None,
        };

        let mut blocks = Vec::new();
        for attachment in self.attachments.as_deref().unwrap_or_default() {
            blocks.push(
                parse_or_raw::<Attachment>(attachment, "attachment", Attachment::to_block),
            );
        }
        for block in self.content.as_deref().unwrap_or_default() {
            blocks.extend(block_from_value(block, options));
        }

        // Pre-`content` payloads put the prose in the flat `text` field, and a
        // transitional payload can carry it alongside non-text blocks. Fall back
        // whenever `content` produced no prose, not only when it produced
        // nothing at all.
        if !blocks.iter().any(|block| block.kind == "text") {
            if let Some(text) = non_empty(self.text.as_deref()) {
                blocks.push(ChatExportBlock::text(text));
            }
        }

        for file in self.files.as_deref().unwrap_or_default() {
            blocks.push(parse_or_raw::<FileRef>(file, "file", FileRef::to_block));
        }

        if blocks.is_empty() {
            return None;
        }

        Some(ChatExportMessage {
            role: role.to_string(),
            text: plain_text(&blocks),
            timestamp: self.created_at.clone(),
            blocks,
        })
    }
}

/// Turns one raw `content` entry into a block, tolerating shape changes.
///
/// A block whose JSON no longer matches [`ContentBlock`] — a `content` field
/// that became a string, say — is preserved as an `unknown:` block carrying its
/// full payload, instead of failing the deserialization of the entire
/// conversation.
fn block_from_value(
    value: &serde_json::Value,
    options: NormalizeOptions,
) -> Option<ChatExportBlock> {
    let block_type = value
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");

    // Options filter blocks out deliberately; that is not a parse failure and
    // must not resurrect them as raw payloads.
    match block_type {
        "thinking" if !options.include_thinking => return None,
        "tool_use" | "tool_result" if !options.include_tools => return None,
        _ => {}
    }

    let parsed = serde_json::from_value::<ContentBlock>(value.clone()).ok();
    let exported = parsed
        .as_ref()
        .filter(|_| is_known_block_type(block_type))
        .and_then(ContentBlock::to_export_block);

    // Serde accepts a block whose fields were all renamed, because every field
    // is optional — so "parsed" is not the same as "understood". A block that
    // produced nothing recognizable is preserved verbatim rather than dropped.
    match exported {
        Some(block) if !is_empty_block(&block) => {
            let unrepresented = parsed.is_some_and(|parsed| !parsed.extra.is_empty())
                || tool_result_items_unrepresented(value);
            Some(attach_raw_if_unrepresented(block, value, unrepresented))
        }
        _ => Some(ChatExportBlock {
            text: value
                .get("text")
                .or_else(|| value.get("message"))
                .and_then(serde_json::Value::as_str)
                .and_then(|text| non_empty(Some(text))),
            ..raw_block(&format!("unknown:{block_type}"), value)
        }),
    }
}

/// Types that retain the payload fields this build does not name.
trait HasExtra {
    fn extra(&self) -> &serde_json::Map<String, serde_json::Value>;
}

macro_rules! impl_has_extra {
    ($($type:ty),+) => {
        $(impl HasExtra for $type {
            fn extra(&self) -> &serde_json::Map<String, serde_json::Value> {
                &self.extra
            }
        })+
    };
}
impl_has_extra!(ContentBlock, ToolResultItem, Attachment, FileRef);

/// A block preserved verbatim, for payloads this build cannot represent.
fn raw_block(kind: &str, value: &serde_json::Value) -> ChatExportBlock {
    ChatExportBlock {
        raw: serde_json::to_string_pretty(value).ok(),
        ..ChatExportBlock::empty(kind)
    }
}

/// Attaches the original JSON when the payload carried fields this build does
/// not understand, so a schema addition survives even though the block itself
/// parsed and rendered fine.
fn attach_raw_if_unrepresented(
    mut block: ChatExportBlock,
    value: &serde_json::Value,
    unrepresented: bool,
) -> ChatExportBlock {
    if block.raw.is_none() && unrepresented {
        block.raw = serde_json::to_string_pretty(value).ok();
    }
    block
}

/// True when any `tool_result` item carries a field this build does not name,
/// or no longer parses at all.
fn tool_result_items_unrepresented(value: &serde_json::Value) -> bool {
    value
        .get("content")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|items| {
            items.iter().any(|item| {
                serde_json::from_value::<ToolResultItem>(item.clone())
                    .map(|item| !item.extra.is_empty())
                    .unwrap_or(true)
            })
        })
}

/// True when a block carries nothing a reader could use.
fn is_empty_block(block: &ChatExportBlock) -> bool {
    block.text.is_none()
        && block.tool_name.is_none()
        && block.tool_input.is_none()
        && block.raw.is_none()
        && block
            .references
            .iter()
            .all(|reference| reference.label.is_none() && reference.url.is_none())
}

fn is_known_block_type(block_type: &str) -> bool {
    matches!(block_type, "text" | "thinking" | "tool_use" | "tool_result")
}

/// Maps one raw item through `to_block`, or preserves it verbatim if its shape
/// no longer parses.
fn parse_or_raw<T: serde::de::DeserializeOwned + HasExtra>(
    value: &serde_json::Value,
    kind: &str,
    to_block: impl Fn(&T) -> ChatExportBlock,
) -> ChatExportBlock {
    // Renamed fields deserialize fine into all-optional structs, so an item that
    // parsed but yielded nothing is treated the same as one that failed to parse.
    let parsed = serde_json::from_value::<T>(value.clone()).ok();
    let block = parsed.as_ref().map(|parsed| to_block(parsed));

    match block {
        Some(block) if !is_empty_block(&block) => attach_raw_if_unrepresented(
            block,
            value,
            parsed.is_some_and(|parsed| !parsed.extra().is_empty()),
        ),
        _ => raw_block(&format!("unknown:{kind}"), value),
    }
}

impl ContentBlock {
    /// Options are applied by [`block_from_value`] before this runs, so `None`
    /// here always means "nothing recognizable", never "filtered out".
    fn to_export_block(&self) -> Option<ChatExportBlock> {
        match self.block_type.as_deref().unwrap_or("") {
            "text" => Some(ChatExportBlock::text(non_empty(self.text.as_deref())?)),
            "thinking" => non_empty(self.thinking.as_deref()).map(|thinking| ChatExportBlock {
                text: Some(thinking),
                ..ChatExportBlock::empty("thinking")
            }),
            "tool_use" => Some(ChatExportBlock {
                text: non_empty(self.message.as_deref()),
                tool_name: non_empty(self.name.as_deref()),
                tool_input: self
                    .input
                    .as_ref()
                    .and_then(|input| serde_json::to_string_pretty(input).ok()),
                ..ChatExportBlock::empty("tool_use")
            }),
            "tool_result" => {
                let items = self.content.as_deref().unwrap_or_default();
                let mut texts = Vec::new();
                let mut references = Vec::new();
                let mut unrepresented = Vec::new();

                for item in items {
                    let parsed = serde_json::from_value::<ToolResultItem>(item.clone()).ok();
                    let text = parsed
                        .as_ref()
                        .and_then(|item| non_empty(item.text.as_deref()));
                    let reference = parsed.as_ref().and_then(ToolResultItem::to_reference);

                    match (text, reference) {
                        (None, None) => unrepresented.push(item.clone()),
                        (text, reference) => {
                            texts.extend(text);
                            references.extend(reference);
                        }
                    }
                }

                Some(ChatExportBlock {
                    text: join_non_empty(texts.into_iter())
                        .or_else(|| non_empty(self.message.as_deref())),
                    tool_name: non_empty(self.name.as_deref()),
                    is_error: self.is_error,
                    references,
                    // An item this build cannot render — an image carrying only
                    // `source`, say — is kept rather than dropped.
                    raw: (!unrepresented.is_empty())
                        .then(|| serde_json::to_string_pretty(&unrepresented).ok())
                        .flatten(),
                    ..ChatExportBlock::empty("tool_result")
                })
            }
            // Unreachable: `block_from_value` routes unknown types itself, so
            // that the preserved payload is the whole block rather than only the
            // fields this struct does not name.
            other => Some(ChatExportBlock {
                text: non_empty(self.text.as_deref())
                    .or_else(|| non_empty(self.message.as_deref())),
                ..ChatExportBlock::empty(&format!("unknown:{other}"))
            }),
        }
    }
}

impl ToolResultItem {
    /// Non-text results (search hits, files, images) become citations rather
    /// than inline text.
    fn to_reference(&self) -> Option<ChatExportReference> {
        match self.item_type.as_deref().unwrap_or("") {
            "text" => None,
            kind => {
                let label = non_empty(self.title.as_deref())
                    .or_else(|| non_empty(self.name.as_deref()))
                    .or_else(|| non_empty(self.file_path.as_deref()))
                    .or_else(|| non_empty(self.mime_type.as_deref()));
                let url = non_empty(self.url.as_deref());
                (label.is_some() || url.is_some()).then(|| ChatExportReference {
                    kind: kind.to_string(),
                    label,
                    url,
                })
            }
        }
    }
}

impl Attachment {
    fn to_block(&self) -> ChatExportBlock {
        ChatExportBlock {
            text: non_empty(self.extracted_content.as_deref()),
            references: vec![ChatExportReference {
                kind: non_empty(self.file_type.as_deref()).unwrap_or_else(|| "file".to_string()),
                label: non_empty(self.file_name.as_deref())
                    .or_else(|| self.file_size.map(|size| format!("{size} bytes"))),
                url: None,
            }],
            ..ChatExportBlock::empty("attachment")
        }
    }
}

impl FileRef {
    fn to_block(&self) -> ChatExportBlock {
        ChatExportBlock {
            references: vec![ChatExportReference {
                kind: non_empty(self.file_kind.as_deref()).unwrap_or_else(|| "file".to_string()),
                label: non_empty(self.file_name.as_deref())
                    .or_else(|| non_empty(self.path.as_deref())),
                url: None,
            }],
            ..ChatExportBlock::empty("file")
        }
    }
}

/// The message's plain conversation text, excluding thinking and tool activity.
fn plain_text(blocks: &[ChatExportBlock]) -> String {
    join_non_empty(
        blocks
            .iter()
            .filter(|block| block.kind == "text")
            .filter_map(|block| block.text.clone()),
    )
    .unwrap_or_default()
}

fn join_non_empty(values: impl Iterator<Item = String>) -> Option<String> {
    let joined = values.collect::<Vec<_>>().join("\n\n");
    (!joined.trim().is_empty()).then_some(joined)
}

fn non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shaped after a real cached payload, including the empty top-level `text`.
    const PAYLOAD: &str = r#"{
      "uuid": "conv-1",
      "name": "  Odoo quote to cash  ",
      "model": "claude-opus-5",
      "created_at": "2026-03-11T15:10:54Z",
      "chat_messages": [
        {
          "sender": "human",
          "text": "",
          "created_at": "2026-03-11T15:12:21Z",
          "content": [{"type": "text", "text": "List the transaction states"}],
          "attachments": [
            {"file_name": "notes.txt", "file_size": 42, "file_type": "txt",
             "extracted_content": "attached notes"}
          ],
          "files": []
        },
        {
          "sender": "assistant",
          "text": "",
          "created_at": "2026-03-11T15:12:23Z",
          "content": [
            {"type": "thinking", "thinking": "internal reasoning"},
            {"type": "tool_use", "name": "web_search", "message": "Searching the web",
             "input": {"query": "odoo states"}},
            {"type": "tool_result", "name": "web_search", "is_error": false,
             "content": [
               {"type": "text", "text": "result body"},
               {"type": "knowledge", "title": "Odoo docs", "url": "https://odoo.com/x"}
             ]},
            {"type": "text", "text": "Here are the states."}
          ]
        },
        {"sender": "system", "content": [{"type": "text", "text": "ignored"}]}
      ]
    }"#;

    fn parse() -> Conversation {
        serde_json::from_str(PAYLOAD).expect("payload should parse")
    }

    #[test]
    fn reads_conversation_metadata() {
        let conversation = parse();
        assert_eq!(conversation.uuid, "conv-1");
        assert_eq!(
            conversation.display_title().as_deref(),
            Some("Odoo quote to cash")
        );
    }

    #[test]
    fn keeps_only_human_and_assistant_turns() {
        let messages = parse().to_export_messages(NormalizeOptions::default());
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[1].role, "claude");
    }

    #[test]
    fn plain_text_excludes_thinking_and_tool_activity() {
        let messages = parse().to_export_messages(NormalizeOptions {
            include_thinking: true,
            include_tools: true,
        });
        assert_eq!(messages[1].text, "Here are the states.");
    }

    #[test]
    fn thinking_is_opt_in_and_tools_are_opt_out() {
        let default = parse().to_export_messages(NormalizeOptions::default());
        let kinds: Vec<&str> = default[1].blocks.iter().map(|b| b.kind.as_str()).collect();
        assert_eq!(kinds, ["tool_use", "tool_result", "text"]);

        let with_thinking = parse().to_export_messages(NormalizeOptions {
            include_thinking: true,
            include_tools: false,
        });
        let kinds: Vec<&str> = with_thinking[1]
            .blocks
            .iter()
            .map(|b| b.kind.as_str())
            .collect();
        assert_eq!(kinds, ["thinking", "text"]);
    }

    #[test]
    fn tool_blocks_keep_name_input_and_citations() {
        let messages = parse().to_export_messages(NormalizeOptions::default());
        let tool_use = &messages[1].blocks[0];
        assert_eq!(tool_use.tool_name.as_deref(), Some("web_search"));
        assert!(tool_use.tool_input.as_deref().unwrap().contains("odoo states"));

        let tool_result = &messages[1].blocks[1];
        assert_eq!(tool_result.text.as_deref(), Some("result body"));
        assert_eq!(tool_result.references.len(), 1);
        assert_eq!(tool_result.references[0].kind, "knowledge");
        assert_eq!(
            tool_result.references[0].url.as_deref(),
            Some("https://odoo.com/x")
        );
    }

    #[test]
    fn attachments_precede_message_content() {
        let messages = parse().to_export_messages(NormalizeOptions::default());
        let blocks = &messages[0].blocks;
        assert_eq!(blocks[0].kind, "attachment");
        assert_eq!(blocks[0].text.as_deref(), Some("attached notes"));
        assert_eq!(blocks[0].references[0].label.as_deref(), Some("notes.txt"));
        assert_eq!(blocks[1].kind, "text");
    }

    #[test]
    fn unknown_block_types_are_preserved_rather_than_dropped() {
        let payload = r#"{"uuid":"c","chat_messages":[{"sender":"assistant",
            "content":[{"type":"future_widget","text":"still readable"}]}]}"#;
        let conversation: Conversation = serde_json::from_str(payload).unwrap();
        let messages = conversation.to_export_messages(NormalizeOptions::default());
        assert_eq!(messages[0].blocks[0].kind, "unknown:future_widget");
        assert_eq!(messages[0].blocks[0].text.as_deref(), Some("still readable"));
    }

    /// A new field alongside recognized ones must still be preserved: the block
    /// renders fine, so nothing else would signal that data was dropped.
    #[test]
    fn keeps_new_fields_that_sit_beside_recognized_ones() {
        let payload = r#"{"uuid":"c","chat_messages":[{"sender":"human","text":"hi",
            "attachments":[{"file_name":"notes.txt","download_url":"https://x/y"}]}]}"#;
        let conversation: Conversation = serde_json::from_str(payload).unwrap();
        let messages = conversation.to_export_messages(NormalizeOptions::default());
        let block = &messages[0].blocks[0];

        assert_eq!(block.kind, "attachment");
        assert_eq!(block.references[0].label.as_deref(), Some("notes.txt"));
        let raw = block.raw.as_deref().expect("new field must be kept");
        assert!(raw.contains("download_url"), "{raw}");
    }

    /// Same for a tool-result item that is partly understood.
    #[test]
    fn keeps_new_fields_on_tool_result_items() {
        let payload = r#"{"uuid":"c","chat_messages":[{"sender":"assistant","content":[
            {"type":"tool_result","name":"view","content":[
              {"type":"image","mime_type":"image/png","source":{"data":"AAAA"}}
            ]}
        ]}]}"#;
        let conversation: Conversation = serde_json::from_str(payload).unwrap();
        let messages = conversation.to_export_messages(NormalizeOptions::default());
        let block = &messages[0].blocks[0];

        assert_eq!(block.kind, "tool_result");
        let raw = block.raw.as_deref().expect("unsupported source must be kept");
        assert!(raw.contains("source"), "{raw}");
    }

    /// A fully understood block must not carry a redundant raw copy.
    #[test]
    fn does_not_attach_raw_when_everything_is_recognized() {
        let payload = r#"{"uuid":"c","chat_messages":[{"sender":"assistant","content":[
            {"type":"text","text":"all known"}
        ]}]}"#;
        let conversation: Conversation = serde_json::from_str(payload).unwrap();
        let messages = conversation.to_export_messages(NormalizeOptions::default());
        assert!(messages[0].blocks[0].raw.is_none());
    }

    /// A renamed field parses cleanly into all-optional structs, so "parsed"
    /// must not be mistaken for "understood".
    #[test]
    fn a_known_block_type_with_renamed_fields_is_preserved_not_dropped() {
        let payload = r#"{"uuid":"c","chat_messages":[{"sender":"assistant","content":[
            {"type":"text","value":"hello"}
        ]}]}"#;
        let conversation: Conversation = serde_json::from_str(payload).unwrap();
        let messages = conversation.to_export_messages(NormalizeOptions::default());
        let block = &messages[0].blocks[0];

        assert_eq!(block.kind, "unknown:text");
        let raw = block.raw.as_deref().expect("payload must be kept");
        assert!(raw.contains("hello"), "{raw}");
    }

    /// Same for a renamed attachment: an empty reference is data loss.
    #[test]
    fn a_renamed_attachment_keeps_its_payload() {
        let payload = r#"{"uuid":"c","chat_messages":[{"sender":"human","text":"hi",
            "attachments":[{"filename":"notes.txt","size":42}]}]}"#;
        let conversation: Conversation = serde_json::from_str(payload).unwrap();
        let messages = conversation.to_export_messages(NormalizeOptions::default());
        let block = &messages[0].blocks[0];

        assert_eq!(block.kind, "unknown:attachment");
        assert!(block.raw.as_deref().unwrap().contains("notes.txt"));
    }

    /// A tool-result item this build cannot render must survive as raw rather
    /// than disappear from the transcript.
    #[test]
    fn unrepresentable_tool_result_items_are_kept() {
        let payload = r#"{"uuid":"c","chat_messages":[{"sender":"assistant","content":[
            {"type":"tool_result","name":"view","content":[
              {"type":"text","text":"rendered"},
              {"type":"image","source":{"kind":"base64","data":"AAAA"}}
            ]}
        ]}]}"#;
        let conversation: Conversation = serde_json::from_str(payload).unwrap();
        let messages = conversation.to_export_messages(NormalizeOptions::default());
        let block = &messages[0].blocks[0];

        assert_eq!(block.kind, "tool_result");
        assert_eq!(block.text.as_deref(), Some("rendered"));
        let raw = block.raw.as_deref().expect("image item must be kept");
        assert!(raw.contains("base64"), "{raw}");
    }

    /// Filtering by options is a deliberate exclusion, not a parse failure, and
    /// must not resurrect blocks as raw payloads.
    #[test]
    fn excluded_blocks_are_not_resurrected_as_raw() {
        let payload = r#"{"uuid":"c","chat_messages":[{"sender":"assistant","content":[
            {"type":"thinking","thinking":"private"},
            {"type":"tool_use","name":"bash","input":{}},
            {"type":"text","text":"visible"}
        ]}]}"#;
        let conversation: Conversation = serde_json::from_str(payload).unwrap();
        let messages = conversation.to_export_messages(NormalizeOptions {
            include_thinking: false,
            include_tools: false,
        });
        let kinds: Vec<&str> = messages[0].blocks.iter().map(|b| b.kind.as_str()).collect();

        assert_eq!(kinds, ["text"]);
        let rendered = format!("{:?}", messages[0].blocks);
        assert!(!rendered.contains("private"), "thinking leaked: {rendered}");
    }

    /// A block whose shape changed upstream must cost that one block, never the
    /// whole transcript.
    #[test]
    fn a_block_with_an_unparseable_shape_does_not_reject_the_conversation() {
        // `content` is an object here, not the array this build expects.
        let payload = r#"{"uuid":"c","chat_messages":[{"sender":"assistant","content":[
            {"type":"tool_result","name":"bash","content":{"stdout":"hi"}},
            {"type":"text","text":"survived"}
        ]}]}"#;
        let conversation: Conversation =
            serde_json::from_str(payload).expect("conversation must still parse");
        let messages = conversation.to_export_messages(NormalizeOptions::default());
        let blocks = &messages[0].blocks;

        assert_eq!(blocks[0].kind, "unknown:tool_result");
        let raw = blocks[0].raw.as_deref().expect("payload should be kept");
        assert!(raw.contains("stdout"), "{raw}");
        assert_eq!(messages[0].text, "survived");
    }

    /// `raw` must carry the entire block, including fields the typed struct
    /// happens to name, not just the leftovers.
    #[test]
    fn raw_payload_covers_recognized_fields_too() {
        let payload = r#"{"uuid":"c","chat_messages":[{"sender":"assistant","content":[
            {"type":"canvas_card","name":"designer","input":{"w":10},"is_error":false}
        ]}]}"#;
        let conversation: Conversation = serde_json::from_str(payload).unwrap();
        let messages = conversation.to_export_messages(NormalizeOptions::default());
        let raw = messages[0].blocks[0].raw.as_deref().unwrap();

        for field in ["type", "canvas_card", "name", "designer", "input", "is_error"] {
            assert!(raw.contains(field), "raw lost {field}: {raw}");
        }
    }

    /// A future block type carrying no field this build knows must still survive,
    /// payload and all.
    #[test]
    fn unknown_block_without_text_keeps_its_raw_payload() {
        let payload = r#"{"uuid":"c","chat_messages":[{"sender":"assistant",
            "content":[{"type":"canvas_card","canvas_id":"abc","cells":[1,2]}]}]}"#;
        let conversation: Conversation = serde_json::from_str(payload).unwrap();
        let messages = conversation.to_export_messages(NormalizeOptions::default());
        let block = &messages[0].blocks[0];
        assert_eq!(block.kind, "unknown:canvas_card");
        assert!(block.text.is_none());
        let raw = block.raw.as_deref().expect("raw payload should be kept");
        assert!(raw.contains("canvas_id"), "{raw}");
        assert!(raw.contains("cells"), "{raw}");
    }

    #[test]
    fn falls_back_to_flat_text_when_content_is_absent() {
        let payload = r#"{"uuid":"c","chat_messages":[{"sender":"human","text":"legacy shape"}]}"#;
        let conversation: Conversation = serde_json::from_str(payload).unwrap();
        let messages = conversation.to_export_messages(NormalizeOptions::default());
        assert_eq!(messages[0].text, "legacy shape");
    }

    /// A transitional payload can carry flat prose next to non-text blocks; the
    /// prose must not be lost just because some other block exists.
    #[test]
    fn falls_back_to_flat_text_when_content_has_no_prose() {
        let payload = r#"{"uuid":"c","chat_messages":[{"sender":"human","text":"flat prose",
            "attachments":[{"file_name":"a.txt"}],
            "content":[{"type":"tool_use","name":"bash","input":{}}]}]}"#;
        let conversation: Conversation = serde_json::from_str(payload).unwrap();
        let messages = conversation.to_export_messages(NormalizeOptions::default());
        assert_eq!(messages[0].text, "flat prose");
        let kinds: Vec<&str> = messages[0].blocks.iter().map(|b| b.kind.as_str()).collect();
        assert_eq!(kinds, ["attachment", "tool_use", "text"]);
    }

    /// Excluding tools must not resurrect flat text that `content` already had.
    #[test]
    fn does_not_duplicate_prose_that_content_already_provides() {
        let payload = r#"{"uuid":"c","chat_messages":[{"sender":"assistant","text":"flat copy",
            "content":[{"type":"text","text":"block copy"}]}]}"#;
        let conversation: Conversation = serde_json::from_str(payload).unwrap();
        let messages = conversation.to_export_messages(NormalizeOptions::default());
        assert_eq!(messages[0].text, "block copy");
        assert_eq!(messages[0].blocks.len(), 1);
    }
}
