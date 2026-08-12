//! Resolves and opens the user-selected transcript export directory.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::capture::CaptureError;

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
        None => std::env::current_dir()
            .map_err(|error| CaptureError::Diagnostic(error.to_string()))?
            .join("exports"),
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
    use super::prepare_export_directory;

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
}
