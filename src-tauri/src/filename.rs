const ILLEGAL_FILENAME_CHARS: [char; 9] = ['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

pub fn sanitize_filename_part(value: &str, fallback: &str) -> String {
    let mut sanitized = String::with_capacity(value.len());

    for character in value.chars() {
        if ILLEGAL_FILENAME_CHARS.contains(&character) || character.is_control() {
            sanitized.push(' ');
        } else {
            sanitized.push(character);
        }
    }

    let collapsed = sanitized.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim_end_matches(['.', ' ']);

    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::sanitize_filename_part;

    #[test]
    fn strips_illegal_filename_characters() {
        assert_eq!(
            sanitize_filename_part("Condor: Workflow / Analysis?", "Fallback"),
            "Condor Workflow Analysis"
        );
    }

    #[test]
    fn falls_back_when_title_has_no_safe_content() {
        assert_eq!(sanitize_filename_part(":/\\*?", "Untitled"), "Untitled");
    }
}
