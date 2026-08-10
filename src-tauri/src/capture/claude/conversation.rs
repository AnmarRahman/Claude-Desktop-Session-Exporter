use crate::models::{
    AccessibilityNode, AccessibilitySnapshot, Bounds, ConversationCandidate, VisibleTextBlock,
};

const MIN_TEXT_CHARS_FOR_CANDIDATE: usize = 80;
const MAX_CANDIDATES: usize = 8;

#[derive(Default)]
struct NodeStats {
    descendants: usize,
    text_nodes: usize,
    text_chars: usize,
    button_count: usize,
    editable_count: usize,
    has_scroll_pattern: bool,
}

pub fn analyze_snapshot(mut snapshot: AccessibilitySnapshot) -> AccessibilitySnapshot {
    let Some(root) = snapshot.nodes.first() else {
        return snapshot;
    };

    snapshot.conversation_candidates = conversation_candidates(root);
    snapshot.visible_text_blocks = visible_text_blocks(root);
    snapshot
}

pub fn conversation_candidates(root: &AccessibilityNode) -> Vec<ConversationCandidate> {
    let mut candidates = Vec::new();
    collect_candidates(root, &mut candidates);
    candidates.sort_by(|left, right| {
        right
            .confidence
            .partial_cmp(&left.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    candidates.truncate(MAX_CANDIDATES);
    for (index, candidate) in candidates.iter_mut().enumerate() {
        candidate.id = index + 1;
    }
    candidates
}

pub fn visible_text_blocks(root: &AccessibilityNode) -> Vec<VisibleTextBlock> {
    let mut blocks = Vec::new();
    collect_text_blocks(root, &mut blocks);
    blocks.sort_by(|left, right| {
        let left_bounds = left.bounds.unwrap_or_default();
        let right_bounds = right.bounds.unwrap_or_default();
        left_bounds
            .y
            .cmp(&right_bounds.y)
            .then(left_bounds.x.cmp(&right_bounds.x))
            .then(left.node_id.cmp(&right.node_id))
    });
    blocks
}

fn collect_candidates(node: &AccessibilityNode, candidates: &mut Vec<ConversationCandidate>) {
    let stats = stats_for(node);
    let (confidence, reasons) = score_candidate(node, &stats);

    if confidence >= 0.35 {
        candidates.push(ConversationCandidate {
            id: 0,
            node_id: node.id,
            control_type: node.control_type.clone(),
            name: node.name.clone(),
            bounds: node.bounds,
            descendant_count: stats.descendants,
            text_node_count: stats.text_nodes,
            text_character_count: stats.text_chars,
            button_count: stats.button_count,
            editable_count: stats.editable_count,
            scrollable: stats.has_scroll_pattern,
            confidence,
            reasons,
        });
    }

    for child in &node.children {
        collect_candidates(child, candidates);
    }
}

fn collect_text_blocks(node: &AccessibilityNode, blocks: &mut Vec<VisibleTextBlock>) {
    if node.offscreen != Some(true) {
        let text = node_text(node);
        if let Some(text) = text {
            if looks_like_conversation_text(node, &text) {
                let (author, confidence, reason) = classify_author(node);
                blocks.push(VisibleTextBlock {
                    node_id: node.id,
                    text,
                    control_type: node.control_type.clone(),
                    bounds: node.bounds,
                    author,
                    confidence,
                    reason,
                });
            }
        }
    }

    for child in &node.children {
        collect_text_blocks(child, blocks);
    }
}

fn score_candidate(node: &AccessibilityNode, stats: &NodeStats) -> (f32, Vec<String>) {
    let mut score: f32 = 0.0;
    let mut reasons = Vec::new();

    if stats.has_scroll_pattern {
        score += 0.32;
        reasons.push("supports Scroll pattern".to_string());
    }

    if stats.text_nodes >= 8 {
        score += 0.24;
        reasons.push(format!("contains {} text nodes", stats.text_nodes));
    }

    if stats.text_chars >= MIN_TEXT_CHARS_FOR_CANDIDATE {
        score += 0.18;
        reasons.push(format!("contains {} text characters", stats.text_chars));
    }

    if let Some(bounds) = node.bounds {
        if bounds.height >= 360 && bounds.width >= 420 {
            score += 0.16;
            reasons.push(format!("large region {}x{}", bounds.width, bounds.height));
        }
    }

    if stats.button_count >= 2 {
        score += 0.06;
        reasons.push(format!("contains {} buttons", stats.button_count));
    }

    if stats.editable_count > 0 {
        score += 0.04;
        reasons.push("contains editable descendants".to_string());
    }

    (score.min(1.0), reasons)
}

fn stats_for(node: &AccessibilityNode) -> NodeStats {
    let mut stats = NodeStats::default();
    accumulate_stats(node, &mut stats);
    stats.descendants = stats.descendants.saturating_sub(1);
    stats
}

fn accumulate_stats(node: &AccessibilityNode, stats: &mut NodeStats) {
    stats.descendants += 1;
    if node
        .supported_patterns
        .iter()
        .any(|pattern| pattern == "Scroll")
    {
        stats.has_scroll_pattern = true;
    }

    let control = node.control_type.as_deref().unwrap_or_default();
    if control.contains("Button") {
        stats.button_count += 1;
    }
    if control.contains("Edit")
        || node
            .supported_patterns
            .iter()
            .any(|pattern| pattern == "Value")
    {
        stats.editable_count += 1;
    }

    if let Some(text) = node_text(node) {
        stats.text_nodes += 1;
        stats.text_chars += text.chars().count();
    }

    for child in &node.children {
        accumulate_stats(child, stats);
    }
}

fn node_text(node: &AccessibilityNode) -> Option<String> {
    let text = node.value.as_ref().or(node.name.as_ref())?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn looks_like_conversation_text(node: &AccessibilityNode, text: &str) -> bool {
    if text.chars().count() < 2 {
        return false;
    }

    let control = node.control_type.as_deref().unwrap_or_default();
    control.contains("Text")
        || control.contains("Document")
        || control.contains("Edit")
        || node
            .supported_patterns
            .iter()
            .any(|pattern| pattern == "Text")
        || text.chars().count() >= 20
}

fn classify_author(node: &AccessibilityNode) -> (String, f32, Option<String>) {
    let Some(bounds) = node.bounds else {
        return (
            "unknown".to_string(),
            0.0,
            Some("no structural bounds available".to_string()),
        );
    };

    if bounds.width > 0 && bounds.x > 900 && bounds.width < 720 {
        return (
            "user".to_string(),
            0.25,
            Some("right-aligned narrow text region; low confidence".to_string()),
        );
    }

    (
        "unknown".to_string(),
        0.0,
        Some("no validated structural author signal".to_string()),
    )
}

impl Default for Bounds {
    fn default() -> Self {
        Self {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{conversation_candidates, visible_text_blocks};
    use crate::models::{AccessibilityNode, Bounds};

    #[test]
    fn scores_large_scrollable_text_regions_as_candidates() {
        let root = node(
            1,
            "Pane",
            Some(Bounds {
                x: 0,
                y: 0,
                width: 900,
                height: 900,
            }),
            vec!["Scroll"],
            vec![
                text_node(2, "A substantial visible message in a Claude conversation."),
                text_node(
                    3,
                    "Another substantial visible response with enough content.",
                ),
                text_node(4, "More text from the same rendered timeline."),
                text_node(5, "Additional visible timeline text."),
                text_node(6, "A fifth text block."),
                text_node(7, "A sixth text block."),
                text_node(8, "A seventh text block."),
                text_node(9, "An eighth text block."),
            ],
        );

        let candidates = conversation_candidates(&root);
        assert_eq!(candidates[0].node_id, 1);
        assert!(candidates[0].confidence > 0.7);
    }

    #[test]
    fn orders_visible_text_by_screen_position() {
        let root = node(
            1,
            "Pane",
            None,
            vec![],
            vec![
                text_node_with_bounds(2, "Second", 40, 200),
                text_node_with_bounds(3, "First", 40, 100),
            ],
        );

        let blocks = visible_text_blocks(&root);
        assert_eq!(blocks[0].text, "First");
        assert_eq!(blocks[1].text, "Second");
    }

    fn text_node(id: usize, text: &str) -> AccessibilityNode {
        text_node_with_bounds(id, text, 10, id as i32 * 10)
    }

    fn text_node_with_bounds(id: usize, text: &str, x: i32, y: i32) -> AccessibilityNode {
        let mut node = node(
            id,
            "Text",
            Some(Bounds {
                x,
                y,
                width: 400,
                height: 24,
            }),
            vec![],
            vec![],
        );
        node.name = Some(text.to_string());
        node
    }

    fn node(
        id: usize,
        control_type: &str,
        bounds: Option<Bounds>,
        patterns: Vec<&str>,
        children: Vec<AccessibilityNode>,
    ) -> AccessibilityNode {
        AccessibilityNode {
            id,
            depth: 0,
            control_type: Some(control_type.to_string()),
            localized_control_type: None,
            name: None,
            automation_id: None,
            class_name: None,
            framework_id: None,
            value: None,
            bounds,
            enabled: Some(true),
            has_keyboard_focus: Some(false),
            offscreen: Some(false),
            supported_patterns: patterns.into_iter().map(str::to_string).collect(),
            child_count: children.len(),
            children,
        }
    }
}
