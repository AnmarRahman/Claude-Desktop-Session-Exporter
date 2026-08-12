//! Read-only Claude Desktop Cowork discovery.
//!
//! Claude currently has two JSONL layouts on macOS:
//! - Desktop Code metadata under `claude-code-sessions`, joined to
//!   `~/.claude/projects` by `cliSessionId`.
//! - Cowork metadata under `local-agent-mode-sessions`, joined to the session's
//!   own nested `.claude/projects` tree by the same key.
//!
//! Both joins use the exact JSONL filename. Encoded project directories are
//! hints only. Each tree is indexed once and nested `subagents` are excluded.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::Deserialize;
use serde_json::Value;

use crate::models::{CoworkDiscoveryDiagnostics, LocalSessionDiscovery, LocalSessionSummary};

pub const SOURCE_CLAUDE_CODE: &str = "Claude Code";
pub const SOURCE_COWORK: &str = "Claude Desktop Cowork";

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct CoworkMetadata {
    pub session_id: Option<String>,
    pub cli_session_id: Option<String>,
    pub cwd: Option<String>,
    pub origin_cwd: Option<String>,
    pub created_at: Option<u64>,
    pub last_activity_at: Option<u64>,
    pub last_focused_at: Option<u64>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub is_archived: Option<bool>,
    pub title: Option<String>,
    pub title_source: Option<String>,
    pub permission_mode: Option<String>,
}

#[derive(Debug, Clone)]
struct MetadataRecord {
    metadata: CoworkMetadata,
    path: PathBuf,
    modified_unix_ms: u64,
}

#[derive(Debug, Default)]
struct TranscriptHints {
    title: Option<String>,
    cwd: Option<String>,
}

pub fn default_roots() -> Option<(PathBuf, PathBuf, PathBuf)> {
    home_directories().into_iter().next().map(|home| {
        (
            default_metadata_root(&home),
            home.join(".claude").join("projects"),
            agent_root_for_home(&home),
        )
    })
}

#[cfg(target_os = "macos")]
fn default_metadata_root(home: &Path) -> PathBuf {
    roots_for_home(home).0
}

#[cfg(windows)]
fn default_metadata_root(home: &Path) -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join("AppData").join("Roaming"))
        .join("Claude")
        .join("claude-code-sessions")
}

#[cfg(all(unix, not(target_os = "macos")))]
fn default_metadata_root(home: &Path) -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"))
        .join("Claude")
        .join("claude-code-sessions")
}

pub(crate) fn roots_for_home(home: &Path) -> (PathBuf, PathBuf) {
    (
        home.join("Library")
            .join("Application Support")
            .join("Claude")
            .join("claude-code-sessions"),
        home.join(".claude").join("projects"),
    )
}

pub(crate) fn agent_root_for_home(home: &Path) -> PathBuf {
    home.join("Library")
        .join("Application Support")
        .join("Claude")
        .join("local-agent-mode-sessions")
}

pub fn discover_default() -> LocalSessionDiscovery {
    match default_roots() {
        Some((metadata_root, projects_root, agent_root)) => {
            discover_with_agent(&metadata_root, &projects_root, &agent_root)
        }
        None => discover(Path::new(""), Path::new("")),
    }
}

