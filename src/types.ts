export type SessionType = "chat" | "cowork" | "unknown";

export interface DetectedProcess {
  pid: number;
  name: string;
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
