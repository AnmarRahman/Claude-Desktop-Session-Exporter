/** Mirrors `session_type` in the Rust `SessionMetadata`. Cowork and Claude Code
 *  are separate products backed by separate stores, so they are separate types. */
export type SessionType = "chat" | "cowork" | "code" | "unknown";

export interface DetectedProcess {
  pid: number;
  name: string;
  path?: string;
}

export interface DetectedWindow {
  title: string;
  process_id?: number;
  process_name?: string;
  hwnd?: string;
  class_name?: string;
  visible: boolean;
  bounds?: Bounds;
  detection_signals: string[];
}

export interface ClaudeDetection {
  detected: boolean;
  platform: string;
  processes: DetectedProcess[];
  windows: DetectedWindow[];
  message: string;
}

export interface SessionMetadata {
  title?: string;
  session_type: SessionType;
}

export type LocalSessionSource = "Claude Home Chat" | "Claude Code" | "Claude Desktop Cowork" | string;

export interface LocalSessionSummary {
  cli_session_id: string;
  desktop_session_id?: string;
  title: string;
  source_type: LocalSessionSource;
  transcript_available: boolean;
  transcript_path?: string;
  metadata_path?: string;
  metadata_modified_at?: number;
  cwd?: string;
  origin_cwd?: string;
  model?: string;
  effort?: string;
  created_at?: number;
  last_activity_at?: number;
  last_focused_at?: number;
  is_archived?: boolean;
  title_source?: string;
  permission_mode?: string;
}

export interface CoworkDiscoveryDiagnostics {
  metadata_root: string;
  metadata_root_found: boolean;
  agent_metadata_root: string;
  agent_metadata_root_found: boolean;
  agent_metadata_records_discovered: number;
  nested_cowork_transcripts_discovered: number;
  metadata_records_discovered: number;
  malformed_metadata_files: number;
  metadata_without_cli_session_id: number;
  duplicate_metadata_records: number;
  claude_projects_root: string;
  claude_projects_root_found: boolean;
  jsonl_transcripts_discovered: number;
  duplicate_transcript_ids: number;
  cowork_matches: number;
  unmatched_cowork_metadata: number;
  warnings: string[];
}

export interface LocalSessionDiscovery {
  sessions: LocalSessionSummary[];
  unmatched_metadata: LocalSessionSummary[];
  active_session_id?: string;
  active_session_signal?: string;
  diagnostics: CoworkDiscoveryDiagnostics;
}

export interface AccessibilityNode {
  id: number;
  depth: number;
  control_type?: string;
  localized_control_type?: string;
  name?: string;
  automation_id?: string;
  class_name?: string;
  framework_id?: string;
  value?: string;
  bounds?: Bounds;
  enabled?: boolean;
  has_keyboard_focus?: boolean;
  offscreen?: boolean;
  supported_patterns: string[];
  child_count: number;
  children: AccessibilityNode[];
}

export interface AccessibilitySnapshot {
  platform: string;
  root_name?: string;
  max_depth: number;
  max_elements: number;
  element_count: number;
  truncated: boolean;
  nodes: AccessibilityNode[];
  conversation_candidates: ConversationCandidate[];
  visible_text_blocks: VisibleTextBlock[];
  warnings: string[];
}

export interface Bounds {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface InspectorOptions {
  max_depth?: number;
  max_elements?: number;
  tree_view?: "raw" | "control" | "content" | string;
}

export interface ConversationCandidate {
  id: number;
  node_id: number;
  control_type?: string;
  name?: string;
  bounds?: Bounds;
  descendant_count: number;
  text_node_count: number;
  text_character_count: number;
  button_count: number;
  editable_count: number;
  scrollable: boolean;
  confidence: number;
  reasons: string[];
}

export interface VisibleTextBlock {
  node_id: number;
  text: string;
  control_type?: string;
  bounds?: Bounds;
  author: "user" | "assistant" | "unknown" | string;
  confidence: number;
  reason?: string;
}

export interface DiagnosticSaveResult {
  path: string;
  warning: string;
}

export interface VisibleContentCapture {
  image_path: string;
  text?: string;
  warnings: string[];
}

export interface ChatExportReference {
  kind: string;
  label?: string;
  url?: string;
}

export interface ChatExportBlock {
  kind: "text" | "thinking" | "tool_use" | "tool_result" | "attachment" | "file" | string;
  text?: string;
  tool_name?: string;
  tool_input?: string;
  is_error?: boolean;
  references: ChatExportReference[];
  /** Verbatim payload of a block this build does not understand. */
  raw?: string;
}

export interface ChatExportMessage {
  role: string;
  /** The message's prose, excluding thinking and tool activity. */
  text: string;
  timestamp?: string;
  blocks: ChatExportBlock[];
}

export interface ChatExportResult {
  title: string;
  session_id: string;
  source_type: string;
  source_path: string;
  markdown_path: string;
  json_path: string;
  pdf_path: string;
  output_directory: string;
  message_count: number;
  warnings: string[];
}

export interface ChatExportOptions {
  source?: "auto" | "home" | "cowork" | "code" | string;
  /** Export this Home conversation or local JSONL session instead of the newest one. */
  conversation_id?: string;
  /** Include Claude's thinking blocks. Off unless requested. */
  include_thinking?: boolean;
  /** Include tool and Cowork activity. On unless disabled. */
  include_tools?: boolean;
  /** Absolute directory selected by the user. Uses the app's default exports folder when absent. */
  output_directory?: string;
}
