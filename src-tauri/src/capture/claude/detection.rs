pub fn is_likely_claude_process_name(name: &str) -> bool {
    let normalized = normalize(name);
    normalized == "claude"
        || normalized == "claude.exe"
        || normalized == "claude desktop"
        || normalized == "claude desktop.exe"
        || normalized.contains("anthropicclaude")
}

pub fn is_likely_claude_desktop_process(name: &str, path: Option<&str>) -> bool {
    is_likely_claude_process_name(name)
        && !path.map(is_known_non_desktop_process_path).unwrap_or(false)
}

pub fn is_likely_claude_window_title(title: &str) -> bool {
    let normalized = normalize(title);
    if is_known_non_desktop_window_title(title) {
        return false;
    }

    normalized == "claude"
        || normalized == "claude desktop"
        || normalized.starts_with("claude - ")
        || normalized.ends_with(" - claude")
}

pub fn is_exporter_window(title: &str, class_name: &str) -> bool {
    is_exporter_window_title(title) || normalize(class_name).contains("tauri")
}

pub fn is_known_non_desktop_window(title: &str, class_name: &str) -> bool {
    is_exporter_window(title, class_name)
        || is_known_non_desktop_window_title(title)
        || is_known_non_desktop_window_class(class_name)
}

fn is_exporter_window_title(title: &str) -> bool {
    normalize(title).contains("claude session exporter")
}

fn is_known_non_desktop_window_title(title: &str) -> bool {
    let normalized = normalize(title);
    [
        "visual studio code",
        "vs code",
        "cursor",
        "windsurf",
        "windows terminal",
        "powershell",
        "command prompt",
        "codex",
        "dde server window",
        "untitled",
    ]
    .iter()
    .any(|blocked| normalized.contains(blocked))
        || is_exporter_window_title(title)
}

fn is_known_non_desktop_window_class(class_name: &str) -> bool {
    let normalized = normalize(class_name);
    [
        "electron_systempreferenceshostwindow",
        "dde",
        "tooltips_class32",
    ]
    .iter()
    .any(|blocked| normalized.contains(blocked))
}

fn is_known_non_desktop_process_path(path: &str) -> bool {
    let normalized = normalize_path(path);
    [
        "/claude-code/",
        "/node_modules/",
        "/microsoft vs code/",
        "/visual studio code/",
        "/cursor/",
        "/windsurf/",
    ]
    .iter()
    .any(|blocked| normalized.contains(blocked))
}

pub fn clean_claude_window_title(title: &str) -> Option<String> {
    let cleaned = title
        .replace(" - Claude", "")
        .replace("Claude - ", "")
        .trim()
        .to_string();

    if cleaned.is_empty() || cleaned.eq_ignore_ascii_case("claude") {
        None
    } else {
        Some(cleaned)
    }
}

fn normalize(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn normalize_path(value: &str) -> String {
    normalize(value).replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::{
        clean_claude_window_title, is_exporter_window, is_known_non_desktop_window,
        is_likely_claude_desktop_process, is_likely_claude_process_name,
        is_likely_claude_window_title,
    };

    #[test]
    fn detects_common_claude_process_names() {
        assert!(is_likely_claude_process_name("Claude.exe"));
        assert!(is_likely_claude_process_name("Claude Desktop.exe"));
        assert!(!is_likely_claude_process_name("notepad.exe"));
    }

    #[test]
    fn excludes_claude_code_helper_process_paths() {
        assert!(is_likely_claude_desktop_process(
            "Claude.exe",
            Some(r"C:\Program Files\WindowsApps\Anthropic.Claude_1.0.0.0_x64__abc\app\Claude.exe")
        ));
        assert!(!is_likely_claude_desktop_process(
            "Claude.exe",
            Some(
                r"C:\Users\Anmar\AppData\Roaming\npm\node_modules\@anthropic-ai\claude-code\claude.exe"
            )
        ));
    }

    #[test]
    fn cleans_window_title_without_forcing_a_title() {
        assert_eq!(
            clean_claude_window_title("Condor Workflow - Claude"),
            Some("Condor Workflow".to_string())
        );
        assert_eq!(clean_claude_window_title("Claude"), None);
    }

    #[test]
    fn does_not_treat_exporter_as_claude_desktop() {
        assert!(!is_likely_claude_window_title("Claude Session Exporter"));
        assert!(is_exporter_window(
            "Claude Session Exporter",
            "Tauri Window"
        ));
        assert!(is_likely_claude_window_title("Claude"));
    }

    #[test]
    fn does_not_treat_editor_window_titles_as_claude_desktop() {
        assert!(!is_likely_claude_window_title(
            "Claude Extractor - Visual Studio Code"
        ));
        assert!(is_likely_claude_window_title("Condor Workflow - Claude"));
    }

    #[test]
    fn excludes_claude_desktop_helper_windows() {
        assert!(is_known_non_desktop_window(
            "Untitled",
            "Electron_SystemPreferencesHostWindow"
        ));
        assert!(is_known_non_desktop_window(
            "DDE Server Window",
            "Chrome_WidgetWin_1"
        ));
    }
}