/// Scan supplied roots. Kept separate from home resolution so tests never read
/// or mutate real Claude data.
pub(crate) fn discover(metadata_root: &Path, projects_root: &Path) -> LocalSessionDiscovery {
    let mut diagnostics = CoworkDiscoveryDiagnostics {
        metadata_root: metadata_root.display().to_string(),
        metadata_root_found: metadata_root.is_dir(),
        agent_metadata_root: String::new(),
        agent_metadata_root_found: false,
        agent_metadata_records_discovered: 0,
        nested_cowork_transcripts_discovered: 0,
        metadata_records_discovered: 0,
        malformed_metadata_files: 0,
        metadata_without_cli_session_id: 0,
        duplicate_metadata_records: 0,
        claude_projects_root: projects_root.display().to_string(),
        claude_projects_root_found: projects_root.is_dir(),
        jsonl_transcripts_discovered: 0,
        duplicate_transcript_ids: 0,
        cowork_matches: 0,
        unmatched_cowork_metadata: 0,
        warnings: Vec::new(),
    };

    let mut metadata_records = Vec::new();
    if metadata_root.is_dir() {
        scan_metadata_tree(metadata_root, &mut metadata_records, &mut diagnostics);
    }

    let mut transcripts: HashMap<String, Vec<PathBuf>> = HashMap::new();
    if projects_root.is_dir() {
        scan_transcript_tree(projects_root, &mut transcripts, &mut diagnostics);
    }
    diagnostics.duplicate_transcript_ids = transcripts
        .values()
        .map(|paths| paths.len().saturating_sub(1))
        .sum();

    let mut metadata_by_cli: HashMap<String, Vec<MetadataRecord>> = HashMap::new();
    for record in metadata_records {
        let Some(id) = record
            .metadata
            .cli_session_id
            .as_deref()
            .map(str::trim)
            .filter(|id| valid_join_id(id))
        else {
            diagnostics.metadata_without_cli_session_id += 1;
            continue;
        };
        metadata_by_cli
            .entry(id.to_string())
            .or_default()
            .push(record);
    }
    diagnostics.duplicate_metadata_records = metadata_by_cli
        .values()
        .map(|records| records.len().saturating_sub(1))
        .sum();

    for records in metadata_by_cli.values_mut() {
        records.sort_by(|left, right| metadata_preference(right).cmp(&metadata_preference(left)));
    }

    let mut transcript_ids: Vec<String> = transcripts.keys().cloned().collect();
    transcript_ids.sort();
    let mut sessions = Vec::with_capacity(transcript_ids.len());
    for cli_session_id in transcript_ids {
        let metadata = metadata_by_cli
            .get(&cli_session_id)
            .and_then(|records| records.first());
        let paths = &transcripts[&cli_session_id];
        let transcript_path = choose_transcript(paths, metadata.map(|record| &record.metadata));
        let summary = summary_for(
            &cli_session_id,
            metadata,
            Some(transcript_path),
            modified_unix_ms(transcript_path),
            SOURCE_CLAUDE_CODE,
        );
        sessions.push(summary);
    }

    let mut unmatched_metadata = Vec::new();
    for (cli_session_id, records) in &metadata_by_cli {
        if !transcripts.contains_key(cli_session_id) {
            // Desktop Code metadata without a shared transcript is diagnostic,
            // but is not a Cowork match.
            unmatched_metadata.push(summary_for(
                cli_session_id,
                records.first(),
                None,
                0,
                SOURCE_CLAUDE_CODE,
            ));
        }
    }
    unmatched_metadata.sort_by(|left, right| session_recency(right).cmp(&session_recency(left)));
    diagnostics.unmatched_cowork_metadata = unmatched_metadata.len();

    if diagnostics.duplicate_metadata_records > 0 {
        diagnostics.warnings.push(format!(
            "{} duplicate Cowork metadata record(s) shared a cliSessionId; the most recently active record was used.",
            diagnostics.duplicate_metadata_records
        ));
    }
    if diagnostics.duplicate_transcript_ids > 0 {
        diagnostics.warnings.push(format!(
            "{} duplicate JSONL path(s) shared a filename; cwd matching and stable path order selected one transcript per session.",
            diagnostics.duplicate_transcript_ids
        ));
    }

    sessions.sort_by(|left, right| session_recency(right).cmp(&session_recency(left)));
    LocalSessionDiscovery {
        sessions,
        unmatched_metadata,
        active_session_id: None,
        active_session_signal: None,
        diagnostics,
    }
}

