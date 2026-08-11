use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
pub struct DetectedProcess {
    pub pid: u32,
    pub name: String,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DetectedWindow {
    pub title: String,
    pub process_id: Option<u32>,
    pub process_name: Option<String>,
    pub hwnd: Option<String>,
    pub class_name: Option<String>,
    pub visible: bool,
    pub bounds: Option<Bounds>,
    pub detection_signals: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClaudeDetection {
    pub detected: bool,
    pub platform: String,
    pub processes: Vec<DetectedProcess>,
    pub windows: Vec<DetectedWindow>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionMetadata {
    pub title: Option<String>,
    pub session_type: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AccessibilityNode {
    pub id: usize,
    pub depth: usize,
    pub control_type: Option<String>,
    pub localized_control_type: Option<String>,
    pub name: Option<String>,
    pub automation_id: Option<String>,
    pub class_name: Option<String>,
    pub framework_id: Option<String>,
    pub value: Option<String>,
    pub bounds: Option<Bounds>,
    pub enabled: Option<bool>,
    pub has_keyboard_focus: Option<bool>,
    pub offscreen: Option<bool>,
    pub supported_patterns: Vec<String>,
    pub child_count: usize,
    pub children: Vec<AccessibilityNode>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Bounds {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, Serialize)]
pub struct AccessibilitySnapshot {
    pub platform: String,
    pub root_name: Option<String>,
    pub max_depth: usize,
    pub max_elements: usize,
    pub element_count: usize,
    pub truncated: bool,
    pub nodes: Vec<AccessibilityNode>,
    pub conversation_candidates: Vec<ConversationCandidate>,
    pub visible_text_blocks: Vec<VisibleTextBlock>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InspectorOptions {
    pub max_depth: Option<usize>,
    pub max_elements: Option<usize>,
    pub tree_view: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConversationCandidate {
    pub id: usize,
    pub node_id: usize,
    pub control_type: Option<String>,
    pub name: Option<String>,
    pub bounds: Option<Bounds>,
    pub descendant_count: usize,
    pub text_node_count: usize,
    pub text_character_count: usize,
    pub button_count: usize,
    pub editable_count: usize,
    pub scrollable: bool,
    pub confidence: f32,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct VisibleTextBlock {
    pub node_id: usize,
    pub text: String,
    pub control_type: Option<String>,
    pub bounds: Option<Bounds>,
    pub author: String,
    pub confidence: f32,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticSaveResult {
    pub path: String,
    pub warning: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct VisibleContentCapture {
    pub image_path: String,
    pub text: Option<String>,
    pub warnings: Vec<String>,
}

/// A citation or file a block points at, when the content itself is not text.
#[derive(Debug, Clone, Serialize)]
pub struct ChatExportReference {
    pub kind: String,
    pub label: Option<String>,
    pub url: Option<String>,
}

/// One piece of a message: prose, thinking, or a unit of tool activity.
///
/// Extraction and rendering stay separate, so an unrecognized block is kept with
/// a descriptive `kind` rather than dropped.
#[derive(Debug, Clone, Serialize)]
pub struct ChatExportBlock {
    pub kind: String,
    pub text: Option<String>,
    pub tool_name: Option<String>,
    pub tool_input: Option<String>,
    pub is_error: Option<bool>,
    pub references: Vec<ChatExportReference>,
    /// Verbatim payload of a block this build does not understand, so an
    /// upstream change loses formatting rather than content.
    pub raw: Option<String>,
}

impl ChatExportBlock {
    pub fn empty(kind: &str) -> Self {
        Self {
            kind: kind.to_string(),
            text: None,
            tool_name: None,
            tool_input: None,
            is_error: None,
            references: Vec::new(),
            raw: None,
        }
    }

    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: Some(text.into()),
            ..Self::empty("text")
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatExportMessage {
    pub role: String,
    /// The message's prose, excluding thinking and tool activity.
    pub text: String,
    pub timestamp: Option<String>,
    pub blocks: Vec<ChatExportBlock>,
}

impl ChatExportMessage {
    /// A message carrying nothing but prose, for sources without block structure.
    // Used by the Claude Code reader, which is Windows-only today.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn plain(role: impl Into<String>, text: String, timestamp: Option<String>) -> Self {
        Self {
            role: role.into(),
            blocks: vec![ChatExportBlock::text(text.clone())],
            text,
            timestamp,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatExportResult {
    pub title: String,
    pub session_id: String,
    pub source_type: String,
    pub source_path: String,
    pub markdown_path: String,
    pub json_path: String,
    pub message_count: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ChatExportOptions {
    /// "auto" | "home" | "code".
    pub source: Option<String>,
    /// Export this Home/Cowork conversation instead of the most recent one.
    pub conversation_id: Option<String>,
    /// Include Claude's thinking blocks. Off unless requested.
    pub include_thinking: Option<bool>,
    /// Include tool and Cowork activity. On unless disabled.
    pub include_tools: Option<bool>,
}
