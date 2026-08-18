//! Export progress reporting.
//!
//! A large transcript takes minutes to lay out, so the export pipeline reports
//! how far along it is. The pipeline itself stays free of any UI dependency: it
//! calls [`report`], and whatever sink the application installed at startup
//! forwards the update. With no sink installed — in tests, for instance — every
//! report is a no-op.

use std::sync::OnceLock;

use serde::Serialize;

/// The phase an export is currently in. Only PDF layout is slow enough to need
/// per-unit progress; the other phases report themselves as they begin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProgressStage {
    ReadingTranscript,
    RenderingPdf,
    WritingFiles,
}

/// One progress update. `total` is zero when a stage has no countable units.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ExportProgress {
    pub stage: ProgressStage,
    pub completed: usize,
    pub total: usize,
}

type Sink = Box<dyn Fn(ExportProgress) + Send + Sync>;

static SINK: OnceLock<Sink> = OnceLock::new();

/// Installs the process-wide progress sink. Called once during startup; later
/// calls are ignored so a stray installation cannot silence the first.
pub fn set_sink(sink: Sink) {
    let _ = SINK.set(sink);
}

/// Reports progress to the installed sink, or does nothing when none is set.
pub fn report(stage: ProgressStage, completed: usize, total: usize) {
    if let Some(sink) = SINK.get() {
        sink(ExportProgress {
            stage,
            completed,
            total,
        });
    }
}