/// Add true Cowork sessions from `local-agent-mode-sessions`. Unlike the shared
/// Claude Code tree, unmatched nested JSONLs are internal continuations or
/// subagents and must never be surfaced as independent Claude Code sessions.
fn discover_with_agent(
    metadata_root: &Path,
    projects_root: &Path,
    agent_root: &Path,
) -> LocalSessionDiscovery {
    let mut result = discover(metadata_root, projects_root);
    result.diagnostics.agent_metadata_root = agent_root.display().to_string();
    result.diagnostics.agent_metadata_root_found = agent_root.is_dir();
    if !agent_root.is_dir() {
        return result;
    }

    let records_before = result.diagnostics.metadata_records_discovered;
    let mut records = Vec::new();
    scan_metadata_tree(agent_root, &mut records, &mut result.diagnostics);
    result.diagnostics.agent_metadata_records_discovered = result
        .diagnostics
        .metadata_records_discovered
        .saturating_sub(records_before);

    let mut by_cli: HashMap<String, Vec<MetadataRecord>> = HashMap::new();
    for record in records {
        let Some(id) = record
            .metadata
            .cli_session_id
            .as_deref()
            .map(str::trim)
            .filter(|id| valid_join_id(id))
        else {
            result.diagnostics.metadata_without_cli_session_id += 1;
            continue;
        };
        by_cli.entry(id.to_string()).or_default().push(record);
    }
    for records in by_cli.values_mut() {
        records.sort_by(|left, right| metadata_preference(right).cmp(&metadata_preference(left)));
    }
    result.diagnostics.duplicate_metadata_records += by_cli
        .values()
        .map(|records| records.len().saturating_sub(1))
        .sum::<usize>();

    let mut nested_transcripts = HashMap::new();
    let transcript_count_before = result.diagnostics.jsonl_transcripts_discovered;
    scan_transcript_tree(agent_root, &mut nested_transcripts, &mut result.diagnostics);
    result.diagnostics.nested_cowork_transcripts_discovered = result
        .diagnostics
        .jsonl_transcripts_discovered
        .saturating_sub(transcript_count_before);

    let mut existing: std::collections::HashSet<String> = result
        .sessions
        .iter()
        .map(|session| session.cli_session_id.clone())
        .collect();
    for (cli_session_id, records) in by_cli {
        let record = records.first();
        let transcript_path = nested_transcripts
            .get(&cli_session_id)
            .map(|paths| choose_transcript(paths, record.map(|record| &record.metadata)));
        let summary = summary_for(
            &cli_session_id,
            record,
            transcript_path,
            transcript_path.map(modified_unix_ms).unwrap_or(0),
            SOURCE_COWORK,
        );
        if transcript_path.is_some() {
            result.diagnostics.cowork_matches += 1;
            if !existing.insert(cli_session_id.clone()) {
                // An exact Cowork metadata/transcript match is stronger than a
                // same-named shared JSONL, so it owns the canonical ID.
                result
                    .sessions
                    .retain(|session| session.cli_session_id != cli_session_id);
            }
            result.sessions.push(summary);
        } else {
            result.unmatched_metadata.push(summary);
        }
    }

    result.diagnostics.unmatched_cowork_metadata = result
        .unmatched_metadata
        .iter()
        .filter(|session| session.source_type == SOURCE_COWORK)
        .count();
    result
        .sessions
        .sort_by(|left, right| session_recency(right).cmp(&session_recency(left)));
    result
}

fn scan_metadata_tree(
    directory: &Path,
    records: &mut Vec<MetadataRecord>,
    diagnostics: &mut CoworkDiscoveryDiagnostics,
) {
    let Ok(entries) = fs::read_dir(directory) else {
        diagnostics.warnings.push(format!(
            "Could not read Cowork metadata directory {}.",
            directory.display()
        ));
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            scan_metadata_tree(&path, records, diagnostics);
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        if !is_local_metadata_file(&path) {
            continue;
        }
        let Ok(contents) = fs::read_to_string(&path) else {
            diagnostics.malformed_metadata_files += 1;
            continue;
        };
        match serde_json::from_str::<CoworkMetadata>(&contents) {
            Ok(metadata) => {
                diagnostics.metadata_records_discovered += 1;
                records.push(MetadataRecord {
                    metadata,
                    modified_unix_ms: modified_unix_ms(&path),
                    path,
                });
            }
            Err(_) => diagnostics.malformed_metadata_files += 1,
        }
    }
}

