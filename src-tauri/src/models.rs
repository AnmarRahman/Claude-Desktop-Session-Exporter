use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
pub struct DetectedProcess {
    pub pid: u32,
    pub name: String,
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
