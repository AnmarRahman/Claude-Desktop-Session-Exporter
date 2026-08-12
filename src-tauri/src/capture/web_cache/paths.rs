use std::path::PathBuf;

/// Roots of Claude Desktop's Chromium profile, newest-install-agnostic.
///
/// Claude Desktop is an Electron app, so the profile layout under the root is
/// expected to match across platforms: `Cache/Cache_Data` for the HTTP disk
/// cache and `Local Storage/leveldb` for renderer local storage. Only the macOS
/// root has been confirmed against a real installation; the Windows paths are
/// carried over from the earlier Windows-only reader and are unverified for the
/// cache directory specifically.
pub fn claude_data_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    #[cfg(target_os = "macos")]
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(
            PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("Claude"),
        );
    }

    #[cfg(windows)]
    {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            dirs.push(PathBuf::from(appdata).join("Claude"));
        }

        // Store/MSIX installs redirect the roaming profile under the package's
        // LocalCache directory.
        if let Some(local_appdata) = std::env::var_os("LOCALAPPDATA") {
            let packages = PathBuf::from(local_appdata).join("Packages");
            if let Ok(entries) = std::fs::read_dir(&packages) {
                for entry in entries.filter_map(Result::ok) {
                    if entry
                        .file_name()
                        .to_string_lossy()
                        .to_lowercase()
                        .starts_with("claude_")
                    {
                        dirs.push(
                            entry
                                .path()
                                .join("LocalCache")
                                .join("Roaming")
                                .join("Claude"),
                        );
                    }
                }
            }
        }
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(config) = std::env::var_os("XDG_CONFIG_HOME") {
            dirs.push(PathBuf::from(config).join("Claude"));
        } else if let Some(home) = std::env::var_os("HOME") {
            dirs.push(PathBuf::from(home).join(".config").join("Claude"));
        }
    }

    dirs.retain(|dir| dir.is_dir());
    dirs
}

/// Directories holding Chromium simple-cache entry files.
///
/// Modern Chromium keeps entries in `Cache/Cache_Data`; older profiles put them
/// directly in `Cache`. Partitioned storage adds one cache per partition.
pub fn http_cache_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    for root in claude_data_dirs() {
        push_cache_dir(&mut dirs, root.join("Cache"));

        if let Ok(partitions) = std::fs::read_dir(root.join("Partitions")) {
            for partition in partitions.filter_map(Result::ok) {
                push_cache_dir(&mut dirs, partition.path().join("Cache"));
            }
        }
    }

    dirs
}

fn push_cache_dir(dirs: &mut Vec<PathBuf>, cache_root: PathBuf) {
    let cache_data = cache_root.join("Cache_Data");
    if cache_data.is_dir() {
        dirs.push(cache_data);
    } else if cache_root.is_dir() {
        dirs.push(cache_root);
    }
}

/// Directories holding renderer local storage (used for Claude's shell mode).
pub fn local_storage_dirs() -> Vec<PathBuf> {
    claude_data_dirs()
        .into_iter()
        .map(|root| root.join("Local Storage").join("leveldb"))
        .filter(|dir| dir.is_dir())
        .collect()
}

/// Directories holding per-window renderer session state. Claude's chat drawer
/// stores a snapshot timestamp for each conversation here.
pub fn session_storage_dirs() -> Vec<PathBuf> {
    claude_data_dirs()
        .into_iter()
        .map(|root| root.join("Session Storage"))
        .filter(|dir| dir.is_dir())
        .collect()
}

/// Which on-disk format a Chromium cache directory uses.
///
/// Chromium has two HTTP cache backends. This reader implements the simple
/// cache, which is what Claude Desktop uses on macOS. The blockfile backend has
/// an entirely different layout, and is reported rather than misread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheBackend {
    Simple,
    Blockfile,
    Unrecognized,
}

pub fn detect_backend(cache_dir: &std::path::Path) -> CacheBackend {
    // The blockfile backend stores everything in `index` plus `data_0..data_3`.
    if cache_dir.join("data_0").is_file() && cache_dir.join("index").is_file() {
        return CacheBackend::Blockfile;
    }
    // The simple cache keeps its index in a sibling directory and one file per
    // entry; either signal is enough.
    if cache_dir.join("index-dir").is_dir() {
        return CacheBackend::Simple;
    }
    if let Ok(entries) = std::fs::read_dir(cache_dir) {
        for entry in entries.filter_map(Result::ok) {
            if entry.path().to_string_lossy().ends_with("_0") {
                return CacheBackend::Simple;
            }
        }
    }

    CacheBackend::Unrecognized
}

/// Human-readable list of the locations a failed lookup actually checked,
/// naming any that use a cache format this reader cannot parse.
pub fn describe_searched_locations() -> String {
    let dirs = http_cache_dirs();
    if dirs.is_empty() {
        return "No Claude Desktop profile directory was found on this computer.".to_string();
    }

    let described: Vec<String> = dirs
        .iter()
        .map(|dir| match detect_backend(dir) {
            CacheBackend::Simple => dir.display().to_string(),
            CacheBackend::Blockfile => format!(
                "{} (Chromium blockfile cache — this reader only understands the simple cache format, so transcripts here cannot be read yet)",
                dir.display()
            ),
            CacheBackend::Unrecognized => {
                format!("{} (empty or unrecognized cache format)", dir.display())
            }
        })
        .collect();

    described.join(", ")
}

#[cfg(test)]
mod tests {
    use super::{detect_backend, CacheBackend};

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn recognizes_the_simple_cache_layout() {
        let dir = temp_dir("cse-backend-simple");
        std::fs::write(dir.join("a1b2c3d4e5f6a7b8_0"), b"entry").unwrap();
        assert_eq!(detect_backend(&dir), CacheBackend::Simple);
    }

    /// Chromium's other backend must be reported, not silently read as empty.
    #[test]
    fn recognizes_the_blockfile_layout() {
        let dir = temp_dir("cse-backend-blockfile");
        std::fs::write(dir.join("index"), b"idx").unwrap();
        for block in 0..4 {
            std::fs::write(dir.join(format!("data_{block}")), b"blk").unwrap();
        }
        assert_eq!(detect_backend(&dir), CacheBackend::Blockfile);
    }

    #[test]
    fn reports_an_unrecognized_directory() {
        let dir = temp_dir("cse-backend-unknown");
        std::fs::write(dir.join("something-else"), b"x").unwrap();
        assert_eq!(detect_backend(&dir), CacheBackend::Unrecognized);
    }
}