fn scan_transcript_tree(
    directory: &Path,
    transcripts: &mut HashMap<String, Vec<PathBuf>>,
    diagnostics: &mut CoworkDiscoveryDiagnostics,
) {
    let Ok(entries) = fs::read_dir(directory) else {
        diagnostics.warnings.push(format!(
            "Could not read Claude projects directory {}.",
            directory.display()
        ));
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            if path.file_name().and_then(|name| name.to_str()) == Some("subagents") {
                continue;
            }
            scan_transcript_tree(&path, transcripts, diagnostics);
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        if path.extension().and_then(|extension| extension.to_str()) != Some("jsonl") {
            continue;
        }
        let Some(id) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if !valid_join_id(id) {
            continue;
        }
        diagnostics.jsonl_transcripts_discovered += 1;
        transcripts.entry(id.to_string()).or_default().push(path);
    }
}

fn is_local_metadata_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("local_") && name.ends_with(".json"))
}

fn valid_join_id(id: &str) -> bool {
    !id.is_empty()
        && id != "."
        && id != ".."
        && !id.contains('/')
        && !id.contains('\\')
        && !id.contains('\0')
}

fn metadata_preference(record: &MetadataRecord) -> (u64, u64, u64, String) {
    (
        record.metadata.last_activity_at.unwrap_or(0),
        record.metadata.last_focused_at.unwrap_or(0),
        record.metadata.created_at.unwrap_or(0),
        record.path.display().to_string(),
    )
}

fn choose_transcript<'a>(paths: &'a [PathBuf], metadata: Option<&CoworkMetadata>) -> &'a Path {
    let expected_project = metadata
        .and_then(|metadata| metadata.cwd.as_deref().or(metadata.origin_cwd.as_deref()))
        .map(claude_project_directory_name);
    let mut ordered: Vec<&PathBuf> = paths.iter().collect();
    ordered.sort();
    if let Some(expected) = expected_project {
        if let Some(path) = ordered.iter().find(|path| {
            path.parent()
                .and_then(Path::file_name)
                .and_then(|name| name.to_str())
                == Some(expected.as_str())
        }) {
            return path;
        }
    }
    ordered[0]
}

fn summary_for(
    cli_session_id: &str,
    record: Option<&MetadataRecord>,
    transcript_path: Option<&Path>,
    transcript_modified_at: u64,
    source_type: &str,
) -> LocalSessionSummary {
    let metadata = record.map(|record| &record.metadata);
    let metadata_title = metadata.and_then(|metadata| metadata.title.as_deref().and_then(nonempty));
    let metadata_cwd =
        metadata.and_then(|metadata| metadata.cwd.clone().or_else(|| metadata.origin_cwd.clone()));
    // Cowork metadata normally provides both values. Avoid opening a potentially
    // large transcript merely to rediscover them.
    let hints = if metadata_title.is_some() && metadata_cwd.is_some() {
        TranscriptHints::default()
    } else {
        transcript_path
            .map(read_transcript_hints)
            .unwrap_or_default()
    };
    let cwd = metadata.and_then(|_| metadata_cwd).or(hints.cwd);
    let title = metadata_title
        .map(str::to_string)
        .or(hints.title)
        .or_else(|| cwd.as_deref().and_then(cwd_basename))
        .unwrap_or_else(|| cli_session_id.to_string());

    LocalSessionSummary {
        cli_session_id: cli_session_id.to_string(),
        desktop_session_id: metadata.and_then(|metadata| metadata.session_id.clone()),
        title,
        source_type: source_type.to_string(),
        transcript_available: transcript_path.is_some(),
        transcript_path: transcript_path.map(|path| path.display().to_string()),
        metadata_path: record.map(|record| record.path.display().to_string()),
        metadata_modified_at: record
            .map(|record| record.modified_unix_ms)
            .filter(|timestamp| *timestamp > 0),
        cwd,
        origin_cwd: metadata.and_then(|metadata| metadata.origin_cwd.clone()),
        model: metadata.and_then(|metadata| metadata.model.clone()),
        effort: metadata.and_then(|metadata| metadata.effort.clone()),
        created_at: metadata.and_then(|metadata| metadata.created_at),
        last_activity_at: metadata
            .and_then(|metadata| metadata.last_activity_at)
            .or(Some(transcript_modified_at).filter(|timestamp| *timestamp > 0)),
        last_focused_at: metadata.and_then(|metadata| metadata.last_focused_at),
        is_archived: metadata.and_then(|metadata| metadata.is_archived),
        title_source: metadata.and_then(|metadata| metadata.title_source.clone()),
        permission_mode: metadata.and_then(|metadata| metadata.permission_mode.clone()),
    }
}

