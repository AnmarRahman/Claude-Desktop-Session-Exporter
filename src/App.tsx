import { invoke } from "@tauri-apps/api/core";
import {
  Activity,
  AlertCircle,
  Bug,
  CheckCircle2,
  ChevronRight,
  FileDown,
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
  ClaudeDetection,
  DiagnosticSaveResult,
  InspectorOptions,
  SessionMetadata,
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
  const [maxDepth, setMaxDepth] = useState(12);
  const [maxElements, setMaxElements] = useState(5000);
  const [treeView, setTreeView] = useState<"control" | "raw" | "content">("control");
  const [isChecking, setIsChecking] = useState(false);
  const [isInspecting, setIsInspecting] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [developerMode, setDeveloperMode] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saveResult, setSaveResult] = useState<DiagnosticSaveResult | null>(null);

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
    setSaveResult(null);
    try {
      const nextSnapshot = await invoke<AccessibilitySnapshot>("get_accessibility_snapshot", {
        options: inspectorOptions,
      });
      setSnapshot(nextSnapshot);
      setSelectedNode(nextSnapshot.nodes[0] ?? null);
      setExpandedNodeIds(new Set(nextSnapshot.nodes[0] ? [nextSnapshot.nodes[0].id] : []));
    } catch (unknownError) {
      setError(formatInvokeError(unknownError));
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
    try {
      setSaveResult(
        await invoke<DiagnosticSaveResult>("save_diagnostic_snapshot", {
          options: inspectorOptions,
        }),
      );
    } catch (unknownError) {
      setError(formatInvokeError(unknownError));
    } finally {
      setIsSaving(false);
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
            <button className="primary" type="button" disabled>
              <FileDown size={18} aria-hidden="true" />
              <span>Capture Current Session</span>
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
              <input type="number" min={1} max={30} value={maxDepth} onChange={(event) => setMaxDepth(Number(event.target.value))} />
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
              {snapshot ? (
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
