pub fn is_likely_claude_process_name(name: &str) -> bool {
    let normalized = normalize(name);
    normalized == "claude"
        || normalized == "claude.exe"
        || normalized == "claude desktop"
        || normalized == "claude desktop.exe"
        || normalized.contains("anthropicclaude")
}

pub fn is_likely_claude_window_title(title: &str) -> bool {
    normalize(title).contains("claude")
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

#[cfg(test)]
mod tests {
    use super::{clean_claude_window_title, is_likely_claude_process_name};

    #[test]
    fn detects_common_claude_process_names() {
        assert!(is_likely_claude_process_name("Claude.exe"));
        assert!(is_likely_claude_process_name("Claude Desktop.exe"));
        assert!(!is_likely_claude_process_name("notepad.exe"));
    }

    #[test]
    fn cleans_window_title_without_forcing_a_title() {
        assert_eq!(
            clean_claude_window_title("Condor Workflow - Claude"),
            Some("Condor Workflow".to_string())
        );
        assert_eq!(clean_claude_window_title("Claude"), None);
    }
}