fn read_transcript_hints(path: &Path) -> TranscriptHints {
    let Ok(file) = File::open(path) else {
        return TranscriptHints::default();
    };
    let mut hints = TranscriptHints::default();
    let mut first_user_title = None;
    // Discovery should stay an index operation, not become a full parse of
    // every potentially huge session. Titles and cwd normally appear near the
    // start; export performs the authoritative full parse only for one session.
    const MAX_HINT_BYTES: u64 = 2 * 1024 * 1024;
    for line in BufReader::new(file)
        .take(MAX_HINT_BYTES)
        .lines()
        .map_while(Result::ok)
    {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if hints.cwd.is_none() {
            hints.cwd = value
                .get("cwd")
                .and_then(Value::as_str)
                .and_then(nonempty)
                .map(str::to_string);
        }
        match value.get("type").and_then(Value::as_str) {
            Some("custom-title") => {
                hints.title = value
                    .get("customTitle")
                    .and_then(Value::as_str)
                    .and_then(nonempty)
                    .map(str::to_string);
            }
            Some("ai-title") if hints.title.is_none() => {
                hints.title = value
                    .get("aiTitle")
                    .and_then(Value::as_str)
                    .and_then(nonempty)
                    .map(str::to_string);
            }
            Some("user") if first_user_title.is_none() => {
                let is_human = value
                    .get("origin")
                    .and_then(|origin| origin.get("kind"))
                    .and_then(Value::as_str)
                    == Some("human");
                if is_human {
                    first_user_title = value
                        .get("message")
                        .and_then(|message| message.get("content"))
                        .and_then(Value::as_str)
                        .and_then(nonempty)
                        .map(|text| {
                            text.lines()
                                .next()
                                .unwrap_or(text)
                                .chars()
                                .take(80)
                                .collect()
                        });
                }
            }
            _ => {}
        }
    }
    if hints.title.is_none() {
        hints.title = first_user_title;
    }
    hints
}

fn nonempty(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

fn cwd_basename(cwd: &str) -> Option<String> {
    Path::new(cwd)
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(nonempty)
        .map(str::to_string)
}

fn claude_project_directory_name(cwd: &str) -> String {
    cwd.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' || character == '-' {
                character
            } else {
                '-'
            }
        })
        .collect()
}

