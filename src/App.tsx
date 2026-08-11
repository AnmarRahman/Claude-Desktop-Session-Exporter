import { invoke } from "@tauri-apps/api/core";
import {
  Activity,
  AlertCircle,
  Bug,
  Camera,
  CheckCircle2,
  ChevronRight,
  FileText,
  Loader2,
  RefreshCw,
  Save,
  Search,
  Settings,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { nodeMatchesSearch } from "./lib/inspector";
import type {
  AccessibilityNode,
  AccessibilitySnapshot,
  ChatExportOptions,
  ChatExportResult,
  ClaudeDetection,
  DiagnosticSaveResult,
  InspectorOptions,
  SessionMetadata,
  VisibleContentCapture,
} from "./types";

const captureOptions = [
  "User messages",
  "Claude responses",
  "Images",
  "Code blocks",
  "Tables",
  "Tool / Cowork activity",
  "File cards",
];

const emptyDetection: ClaudeDetection = {
  detected: false,
  platform: "unknown",
  processes: [],
  windows: [],
  message: "Claude Desktop has not been checked yet.",
};

const emptySession: SessionMetadata = {
  session_type: "unknown",
};

function App() {
  const [detection, setDetection] = useState<ClaudeDetection>(emptyDetection);
  const [session, setSession] = useState<SessionMetadata>(emptySession);
  const [snapshot, setSnapshot] = useState<AccessibilitySnapshot | null>(null);
  const [selectedNode, setSelectedNode] = useState<AccessibilityNode | null>(null);
  const [expandedNodeIds, setExpandedNodeIds] = useState<Set<number>>(new Set());
  const [searchText, setSearchText] = useState("");
  const [maxDepth, setMaxDepth] = useState(0);
  const [maxElements, setMaxElements] = useState(1500);
  const [treeView, setTreeView] = useState<"control" | "raw" | "content">("control");
  const [exportSource, setExportSource] = useState<"auto" | "home" | "code">("auto");
  const [isChecking, setIsChecking] = useState(false);
  const [isInspecting, setIsInspecting] = useState(false);
  const [isCapturingVisible, setIsCapturingVisible] = useState(false);
  const [isExportingChat, setIsExportingChat] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [developerMode, setDeveloperMode] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [inspectorError, setInspectorError] = useState<string | null>(null);
  const [inspectorMessage, setInspectorMessage] = useState<string | null>(null);
  const [saveResult, setSaveResult] = useState<DiagnosticSaveResult | null>(null);
  const [visibleCapture, setVisibleCapture] = useState<VisibleContentCapture | null>(null);
  const [chatExport, setChatExport] = useState<ChatExportResult | null>(null);

  const detectedTitle = useMemo(() => {
    if (session.title) return session.title;
    const titledWindow = detection.windows.find((window) => window.title.trim().length > 0);
    return titledWindow?.title;
  }, [detection.windows, session.title]);

  const inspectorOptions = useMemo<InspectorOptions>(
    () => ({ max_depth: maxDepth, max_elements: maxElements, tree_view: treeView }),
    [maxDepth, maxElements, treeView],
  );

  async function refreshClaudeStatus() {
    setIsChecking(true);
    setError(null);
    try {
      const [nextDetection, nextSession] = await Promise.all([
        invoke<ClaudeDetection>("detect_claude"),
        invoke<SessionMetadata>("get_active_session"),
      ]);
      setDetection(nextDetection);
      setSession(nextSession);
    } catch (unknownError) {
      setError(formatInvokeError(unknownError));
      setDetection(emptyDetection);
      setSession(emptySession);
    } finally {
      setIsChecking(false);
    }
  }

  async function loadAccessibilitySnapshot() {
    setIsInspecting(true);
    setError(null);
    setInspectorError(null);
    setInspectorMessage("Requesting a UI Automation snapshot from the detected Claude window...");
    setSaveResult(null);
    try {
      const nextSnapshot = await invoke<AccessibilitySnapshot>("get_accessibility_snapshot", {
        options: inspectorOptions,
      });
      setSnapshot(nextSnapshot);
      setSelectedNode(nextSnapshot.nodes[0] ?? null);
      setExpandedNodeIds(new Set(nextSnapshot.nodes[0] ? [nextSnapshot.nodes[0].id] : []));
      setInspectorMessage(
        nextSnapshot.nodes.length > 0
          ? `Loaded ${nextSnapshot.element_count.toLocaleString()} UIA elements.`
          : "Snapshot completed, but no UIA tree nodes were returned.",
      );
    } catch (unknownError) {
      const message = formatInvokeError(unknownError);
      setError(message);
      setInspectorError(message);
      setInspectorMessage(null);
    } finally {
      setIsInspecting(false);
    }
  }

  async function saveDiagnosticSnapshot() {
    const confirmed = window.confirm(
      "Diagnostic snapshots can contain text from your Claude conversation.\nThey remain on this computer.",
    );
    if (!confirmed) return;

    setIsSaving(true);
    setError(null);
    setInspectorError(null);
    try {
      const result = await invoke<DiagnosticSaveResult>("save_diagnostic_snapshot", {
        options: inspectorOptions,
      });
      setSaveResult(result);
      setInspectorMessage(`Saved diagnostic snapshot to ${result.path}`);
    } catch (unknownError) {
      const message = formatInvokeError(unknownError);
      setError(message);
      setInspectorError(message);
    } finally {
      setIsSaving(false);
    }
  }

  async function captureVisibleContent() {
    const confirmed = window.confirm(
      "Visible content capture saves an image of the detected Claude window on this computer.",
    );
    if (!confirmed) return;

    setIsCapturingVisible(true);
    setError(null);
    setInspectorError(null);
    setInspectorMessage("Capturing the currently visible Claude window...");
    try {
      const result = await invoke<VisibleContentCapture>("capture_visible_content");
      setVisibleCapture(result);
      setInspectorMessage(`Captured visible Claude content to ${result.image_path}`);
    } catch (unknownError) {
      const message = formatInvokeError(unknownError);
      setError(message);
      setInspectorError(message);
      setInspectorMessage(null);
    } finally {
      setIsCapturingVisible(false);
    }
  }

  async function exportChatTranscript() {
    const confirmed = window.confirm(
      "Chat export saves the detected Claude session transcript as Markdown and JSON on this computer.",
    );
    if (!confirmed) return;

    setIsExportingChat(true);
    setError(null);
    setInspectorError(null);
    try {
      const options: ChatExportOptions = { source: exportSource };
      const result = await invoke<ChatExportResult>("export_chat_transcript", { options });
      setChatExport(result);
      setInspectorMessage(`Exported ${result.message_count.toLocaleString()} chat messages to ${result.markdown_path}`);
    } catch (unknownError) {
      const message = formatInvokeError(unknownError);
      setError(message);
      setInspectorError(message);
      setInspectorMessage(null);
    } finally {
      setIsExportingChat(false);
    }
  }

  function toggleNode(nodeId: number) {
    setExpandedNodeIds((current) => {
      const next = new Set(current);
      if (next.has(nodeId)) {
        next.delete(nodeId);
      } else {
        next.add(nodeId);
      }
      return next;
    });
  }

  useEffect(() => {
    void refreshClaudeStatus();
  }, []);

  return (
    <main className="app-shell">
      <section className="toolbar" aria-label="Application toolbar">
        <div>
          <p className="eyebrow">Claude Desktop</p>
          <h1>Claude Session Exporter</h1>
        </div>
        <button className="icon-button" type="button" onClick={() => setDeveloperMode((value) => !value)}>
          <Settings size={18} aria-hidden="true" />
          <span>{developerMode ? "Hide Developer Mode" : "Developer Mode"}</span>
        </button>
      </section>

      <section className="workspace">
        <div className="status-panel">
          <div className="status-row">
            {detection.detected ? (
              <CheckCircle2 className="status-icon detected" size={22} aria-hidden="true" />
            ) : (
              <AlertCircle className="status-icon missing" size={22} aria-hidden="true" />
            )}
            <div>
              <p className="label">Status</p>
              <p className="strong">{detection.message}</p>
            </div>
          </div>

          <dl className="facts">
            <div>
              <dt>Current Session</dt>
              <dd>{detectedTitle ?? "No conversation detected yet"}</dd>
            </div>
            <div>
              <dt>Session Type</dt>
              <dd>{session.session_type === "unknown" ? "Claude session" : session.session_type}</dd>
            </div>
            <div>
              <dt>Platform Adapter</dt>
              <dd>{detection.platform}</dd>
            </div>
          </dl>

          {detection.windows[0] && (
            <div className="window-details">
              <p><strong>Process:</strong> {detection.windows[0].process_name ?? detection.processes[0]?.name ?? "Unknown"}</p>
              <p><strong>Window:</strong> {detection.windows[0].title || "Untitled"}</p>
              <p><strong>PID:</strong> {detection.windows[0].process_id ?? "Unknown"}</p>
              <p><strong>HWND:</strong> {detection.windows[0].hwnd ?? "Unknown"}</p>
              <p><strong>Class:</strong> {detection.windows[0].class_name ?? "Unknown"}</p>
              <p><strong>Bounds:</strong> {formatBounds(detection.windows[0].bounds)}</p>
            </div>
          )}

          {!detection.detected && (
            <div className="notice">
              <p>Claude Desktop was not detected.</p>
              <p>Open Claude and navigate to the conversation you want to export.</p>
            </div>
          )}

          {error && (
            <div className="error">
              <AlertCircle size={18} aria-hidden="true" />
              <span>{error}</span>
            </div>
          )}

          <div className="options">
            <p className="label">Capture options</p>
            <div className="option-grid">
              {captureOptions.map((option) => (
                <label key={option} className="checkbox-row">
                  <input type="checkbox" defaultChecked disabled />
                  <span>{option}</span>
                </label>
              ))}
            </div>
          </div>

          <div className="actions">
            <label className="source-select">
              <span>Source</span>
              <select value={exportSource} onChange={(event) => setExportSource(event.target.value as "auto" | "home" | "code")}>
                <option value="auto">Auto</option>
                <option value="home">Home / Cowork</option>
                <option value="code">Claude Code</option>
              </select>
            </label>
            <button
              className="primary"
              type="button"
              disabled={!detection.detected || isExportingChat}
              title="Exports the detected Claude transcript to local Markdown and JSON files."
              aria-label="Export Claude chat transcript"
              onClick={exportChatTranscript}
            >
              {isExportingChat ? <Loader2 className="spin" size={18} aria-hidden="true" /> : <FileText size={18} aria-hidden="true" />}
              <span>Export Chat Transcript</span>
            </button>
            <button
              className="secondary"
              type="button"
              disabled={!detection.detected || isCapturingVisible}
              title="Captures the currently visible Claude window to a local image. Scrolling and PDF export are not implemented yet."
              aria-label="Capture visible Claude content"
              onClick={captureVisibleContent}
            >
              {isCapturingVisible ? <Loader2 className="spin" size={18} aria-hidden="true" /> : <Camera size={18} aria-hidden="true" />}
              <span>Capture Visible</span>
            </button>
            <button className="secondary" type="button" onClick={refreshClaudeStatus} disabled={isChecking}>
              {isChecking ? <Loader2 className="spin" size={18} aria-hidden="true" /> : <RefreshCw size={18} aria-hidden="true" />}
              <span>Retry</span>
            </button>
          </div>
        </div>

        <aside className="summary-panel" aria-label="Phase summary">
          <Metric value={detection.processes.length} label="Claude processes" />
          <Metric value={detection.windows.length} label="Claude windows" />
          <Metric value={snapshot?.element_count ?? 0} label="UIA elements" />
        </aside>
      </section>

      {developerMode && (
        <section className="diagnostics">
          <div className="section-header">
            <div>
              <p className="eyebrow">Developer Tools</p>
              <h2>Accessibility Inspector</h2>
            </div>
            <div className="actions compact">
              <button className="secondary" type="button" onClick={loadAccessibilitySnapshot} disabled={isInspecting}>
                {isInspecting ? <Loader2 className="spin" size={18} aria-hidden="true" /> : <Search size={18} aria-hidden="true" />}
                <span>Refresh Tree</span>
              </button>
              <button className="secondary" type="button" onClick={saveDiagnosticSnapshot} disabled={isSaving}>
                {isSaving ? <Loader2 className="spin" size={18} aria-hidden="true" /> : <Save size={18} aria-hidden="true" />}
                <span>Save Snapshot</span>
              </button>
            </div>
          </div>

          <div className="inspector-controls">
            <label>
              <span>Search tree</span>
              <input value={searchText} onChange={(event) => setSearchText(event.target.value)} placeholder="Text, role, class, pattern" />
            </label>
            <label>
              <span>Max depth</span>
              <input type="number" min={0} max={30} value={maxDepth} onChange={(event) => setMaxDepth(Number(event.target.value))} />
            </label>
            <label>
              <span>Max elements</span>
              <input type="number" min={1} max={25000} step={250} value={maxElements} onChange={(event) => setMaxElements(Number(event.target.value))} />
            </label>
            <label>
              <span>Tree view</span>
              <select value={treeView} onChange={(event) => setTreeView(event.target.value as "control" | "raw" | "content")}>
                <option value="control">Control</option>
                <option value="raw">Raw</option>
                <option value="content">Content</option>
              </select>
            </label>
          </div>

          {saveResult && <p className="notice">{saveResult.warning} Saved to {saveResult.path}</p>}
          {visibleCapture && (
            <div className="notice">
              <p>Captured visible Claude content to {visibleCapture.image_path}</p>
              {visibleCapture.warnings.map((warning) => (
                <p key={warning}>{warning}</p>
              ))}
            </div>
          )}
          {chatExport && (
            <div className="notice">
              <p>Exported {chatExport.message_count.toLocaleString()} chat messages from {chatExport.title}.</p>
              <p>Source: {chatExport.source_type}</p>
              <p>Markdown: {chatExport.markdown_path}</p>
              <p>JSON: {chatExport.json_path}</p>
              {chatExport.warnings.map((warning) => (
                <p key={warning}>{warning}</p>
              ))}
            </div>
          )}
          {inspectorError && (
            <div className="error diagnostic-status">
              <AlertCircle size={18} aria-hidden="true" />
              <span>{inspectorError}</span>
            </div>
          )}
          {inspectorMessage && !inspectorError && <p className="notice diagnostic-status">{inspectorMessage}</p>}

          <div className="diagnostic-content">
            <div className="diagnostic-card">
              <div className="card-title">
                <Activity size={18} aria-hidden="true" />
                <h3>Detected Windows</h3>
              </div>
              {detection.windows.length > 0 ? (
                <ul className="plain-list">
                  {detection.windows.map((window, index) => (
                    <li key={`${window.title}-${window.process_id ?? index}`}>
                      <span>{window.title || "Untitled window"}</span>
                      <small>{[window.process_name, window.process_id ? `PID ${window.process_id}` : undefined, window.hwnd, window.class_name].filter(Boolean).join(" | ")}</small>
                    </li>
                  ))}
                </ul>
              ) : (
                <p className="muted">No Claude windows have been found.</p>
              )}
            </div>

            <div className="diagnostic-card tree-card">
              <div className="card-title">
                <Bug size={18} aria-hidden="true" />
                <h3>UI Automation Tree</h3>
              </div>
              {isInspecting ? (
                <p className="muted">Reading UI Automation tree from the detected Claude window...</p>
              ) : snapshot ? (
                <>
                  <p className="muted">
                    {snapshot.element_count.toLocaleString()} elements discovered
                    {snapshot.truncated ? " (truncated)" : ""}
                  </p>
                  {snapshot.warnings.map((warning) => (
                    <p className="warning" key={warning}>{warning}</p>
                  ))}
                  <div className="tree">
                    {snapshot.nodes.map((node) => (
                      <TreeNode
                        key={node.id}
                        node={node}
                        expandedNodeIds={expandedNodeIds}
                        searchText={searchText}
                        selectedNodeId={selectedNode?.id}
                        onToggle={toggleNode}
                        onSelect={setSelectedNode}
                      />
                    ))}
                  </div>
                </>
              ) : (
                <p className="muted">Refresh the tree after opening Claude Desktop.</p>
              )}
            </div>

            <div className="diagnostic-card">
              <h3>Selected Element</h3>
              {selectedNode ? <PropertyInspector node={selectedNode} /> : <p className="muted">Select a tree element.</p>}
            </div>

            <div className="diagnostic-card">
              <h3>Conversation Candidates</h3>
              {snapshot?.conversation_candidates.length ? (
                <ul className="candidate-list">
                  {snapshot.conversation_candidates.map((candidate) => (
                    <li key={candidate.node_id}>
                      <strong>Candidate #{candidate.id}</strong>
                      <span>{candidate.control_type ?? "element"} | {Math.round(candidate.confidence * 100)}%</span>
                      <small>
                        {candidate.bounds ? `${candidate.bounds.width}x${candidate.bounds.height}` : "no bounds"} | {candidate.descendant_count} descendants | {candidate.text_node_count} text nodes
                      </small>
                      <p>{candidate.reasons.join(", ")}</p>
                    </li>
                  ))}
                </ul>
              ) : (
                <p className="muted">No likely conversation containers have been identified yet.</p>
              )}
            </div>

            <div className="diagnostic-card text-card">
              <h3>Chat Transcript Export</h3>
              {chatExport ? (
                <div className="capture-result">
                  <p><strong>Title:</strong> {chatExport.title}</p>
                  <p><strong>Source Type:</strong> {chatExport.source_type}</p>
                  <p><strong>Messages:</strong> {chatExport.message_count.toLocaleString()}</p>
                  <p><strong>Markdown:</strong> {chatExport.markdown_path}</p>
                  <p><strong>JSON:</strong> {chatExport.json_path}</p>
                  <p><strong>Source:</strong> {chatExport.source_path}</p>
                </div>
              ) : (
                <p className="muted">Use Export Chat Transcript to save the selected Claude source.</p>
              )}
            </div>

            <div className="diagnostic-card text-card">
              <h3>Visible Content Capture</h3>
              {visibleCapture ? (
                <div className="capture-result">
                  <p><strong>Image:</strong> {visibleCapture.image_path}</p>
                  {visibleCapture.text ? <p>{visibleCapture.text}</p> : <p className="muted">No OCR text was extracted yet.</p>}
                </div>
              ) : (
                <p className="muted">Use Capture Visible Content to save the currently visible Claude window.</p>
              )}
            </div>

            <div className="diagnostic-card text-card">
              <h3>Visible Text Blocks</h3>
              {snapshot?.visible_text_blocks.length ? (
                <ol className="text-blocks">
                  {snapshot.visible_text_blocks.map((block) => (
                    <li key={block.node_id}>
                      <span>{block.author} | {block.control_type ?? "element"}</span>
                      <p>{block.text}</p>
                      {block.reason ? <small>{block.reason}</small> : null}
                    </li>
                  ))}
                </ol>
              ) : (
                <p className="muted">No visible text blocks have been extracted yet.</p>
              )}
            </div>
          </div>
        </section>
      )}
    </main>
  );
}

function Metric({ value, label }: { value: number; label: string }) {
  return (
    <div className="metric">
      <span>{value}</span>
      <p>{label}</p>
    </div>
  );
}

function TreeNode({
  node,
  expandedNodeIds,
  searchText,
  selectedNodeId,
  onToggle,
  onSelect,
}: {
  node: AccessibilityNode;
  expandedNodeIds: Set<number>;
  searchText: string;
  selectedNodeId?: number;
  onToggle: (nodeId: number) => void;
  onSelect: (node: AccessibilityNode) => void;
}) {
  const isExpanded = expandedNodeIds.has(node.id);
  const isMatch = nodeMatchesSearch(node, searchText);
  const hasChildren = node.children.length > 0;

  return (
    <div className="tree-node">
      <button
        className={`tree-row ${selectedNodeId === node.id ? "selected" : ""} ${isMatch ? "match" : ""}`}
        type="button"
        style={{ paddingLeft: `${node.depth * 14 + 6}px` }}
        onClick={() => onSelect(node)}
      >
        <span className="twisty" onClick={(event) => {
          event.stopPropagation();
          if (hasChildren) onToggle(node.id);
        }}>
          {hasChildren ? <ChevronRight className={isExpanded ? "expanded" : ""} size={14} aria-hidden="true" /> : null}
        </span>
        <strong>{node.control_type ?? node.localized_control_type ?? "Element"}</strong>
        <span>{node.name ?? node.value ?? node.class_name ?? "unnamed"}</span>
        <small>{node.children.length}</small>
      </button>
      {isExpanded &&
        node.children.map((child) => (
          <TreeNode
            key={child.id}
            node={child}
            expandedNodeIds={expandedNodeIds}
            searchText={searchText}
            selectedNodeId={selectedNodeId}
            onToggle={onToggle}
            onSelect={onSelect}
          />
        ))}
    </div>
  );
}

function PropertyInspector({ node }: { node: AccessibilityNode }) {
  const rows = [
    ["ID", node.id],
    ["Control type", node.control_type],
    ["Localized type", node.localized_control_type],
    ["Name", node.name],
    ["Automation ID", node.automation_id],
    ["Class", node.class_name],
    ["Framework", node.framework_id],
    ["Value", node.value],
    ["Bounds", node.bounds ? `${node.bounds.x},${node.bounds.y} ${node.bounds.width}x${node.bounds.height}` : undefined],
    ["Enabled", formatBoolean(node.enabled)],
    ["Focused", formatBoolean(node.has_keyboard_focus)],
    ["Offscreen", formatBoolean(node.offscreen)],
    ["Patterns", node.supported_patterns.join(", ")],
  ];

  return (
    <dl className="property-list">
      {rows.map(([label, value]) => (
        <div key={String(label)}>
          <dt>{label}</dt>
          <dd>{value || "None"}</dd>
        </div>
      ))}
    </dl>
  );
}

function formatBoolean(value: boolean | undefined): string | undefined {
  if (value === undefined) return undefined;
  return value ? "Yes" : "No";
}

function formatBounds(bounds: { x: number; y: number; width: number; height: number } | undefined): string {
  if (!bounds) return "Unknown";
  return `${bounds.x},${bounds.y} ${bounds.width}x${bounds.height}`;
}

function formatInvokeError(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  return "The native command failed.";
}

export default App;
