//! Resolves and opens the user-selected transcript export directory.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::capture::CaptureError;
use crate::models::ChatExportOptions;

/// File types requested for one transcript export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportFormats {
    pub markdown: bool,
    pub json: bool,
    pub pdf: bool,
}

impl ExportFormats {
    pub fn from_options(options: &ChatExportOptions) -> Result<Self, CaptureError> {
        let formats = Self {
            markdown: options.export_markdown.unwrap_or(true),
            json: options.export_json.unwrap_or(true),
            pdf: options.export_pdf.unwrap_or(true),
        };
        if !formats.markdown && !formats.json && !formats.pdf {
            return Err(CaptureError::Diagnostic(
                "Select at least one export file type: Markdown, JSON, or PDF.".to_string(),
            ));
        }
        Ok(formats)
    }
}

/// Paths sharing one collision-free basename. Unselected formats have no path.
#[derive(Debug)]
pub struct ExportPaths {
    pub markdown: Option<PathBuf>,
    pub json: Option<PathBuf>,
    pub pdf: Option<PathBuf>,
}

impl ExportPaths {
    pub fn remove_files(&self) {
        for path in self.iter() {
            let _ = fs::remove_file(path);
        }
    }

    fn iter(&self) -> impl Iterator<Item = &PathBuf> {
        self.markdown
            .iter()
            .chain(self.json.iter())
            .chain(self.pdf.iter())
    }
}

/// Claims a basename so concurrent or same-second exports never overwrite one another.
pub fn reserve_export_paths(
    exports_dir: &Path,
    title: &str,
    timestamp: u64,
    formats: ExportFormats,
) -> Result<ExportPaths, CaptureError> {
    const MAX_ATTEMPTS: u32 = 100;

    for attempt in 0..MAX_ATTEMPTS {
        let basename = match attempt {
            0 => format!("{title}-{timestamp}"),
            n => format!("{title}-{timestamp}-{}", n + 1),
        };
        let paths = ExportPaths {
            markdown: formats
                .markdown
                .then(|| exports_dir.join(format!("{basename}.md"))),
            json: formats
                .json
                .then(|| exports_dir.join(format!("{basename}.json"))),
            pdf: formats
                .pdf
                .then(|| exports_dir.join(format!("{basename}.pdf"))),
        };
        let reservation = paths
            .iter()
            .next()
            .expect("ExportFormats guarantees at least one selected format")
            .clone();

        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&reservation)
        {
            Ok(_) if paths.iter().skip(1).any(|path| path.exists()) => {
                let _ = fs::remove_file(&reservation);
                continue;
            }
            Ok(_) => return Ok(paths),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(CaptureError::Diagnostic(error.to_string())),
        }
    }

    Err(CaptureError::Diagnostic(format!(
        "Could not find an unused export filename for {title} after {MAX_ATTEMPTS} attempts."
    )))
}

/// Folder created for transcripts when the user has picked no directory.
const DEFAULT_EXPORT_FOLDER: &str = "Claude Session Exporter";

/// Where transcripts land when the user has selected no extraction directory.
///
/// An installed app is launched by the OS rather than from a shell, so the
/// working directory is arbitrary — `/` on macOS — and a relative `./exports`
/// would resolve somewhere unwritable. Anchor the default to the user's home
/// instead, preferring `Documents` when it exists.
pub fn default_export_directory() -> PathBuf {
    match home_directory() {
        Some(home) => {
            let documents = home.join("Documents");
            if documents.is_dir() {
                documents.join(DEFAULT_EXPORT_FOLDER)
            } else {
                home.join(DEFAULT_EXPORT_FOLDER)
            }
        }
        None => std::env::temp_dir().join(DEFAULT_EXPORT_FOLDER),
    }
}

fn home_directory() -> Option<PathBuf> {
    #[cfg(windows)]
    let home = std::env::var_os("USERPROFILE");
    #[cfg(not(windows))]
    let home = std::env::var_os("HOME");

    home.map(PathBuf::from).filter(|path| path.is_absolute())
}