fn modified_unix_ms(path: &Path) -> u64 {
    path.metadata()
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn session_recency(session: &LocalSessionSummary) -> (u64, u64, u64, &str) {
    (
        session
            .metadata_modified_at
            .unwrap_or(0)
            .max(session.last_focused_at.unwrap_or(0))
            .max(session.last_activity_at.unwrap_or(0)),
        session.last_activity_at.unwrap_or(0),
        session.created_at.unwrap_or(0),
        session.cli_session_id.as_str(),
    )
}

fn home_directories() -> Vec<PathBuf> {
    let ordered = if cfg!(windows) {
        ["USERPROFILE", "HOME"]
    } else {
        ["HOME", "USERPROFILE"]
    };
    let mut homes: Vec<PathBuf> = ordered
        .iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .collect();
    homes.dedup();
    homes
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{discover, discover_with_agent, roots_for_home, SOURCE_CLAUDE_CODE, SOURCE_COWORK};

    /// Read-only integration check for the installed Claude profile. It prints
    /// counts and roots only, never titles or conversation content.
    #[test]
    #[ignore = "requires a local macOS Claude profile"]
    fn discovers_the_real_filesystem_layout() {
        let result = super::discover_default();
        let diagnostics = result.diagnostics;
        eprintln!(
            "metadata root: {} ({})",
            diagnostics.metadata_root, diagnostics.metadata_root_found
        );
        eprintln!(
            "projects root: {} ({})",
            diagnostics.claude_projects_root, diagnostics.claude_projects_root_found
        );
        eprintln!(
            "metadata={} jsonl={} matches={} unmatched={}",
            diagnostics.metadata_records_discovered,
            diagnostics.jsonl_transcripts_discovered,
            diagnostics.cowork_matches,
            diagnostics.unmatched_cowork_metadata
        );
        assert!(diagnostics.metadata_root_found);
        assert!(diagnostics.claude_projects_root_found);
        assert!(diagnostics.jsonl_transcripts_discovered > 0);
    }

    struct Fixture {
        root: PathBuf,
        metadata: PathBuf,
        projects: PathBuf,
        agent: PathBuf,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir().join(format!("cse-cowork-{name}-{unique}"));
            let metadata = root.join("Library/Application Support/Claude/claude-code-sessions");
            let projects = root.join(".claude/projects");
            let agent = root.join("Library/Application Support/Claude/local-agent-mode-sessions");
            fs::create_dir_all(&metadata).unwrap();
            fs::create_dir_all(&projects).unwrap();
            fs::create_dir_all(&agent).unwrap();
            Self {
                root,
                metadata,
                projects,
                agent,
            }
        }

        fn metadata(&self, folder: &str, name: &str, json: &str) {
            let directory = self.metadata.join(folder);
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join(name), json).unwrap();
        }

        fn transcript(&self, project: &str, id: &str, lines: &str) {
            let directory = self.projects.join(project);
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join(format!("{id}.jsonl")), lines).unwrap();
        }

        fn agent_session(&self, local_id: &str, cli_id: &str, title: &str) {
            let container = self.agent.join("account/container");
            fs::create_dir_all(&container).unwrap();
            fs::write(
                container.join(format!("{local_id}.json")),
                format!(r#"{{"sessionId":"{local_id}","cliSessionId":"{cli_id}","title":"{title}","cwd":"/sessions/test"}}"#),
            )
            .unwrap();
            let projects = container
                .join(local_id)
                .join(".claude/projects/-sessions-test");
            fs::create_dir_all(&projects).unwrap();
            fs::write(projects.join(format!("{cli_id}.jsonl")), "").unwrap();
            let subagents = projects.join(cli_id).join("subagents");
            fs::create_dir_all(&subagents).unwrap();
            fs::write(subagents.join("agent-internal.jsonl"), "").unwrap();
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn parses_valid_metadata_and_correlates_exact_filename() {
        let fixture = Fixture::new("valid");
        fixture.metadata(
            "account/container",
            "local_desktop.json",
            r#"{
          "sessionId":"local_desktop","cliSessionId":"cli-1","cwd":"/Users/A/My Project",
          "originCwd":"/Users/A/My Project","createdAt":1,"lastActivityAt":3,"lastFocusedAt":4,
          "model":"claude-opus-5","effort":"high","isArchived":true,"title":"Desktop title",
          "titleSource":"auto","permissionMode":"acceptEdits","unknownFutureField":42
        }"#,
        );
        fixture.transcript("-Users-A-My-Project", "cli-1", "");
        let result = discover(&fixture.metadata, &fixture.projects);
        assert_eq!(result.sessions.len(), 1);
        let session = &result.sessions[0];
        assert_eq!(session.source_type, SOURCE_CLAUDE_CODE);
        assert_eq!(session.title, "Desktop title");
        assert_eq!(session.desktop_session_id.as_deref(), Some("local_desktop"));
        assert_eq!(session.effort.as_deref(), Some("high"));
        assert_eq!(session.is_archived, Some(true));
        assert_eq!(result.diagnostics.cowork_matches, 0);
    }

    #[test]
    fn skips_malformed_metadata_and_accepts_missing_optional_fields() {
        let fixture = Fixture::new("malformed");
        fixture.metadata("a", "local_bad.json", "{not-json");
        fixture.metadata("a", "local_minimal.json", r#"{"cliSessionId":"cli-2"}"#);
        fixture.metadata("a", "scheduled-tasks.json", "{not-json");
        fixture.metadata("a", "deleted_x", "{not-json");
        fixture.transcript("project", "cli-2", "");
        let result = discover(&fixture.metadata, &fixture.projects);
        assert_eq!(result.diagnostics.malformed_metadata_files, 1);
        assert_eq!(result.diagnostics.metadata_records_discovered, 1);
        assert_eq!(result.sessions[0].source_type, SOURCE_CLAUDE_CODE);
    }

    #[test]
    fn shared_project_jsonls_are_claude_code_and_deduplicated() {
        let fixture = Fixture::new("classify");
        fixture.metadata("a", "local_cowork.json", r#"{"cliSessionId":"cowork-id"}"#);
        fixture.transcript("Project One", "cowork-id", "");
        fixture.transcript("Project Two", "code-id", "");
        let result = discover(&fixture.metadata, &fixture.projects);
        assert_eq!(result.sessions.len(), 2);
        assert_eq!(
            result
                .sessions
                .iter()
                .filter(|s| s.cli_session_id == "cowork-id")
                .count(),
            1
        );
        assert_eq!(
            result
                .sessions
                .iter()
                .find(|s| s.cli_session_id == "cowork-id")
                .unwrap()
                .source_type,
            SOURCE_CLAUDE_CODE
        );
        assert_eq!(
            result
                .sessions
                .iter()
                .find(|s| s.cli_session_id == "code-id")
                .unwrap()
                .source_type,
            SOURCE_CLAUDE_CODE
        );
    }

    #[test]
    fn discovers_nested_cowork_transcript_and_excludes_subagents() {
        let fixture = Fixture::new("nested-cowork");
        fixture.agent_session(
            "local_328cb477-b116-42f0-8b52-fb19cee34c9d",
            "5121dabb-5e27-4f85-ad51-812569de176a",
            "MSAccess vs SQL data vs Odoo",
        );
        let result = discover_with_agent(&fixture.metadata, &fixture.projects, &fixture.agent);
        assert_eq!(result.sessions.len(), 1);
        assert_eq!(result.sessions[0].source_type, SOURCE_COWORK);
        assert_eq!(result.sessions[0].title, "MSAccess vs SQL data vs Odoo");
        assert!(result.sessions[0]
            .transcript_path
            .as_deref()
            .unwrap()
            .ends_with("5121dabb-5e27-4f85-ad51-812569de176a.jsonl"));
        assert_eq!(result.diagnostics.cowork_matches, 1);
        assert_eq!(result.diagnostics.nested_cowork_transcripts_discovered, 1);
        assert!(!result
            .sessions
            .iter()
            .any(|session| session.cli_session_id == "agent-internal"));
    }

    #[test]
    fn reports_missing_transcript_without_adding_extractable_session() {
        let fixture = Fixture::new("missing");
        fixture.metadata(
            "a",
            "local_missing.json",
            r#"{"cliSessionId":"missing-id","title":"Missing"}"#,
        );
        let result = discover(&fixture.metadata, &fixture.projects);
        assert!(result.sessions.is_empty());
        assert_eq!(result.unmatched_metadata.len(), 1);
        assert!(!result.unmatched_metadata[0].transcript_available);
        assert_eq!(result.diagnostics.unmatched_cowork_metadata, 1);
    }

    #[test]
    fn handles_multiple_sessions_and_duplicate_metadata_deterministically() {
        let fixture = Fixture::new("duplicates");
        fixture.metadata(
            "a",
            "local_old.json",
            r#"{"cliSessionId":"one","title":"Old","lastActivityAt":10}"#,
        );
        fixture.metadata(
            "b",
            "local_new.json",
            r#"{"cliSessionId":"one","title":"New","lastActivityAt":20}"#,
        );
        fixture.metadata(
            "b",
            "local_two.json",
            r#"{"cliSessionId":"two","title":"Two"}"#,
        );
        fixture.transcript("p", "one", "");
        fixture.transcript("p", "two", "");
        let result = discover(&fixture.metadata, &fixture.projects);
        assert_eq!(result.sessions.len(), 2);
        assert_eq!(
            result
                .sessions
                .iter()
                .find(|s| s.cli_session_id == "one")
                .unwrap()
                .title,
            "New"
        );
        assert_eq!(result.diagnostics.duplicate_metadata_records, 1);
    }

    #[test]
    fn resolves_duplicate_transcript_ids_with_cwd_hint() {
        let fixture = Fixture::new("ambiguous-transcript");
        fixture.metadata(
            "a",
            "local_one.json",
            r#"{"cliSessionId":"same-id","cwd":"/Users/A/Right Project"}"#,
        );
        fixture.transcript("-Users-A-Other-Project", "same-id", "");
        fixture.transcript("-Users-A-Right-Project", "same-id", "");
        let result = discover(&fixture.metadata, &fixture.projects);
        assert_eq!(result.sessions.len(), 1);
        assert!(result.sessions[0]
            .transcript_path
            .as_deref()
            .unwrap()
            .contains("Right-Project"));
        assert_eq!(result.diagnostics.duplicate_transcript_ids, 1);
    }

    #[test]
    fn title_falls_back_to_transcript_then_cwd_then_id_and_handles_spaces() {
        let fixture = Fixture::new("titles with spaces");
        fixture.metadata(
            "container with spaces",
            "local_ai.json",
            r#"{"cliSessionId":"ai","cwd":"/tmp/Project With Spaces"}"#,
        );
        fixture.metadata(
            "x",
            "local_cwd.json",
            r#"{"cliSessionId":"cwd","cwd":"/tmp/Cwd Name"}"#,
        );
        fixture.metadata("x", "local_id.json", r#"{"cliSessionId":"only-id"}"#);
        fixture.transcript(
            "Project With Spaces",
            "ai",
            r#"{"type":"ai-title","aiTitle":"Transcript title"}"#,
        );
        fixture.transcript("Cwd Name", "cwd", "");
        fixture.transcript("x", "only-id", "");
        let result = discover(&fixture.metadata, &fixture.projects);
        let title = |id: &str| {
            result
                .sessions
                .iter()
                .find(|s| s.cli_session_id == id)
                .unwrap()
                .title
                .as_str()
        };
        assert_eq!(title("ai"), "Transcript title");
        assert_eq!(title("cwd"), "Cwd Name");
        assert_eq!(title("only-id"), "only-id");
    }

    #[test]
    fn resolves_macos_roots_from_home_without_hardcoded_username() {
        let (metadata, projects) = roots_for_home(Path::new("/Users/Person With Spaces"));
        assert_eq!(
            metadata,
            Path::new(
                "/Users/Person With Spaces/Library/Application Support/Claude/claude-code-sessions"
            )
        );
        assert_eq!(
            projects,
            Path::new("/Users/Person With Spaces/.claude/projects")
        );
    }
}
