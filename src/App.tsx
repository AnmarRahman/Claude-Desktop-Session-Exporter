import { invoke } from "@tauri-apps/api/core";
import { open as openDirectoryDialog } from "@tauri-apps/plugin-dialog";
import {
  Activity,
  AlertCircle,
  Bug,
  Camera,
  CheckCircle2,
  ChevronRight,
  ExternalLink,
  FileText,
  FolderOpen,
  Loader2,
  RefreshCw,
  RotateCcw,
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
  LocalSessionDiscovery,
  LocalSessionSummary,
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

const SOURCE_LABELS: Record<"auto" | "home" | "cowork" | "code", string> = {
  auto: "the auto-detected source",
  home: "the Home chat",
  cowork: "the selected Cowork session",
  code: "the selected Claude Code session",
};

const EXPORT_DIRECTORY_STORAGE_KEY = "claude-session-exporter.output-directory";

function storedExportDirectory(): string {
  try {
    return window.localStorage.getItem(EXPORT_DIRECTORY_STORAGE_KEY)?.trim() ?? "";
  } catch {
    return "";
  }
}

function App() {
  const [detection, setDetection] = useState<ClaudeDetection>(emptyDetection);
  const [localDiscovery, setLocalDiscovery] = useState<LocalSessionDiscovery | null>(null);
  const [selectedLocalSessionId, setSelectedLocalSessionId] = useState<string | null>(null);
  const [sessionSearch, setSessionSearch] = useState("");
  const [sessionsExpanded, setSessionsExpanded] = useState(false);
  const [snapshot, setSnapshot] = useState<AccessibilitySnapshot | null>(null);
  const [selectedNode, setSelectedNode] = useState<AccessibilityNode | null>(null);
  const [expandedNodeIds, setExpandedNodeIds] = useState<Set<number>>(new Set());
  const [searchText, setSearchText] = useState("");
  const [maxDepth, setMaxDepth] = useState(0);
  const [maxElements, setMaxElements] = useState(1500);
  const [treeView, setTreeView] = useState<"control" | "raw" | "content">("control");
  const [exportSource, setExportSource] = useState<"auto" | "home" | "cowork" | "code">("auto");
  const [isChecking, setIsChecking] = useState(false);
  const [isInspecting, setIsInspecting] = useState(false);
  const [isCapturingVisible, setIsCapturingVisible] = useState(false);
  const [isExportingChat, setIsExportingChat] = useState(false);
  const [isChoosingExportDirectory, setIsChoosingExportDirectory] = useState(false);
  const [isOpeningExportDirectory, setIsOpeningExportDirectory] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [exportDirectory, setExportDirectory] = useState(storedExportDirectory);
  const [developerMode, setDeveloperMode] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [inspectorError, setInspectorError] = useState<string | null>(null);
  const [inspectorMessage, setInspectorMessage] = useState<string | null>(null);
  const [saveResult, setSaveResult] = useState<DiagnosticSaveResult | null>(null);
  const [visibleCapture, setVisibleCapture] = useState<VisibleContentCapture | null>(null);
  const [chatExport, setChatExport] = useState<ChatExportResult | null>(null);
  const [pendingConfirm, setPendingConfirm] = useState<{
    message: string;
    confirmLabel: string;
    run: () => Promise<void>;
  } | null>(null);

  const selectedLocalSession = useMemo(
    () => localDiscovery?.sessions.find((item) => item.cli_session_id === selectedLocalSessionId) ?? null,
    [localDiscovery, selectedLocalSessionId],
  );
  const selectedSessionIsDetected =
    detection.detected
    && selectedLocalSession !== null
    && localDiscovery?.active_session_id === selectedLocalSession.cli_session_id;

  const visibleLocalSessions = useMemo(() => {
    const query = sessionSearch.trim().toLocaleLowerCase();
    if (!query) return localDiscovery?.sessions ?? [];
    return (localDiscovery?.sessions ?? []).filter((item) =>
      [item.title, item.source_type, item.cwd, item.model]
        .filter(Boolean)
        .some((value) => value!.toLocaleLowerCase().includes(query)),
    );
  }, [localDiscovery, sessionSearch]);

  const detectedTitle = useMemo(() => {
    if (selectedLocalSession) return selectedLocalSession.title;
    const titledWindow = detection.windows.find((window) => window.title.trim().length > 0);
    return titledWindow?.title;
  }, [detection.windows, selectedLocalSession]);

  const inspectorOptions = useMemo<InspectorOptions>(
    () => ({ max_depth: maxDepth, max_elements: maxElements, tree_view: treeView }),
    [maxDepth, maxElements, treeView],
  );

  // `window.confirm` is unusable here: wry's WKUIDelegate does not implement
  // WKWebView's JavaScript confirm panel, so the call returns false immediately
  // without showing anything and the action silently never runs.
  function requestConfirm(message: string, confirmLabel: string, run: () => Promise<void>) {
    setError(null);
    setPendingConfirm({ message, confirmLabel, run });
  }

  async function acceptPendingConfirm() {
    const pending = pendingConfirm;
    setPendingConfirm(null);
    if (pending) await pending.run();
  }

  async function refreshClaudeStatus() {
    setIsChecking(true);
    setError(null);
    try {
      const [nextDetection, nextLocalDiscovery] = await Promise.all([
        invoke<ClaudeDetection>("detect_claude"),
        invoke<LocalSessionDiscovery>("discover_local_sessions"),
      ]);
      setDetection(nextDetection);
      setLocalDiscovery(nextLocalDiscovery);
      const currentSession = nextLocalDiscovery.sessions.find(
        (item) => item.cli_session_id === selectedLocalSessionId,
      );
      const windowMatch = matchSessionToWindow(nextDetection, nextLocalDiscovery.sessions);
      const stateMatch = nextDetection.detected
        ? nextLocalDiscovery.sessions.find(
            (item) => item.cli_session_id === nextLocalDiscovery.active_session_id,
          )
        : undefined;
      const nextSelected = windowMatch ?? stateMatch ?? currentSession ?? nextLocalDiscovery.sessions[0] ?? null;
      setSelectedLocalSessionId(nextSelected?.cli_session_id ?? null);
      if (nextSelected) setExportSource(exportSourceForSession(nextSelected));
    } catch (unknownError) {
      setError(formatInvokeError(unknownError));
      setDetection(emptyDetection);
      setLocalDiscovery(null);
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

  function saveDiagnosticSnapshot() {
    requestConfirm(
      "Diagnostic snapshots can contain text from your Claude conversation. They remain on this computer.",
      "Save Snapshot",
      runSaveDiagnosticSnapshot,
    );
  }

  async function runSaveDiagnosticSnapshot() {
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

  function captureVisibleContent() {
    requestConfirm(
      "Visible content capture saves an image of the detected Claude window on this computer.",
      "Capture Visible",
      runCaptureVisibleContent,
    );
  }

  async function runCaptureVisibleContent() {
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

  function exportChatTranscript() {
    // Naming the resolved conversation here is the only check the user gets that
    // Auto picked the right session: Claude's shell-mode keys are often missing,
    // so Auto can resolve to a stale Home chat while a local session is open.
    const localTarget = exportSource !== "auto" ? selectedLocalSession : null;
    const target = localTarget?.title;
    requestConfirm(
      `Export ${SOURCE_LABELS[exportSource]} — ${target ? `"${target}"` : "the most recent available conversation"} — as Markdown, JSON, and PDF into ${exportDirectory ? `"${exportDirectory}"` : "the default extraction directory"}.`,
      "Export Transcript",
      runExportChatTranscript,
    );
  }

  async function runExportChatTranscript() {
    setIsExportingChat(true);
    setError(null);
    setInspectorError(null);
    setChatExport(null);
    try {
      const options: ChatExportOptions = {
        source: exportSource,
        conversation_id: exportSource === "auto" ? undefined : selectedLocalSession?.cli_session_id,
        output_directory: exportDirectory || undefined,
      };
      const result = await invoke<ChatExportResult>("export_chat_transcript", { options });
      setChatExport(result);
      setInspectorMessage(`Exported ${result.message_count.toLocaleString()} chat messages to Markdown, JSON, and PDF.`);
    } catch (unknownError) {
      const message = formatInvokeError(unknownError);
      setError(message);
      setInspectorError(message);
      setInspectorMessage(null);
    } finally {
      setIsExportingChat(false);
    }
  }

  async function chooseExportDirectory() {
    setIsChoosingExportDirectory(true);
    setError(null);
    try {
      const selected = await openDirectoryDialog({
        directory: true,
        multiple: false,
        title: "Choose transcript extraction directory",
        defaultPath: exportDirectory || undefined,
      });
      if (typeof selected === "string" && selected.trim()) {
        const directory = selected.trim();
        setExportDirectory(directory);
        window.localStorage.setItem(EXPORT_DIRECTORY_STORAGE_KEY, directory);
      }
    } catch (unknownError) {
      setError(formatInvokeError(unknownError));
    } finally {
      setIsChoosingExportDirectory(false);
    }
  }

  function useDefaultExportDirectory() {
    setExportDirectory("");
    try {
      window.localStorage.removeItem(EXPORT_DIRECTORY_STORAGE_KEY);
    } catch {
      // The in-memory setting still resets when storage is unavailable.
    }
  }

  async function openExportDirectory(directory = exportDirectory) {
    setIsOpeningExportDirectory(true);
    setError(null);
    try {
      await invoke<string>("open_export_directory", {
        outputDirectory: directory || null,
      });
    } catch (unknownError) {
      setError(formatInvokeError(unknownError));
    } finally {
      setIsOpeningExportDirectory(false);
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

  function selectLocalSession(item: LocalSessionSummary) {
    setSelectedLocalSessionId(item.cli_session_id);
    setExportSource(exportSourceForSession(item));
  }

  useEffect(() => {
    void refreshClaudeStatus();
  }, []);

  return (
    <main className="app-shell">
      <section className="toolbar" aria-label="Application toolbar">
        <div className="wordmark">
          {/* Decorative: the heading beside it already names the app. */}
          <img className="logo" src="/logo.svg" alt="" width={44} height={44} />
          <div>
            <p className="eyebrow">Claude Desktop</p>
            <h1>Claude Session Exporter</h1>
          </div>
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
              <dt>{selectedSessionIsDetected ? "Detected Session" : "Selected Session"}</dt>
              <dd>{detectedTitle ?? "No conversation detected yet"}</dd>
            </div>
            <div>
              <dt>Source</dt>
              <dd>{selectedLocalSession?.source_type ?? "No source selected"}</dd>
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
              <p>Stored Cowork and Claude Code sessions remain discoverable and exportable while it is closed.</p>
            </div>
          )}

          {error && (
            <div className="error">
              <AlertCircle size={18} aria-hidden="true" />
              <span>{error}</span>
            </div>
          )}

          <div className="local-sessions">
            <div className="local-sessions-heading">
              <div>
                <p className="label">Stored sessions</p>
                <p className="muted">Home chats, Cowork sessions, and Claude Code sessions found on this device.</p>
              </div>
              <button
                className="session-toggle"
                type="button"
                aria-expanded={sessionsExpanded}
                aria-controls="stored-session-list"
                onClick={() => setSessionsExpanded((value) => !value)}
              >
                <span>{localDiscovery?.sessions.length ?? 0} extractable</span>
                <ChevronRight className={sessionsExpanded ? "expanded" : ""} size={17} aria-hidden="true" />
              </button>
            </div>
            {sessionsExpanded && (
              <div id="stored-session-list">
                {selectedSessionIsDetected ? (
                  <p className="selection-note detected-selection">
                    Matched from Claude's current chat-drawer state. You can select a different conversation below.
                  </p>
                ) : (
                  <p className="selection-note">
                    Select the conversation you want to export. If Claude has not persisted its active drawer state and macOS hides the window title, the visible conversation cannot be matched automatically.
                  </p>
                )}
                <label className="session-search">
                  <Search size={16} aria-hidden="true" />
                  <span className="sr-only">Search stored sessions</span>
                  <input
                    type="search"
                    value={sessionSearch}
                    onChange={(event) => setSessionSearch(event.target.value)}
                    placeholder="Search titles, source, folder, or model"
                  />
                </label>
                {localDiscovery && localDiscovery.sessions.length > 0 ? (
                  <div className="session-list" role="listbox" aria-label="Stored Claude sessions">
                    {visibleLocalSessions.map((item) => (
                      <button
                        className={`session-item${selectedLocalSessionId === item.cli_session_id ? " selected" : ""}`}
                        key={item.cli_session_id}
                        type="button"
                        role="option"
                        aria-selected={selectedLocalSessionId === item.cli_session_id}
                        onClick={() => selectLocalSession(item)}
                      >
                        <strong>{item.title}</strong>
                        <span>{formatSessionSubtitle(item)}</span>
                        <small>{item.cwd ?? item.transcript_path}</small>
                        <small>{formatLastActive(item.last_activity_at)}</small>
                      </button>
                    ))}
                    {visibleLocalSessions.length === 0 && <p className="muted empty-sessions">No sessions match that search.</p>}
                  </div>
                ) : (
                  <p className="muted empty-sessions">No stored transcripts were found.</p>
                )}
              </div>
            )}
          </div>

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

          <div className="export-directory">
            <div className="export-directory-copy">
              <p className="label">Extraction directory</p>
              <p className="export-directory-path" title={exportDirectory || "Default exports folder"}>
                {exportDirectory || "Default exports folder (./exports)"}
              </p>
              <p className="muted">
                {exportDirectory ? "This folder is saved for future app launches." : "Choose a folder to replace the app default."}
              </p>
            </div>
            <div className="directory-actions">
              <button
                className="secondary"
                type="button"
                disabled={isChoosingExportDirectory || isExportingChat || pendingConfirm !== null}
                onClick={chooseExportDirectory}
              >
                {isChoosingExportDirectory ? <Loader2 className="spin" size={17} aria-hidden="true" /> : <FolderOpen size={17} aria-hidden="true" />}
                <span>Change</span>
              </button>
              {exportDirectory && (
                <button
                  className="secondary"
                  type="button"
                  disabled={isExportingChat || pendingConfirm !== null}
                  onClick={useDefaultExportDirectory}
                >
                  <RotateCcw size={17} aria-hidden="true" />
                  <span>Use Default</span>
                </button>
              )}
              <button
                className="secondary"
                type="button"
                disabled={isOpeningExportDirectory}
                onClick={() => void openExportDirectory()}
              >
                {isOpeningExportDirectory ? <Loader2 className="spin" size={17} aria-hidden="true" /> : <ExternalLink size={17} aria-hidden="true" />}
                <span>Open Folder</span>
              </button>
            </div>
          </div>

          <div className="actions">
            <label className="source-select">
              <span>Source</span>
              <select
                value={exportSource}
                disabled={pendingConfirm !== null}
                onChange={(event) => {
                  const source = event.target.value as "auto" | "home" | "cowork" | "code";
                  setExportSource(source);
                  if (source !== "auto") {
                    const sourceType =
                      source === "cowork"
                        ? "Claude Desktop Cowork"
                        : source === "home"
                          ? "Claude Home Chat"
                          : "Claude Code";
                    const first = localDiscovery?.sessions.find((item) => item.source_type === sourceType);
                    setSelectedLocalSessionId(first?.cli_session_id ?? null);
                  }
                }}
              >
                <option value="auto">Auto</option>
                <option value="home">Home chat</option>
                <option value="cowork">Cowork session</option>
                <option value="code">Claude Code</option>
              </select>
            </label>
            <button
              className="primary"
              type="button"
              disabled={isExportingChat || pendingConfirm !== null || (exportSource !== "auto" && !selectedLocalSession)}
              title="Exports the selected Claude transcript to local Markdown, JSON, and PDF files."
              aria-label="Export Claude chat transcript"
              onClick={exportChatTranscript}
            >
              {isExportingChat ? <Loader2 className="spin" size={18} aria-hidden="true" /> : <FileText size={18} aria-hidden="true" />}
              <span>Export Chat Transcript</span>
            </button>
            <button
              className="secondary"
              type="button"
              disabled={!detection.detected || isCapturingVisible || pendingConfirm !== null}
              title="Captures the currently visible Claude window to a local image. Scrolling capture is not implemented yet."
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

          {chatExport && (
            <div className="export-success" role="status" aria-live="polite">
              <CheckCircle2 size={22} aria-hidden="true" />
              <div>
                <strong>Transcript extracted successfully</strong>
                <p>{chatExport.message_count.toLocaleString()} messages from “{chatExport.title}”.</p>
                <dl>
                  <div><dt>PDF</dt><dd>{chatExport.pdf_path}</dd></div>
                  <div><dt>Markdown</dt><dd>{chatExport.markdown_path}</dd></div>
                  <div><dt>JSON</dt><dd>{chatExport.json_path}</dd></div>
                </dl>
                <button
                  className="secondary export-open-folder"
                  type="button"
                  onClick={() => void openExportDirectory(chatExport.output_directory)}
                >
                  <FolderOpen size={16} aria-hidden="true" />
                  <span>Open extraction folder</span>
                </button>
                {chatExport.warnings.map((warning) => <p className="warning" key={warning}>{warning}</p>)}
              </div>
            </div>
          )}

          {pendingConfirm && (
            <div className="confirm-bar" role="alertdialog" aria-label="Confirm action">
              <p>{pendingConfirm.message}</p>
              <div className="actions compact">
                <button className="primary" type="button" onClick={acceptPendingConfirm}>
                  <span>{pendingConfirm.confirmLabel}</span>
                </button>
                <button className="secondary" type="button" onClick={() => setPendingConfirm(null)}>
                  <span>Cancel</span>
                </button>
              </div>
            </div>
          )}
        </div>

        <aside className="summary-panel" aria-label="Phase summary">
          <Metric value={localDiscovery?.sessions.length ?? 0} label="Local sessions" />
          <Metric value={localDiscovery?.diagnostics.cowork_matches ?? 0} label="Cowork matches" />
          <Metric value={localDiscovery?.diagnostics.unmatched_cowork_metadata ?? 0} label="Missing transcripts" />
        </aside>
      </section>

      {developerMode && (
        <section className="diagnostics">
          <div className="section-header">
            <div>
              <p className="eyebrow">Developer Tools</p>
              <h2>Developer Inspector</h2>
            </div>
            <div className="actions compact">
              <button className="secondary" type="button" onClick={loadAccessibilitySnapshot} disabled={isInspecting}>
                {isInspecting ? <Loader2 className="spin" size={18} aria-hidden="true" /> : <Search size={18} aria-hidden="true" />}
                <span>Refresh Tree</span>
              </button>
              <button
                className="secondary"
                type="button"
                onClick={saveDiagnosticSnapshot}
                disabled={isSaving || pendingConfirm !== null}
              >
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
          {inspectorError && (
            <div className="error diagnostic-status">
              <AlertCircle size={18} aria-hidden="true" />
              <span>{inspectorError}</span>
            </div>
          )}
          {inspectorMessage && !inspectorError && <p className="notice diagnostic-status">{inspectorMessage}</p>}

          <div className="diagnostic-content">
            <div className="diagnostic-card text-card">
              <div className="card-title">
                <Activity size={18} aria-hidden="true" />
                <h3>Cowork Filesystem Discovery</h3>
              </div>
              {localDiscovery ? (
                <>
                  <dl className="property-list discovery-properties">
                    <DiagnosticRow label="Desktop Code metadata" value={`${foundLabel(localDiscovery.diagnostics.metadata_root_found)} — ${localDiscovery.diagnostics.metadata_root}`} />
                    <DiagnosticRow label="Cowork agent metadata" value={`${foundLabel(localDiscovery.diagnostics.agent_metadata_root_found)} — ${localDiscovery.diagnostics.agent_metadata_root}`} />
                    <DiagnosticRow label="All metadata records" value={localDiscovery.diagnostics.metadata_records_discovered} />
                    <DiagnosticRow label="Cowork records" value={localDiscovery.diagnostics.agent_metadata_records_discovered} />
                    <DiagnosticRow label="Malformed metadata" value={localDiscovery.diagnostics.malformed_metadata_files} />
                    <DiagnosticRow label="Claude project root" value={`${foundLabel(localDiscovery.diagnostics.claude_projects_root_found)} — ${localDiscovery.diagnostics.claude_projects_root}`} />
                    <DiagnosticRow label="JSONL transcripts" value={localDiscovery.diagnostics.jsonl_transcripts_discovered} />
                    <DiagnosticRow label="Nested Cowork JSONLs" value={localDiscovery.diagnostics.nested_cowork_transcripts_discovered} />
                    <DiagnosticRow label="Cowork matches" value={localDiscovery.diagnostics.cowork_matches} />
                    <DiagnosticRow label="Active-session signal" value={localDiscovery.active_session_signal ?? "Not available"} />
                    <DiagnosticRow label="Unmatched metadata" value={localDiscovery.diagnostics.unmatched_cowork_metadata} />
                    <DiagnosticRow label="Duplicate metadata" value={localDiscovery.diagnostics.duplicate_metadata_records} />
                  </dl>
                  {localDiscovery.unmatched_metadata.length > 0 && (
                    <p className="warning">
                      Transcript unavailable: {localDiscovery.unmatched_metadata.map((item) => item.title).join(", ")}
                    </p>
                  )}
                  {localDiscovery.diagnostics.warnings.map((warning) => <p className="warning" key={warning}>{warning}</p>)}
                </>
              ) : (
                <p className="muted">Discovery has not completed.</p>
              )}
            </div>

            <div className="diagnostic-card text-card">
              <div className="card-title">
                <Bug size={18} aria-hidden="true" />
                <h3>Selected Device Session</h3>
              </div>
              {selectedLocalSession ? (
                <dl className="property-list discovery-properties">
                  <DiagnosticRow label="Title" value={selectedLocalSession.title} />
                  <DiagnosticRow label="Source" value={selectedLocalSession.source_type} />
                  <DiagnosticRow label="Metadata file" value={selectedLocalSession.metadata_path ?? "Not applicable"} />
                  <DiagnosticRow label="Transcript" value={selectedLocalSession.transcript_path ?? "Transcript unavailable"} />
                  <DiagnosticRow label="Desktop session ID" value={selectedLocalSession.desktop_session_id ?? "Not available"} />
                  <DiagnosticRow label={selectedLocalSession.source_type === "Claude Home Chat" ? "Conversation ID" : "CLI session ID"} value={selectedLocalSession.cli_session_id} />
                  <DiagnosticRow label="cwd" value={selectedLocalSession.cwd ?? "Not available"} />
                  <DiagnosticRow label="Model" value={selectedLocalSession.model ?? "Not available"} />
                  <DiagnosticRow label="Effort" value={selectedLocalSession.effort ?? "Not available"} />
                  <DiagnosticRow label="Created" value={formatTimestamp(selectedLocalSession.created_at)} />
                  <DiagnosticRow label="Last activity" value={formatTimestamp(selectedLocalSession.last_activity_at)} />
                  <DiagnosticRow label="Archived" value={formatBoolean(selectedLocalSession.is_archived) ?? "Not available"} />
                </dl>
              ) : (
                <p className="muted">Select a stored session above.</p>
              )}
            </div>

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
                  <p><strong>PDF:</strong> {chatExport.pdf_path}</p>
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

function DiagnosticRow({ label, value }: { label: string; value: string | number }) {
  return (
    <div>
      <dt>{label}</dt>
      <dd>{value}</dd>
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

function foundLabel(found: boolean): string {
  return found ? "FOUND" : "NOT FOUND";
}

function formatTimestamp(timestamp: number | undefined): string {
  if (!timestamp) return "Not available";
  const milliseconds = timestamp < 10_000_000_000 ? timestamp * 1000 : timestamp;
  const date = new Date(milliseconds);
  return Number.isNaN(date.getTime()) ? String(timestamp) : date.toLocaleString();
}

function formatLastActive(timestamp: number | undefined): string {
  return timestamp ? `Last active ${formatTimestamp(timestamp)}` : "Last activity unavailable";
}

function formatModel(model: string | undefined): string | undefined {
  if (!model) return undefined;
  return model
    .split("-")
    .filter(Boolean)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

function formatSessionSubtitle(session: LocalSessionSummary): string {
  const source =
    session.source_type === "Claude Desktop Cowork"
      ? "Cowork"
      : session.source_type === "Claude Home Chat"
        ? "Home chat"
        : "Claude Code";
  return [source, formatModel(session.model), session.effort].filter(Boolean).join(" • ");
}

function exportSourceForSession(session: LocalSessionSummary): "home" | "cowork" | "code" {
  if (session.source_type === "Claude Desktop Cowork") return "cowork";
  if (session.source_type === "Claude Home Chat") return "home";
  return "code";
}

function matchSessionToWindow(
  detection: ClaudeDetection,
  sessions: LocalSessionSummary[],
): LocalSessionSummary | undefined {
  const titles = detection.windows.map((window) => normalizeTitle(window.title)).filter(Boolean);
  return sessions.find((session) => {
    const title = normalizeTitle(session.title);
    return title.length > 0 && titles.some((windowTitle) => windowTitle === title || windowTitle.includes(title));
  });
}

function normalizeTitle(value: string): string {
  return value.replace(/^\*+/, "").replace(/\s+/g, " ").trim().toLocaleLowerCase();
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