pub fn prepare_export_directory(requested: Option<&str>) -> Result<PathBuf, CaptureError> {
    let directory = match requested.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => {
            let path = PathBuf::from(value);
            if !path.is_absolute() {
                return Err(CaptureError::Diagnostic(
                    "The extraction directory must be an absolute path.".to_string(),
                ));
            }
            path
        }
        None => default_export_directory(),
    };

    if directory.exists() && !directory.is_dir() {
        return Err(CaptureError::Diagnostic(format!(
            "The extraction destination is not a directory: {}",
            directory.display()
        )));
    }

    fs::create_dir_all(&directory).map_err(|error| {
        CaptureError::Diagnostic(format!(
            "Could not create extraction directory {}: {error}",
            directory.display()
        ))
    })?;

    Ok(directory)
}

pub fn open_in_file_manager(directory: &Path) -> Result<(), CaptureError> {
    let canonical = directory.canonicalize().map_err(|error| {
        CaptureError::Diagnostic(format!(
            "Could not resolve extraction directory {}: {error}",
            directory.display()
        ))
    })?;

    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");
    #[cfg(windows)]
    let mut command = Command::new("explorer.exe");
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = Command::new("xdg-open");
    #[cfg(not(any(unix, windows)))]
    return Err(CaptureError::Diagnostic(
        "Opening the extraction directory is not supported on this platform.".to_string(),
    ));

    command.arg(&canonical).spawn().map_err(|error| {
        CaptureError::Diagnostic(format!(
            "Could not open extraction directory {}: {error}",
            canonical.display()
        ))
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        default_export_directory, prepare_export_directory, reserve_export_paths, ExportFormats,
    };
    use crate::models::ChatExportOptions;

    /// A bundled app inherits an arbitrary working directory, so the default
    /// must never be relative to it.
    #[test]
    fn defaults_to_an_absolute_directory_outside_the_working_directory() {
        let directory = default_export_directory();
        assert!(directory.is_absolute());
        assert!(!directory.starts_with(std::env::current_dir().unwrap()));
    }

    #[test]
    fn creates_an_absolute_custom_directory() {
        let directory = std::env::temp_dir().join(format!(
            "claude-session-exporter-output-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);

        let resolved = prepare_export_directory(directory.to_str()).unwrap();
        assert_eq!(resolved, directory);
        assert!(resolved.is_dir());

        std::fs::remove_dir_all(resolved).unwrap();
    }

    #[test]
    fn rejects_relative_custom_directories() {
        let error = prepare_export_directory(Some("relative/exports")).unwrap_err();
        assert!(error.to_string().contains("absolute path"));
    }

    #[test]
    fn defaults_to_all_export_formats() {
        let formats = ExportFormats::from_options(&ChatExportOptions::default()).unwrap();
        assert_eq!(
            formats,
            ExportFormats {
                markdown: true,
                json: true,
                pdf: true,
            }
        );
    }

    #[test]
    fn rejects_an_export_with_no_file_types() {
        let error = ExportFormats::from_options(&ChatExportOptions {
            export_markdown: Some(false),
            export_json: Some(false),
            export_pdf: Some(false),
            ..ChatExportOptions::default()
        })
        .unwrap_err();
        assert!(error.to_string().contains("at least one"));
    }

    #[test]
    fn reserves_only_selected_formats() {
        let directory = std::env::temp_dir().join(format!(
            "claude-session-exporter-reservation-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();

        let paths = reserve_export_paths(
            &directory,
            "Chat",
            1786455779,
            ExportFormats {
                markdown: false,
                json: true,
                pdf: true,
            },
        )
        .unwrap();
        assert!(paths.markdown.is_none());
        assert!(paths.json.as_ref().unwrap().exists());
        assert!(!paths.pdf.as_ref().unwrap().exists());

        paths.remove_files();
        std::fs::remove_dir_all(directory).unwrap();
    }
}
