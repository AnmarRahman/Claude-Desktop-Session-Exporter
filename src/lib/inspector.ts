import type { AccessibilityNode } from "../types";

export function nodeMatchesSearch(node: AccessibilityNode, searchText: string): boolean {
  const needle = searchText.trim().toLowerCase();
  if (!needle) return false;

  return [
    node.control_type,
    node.localized_control_type,
    node.name,
    node.value,
    node.automation_id,
    node.class_name,
    node.framework_id,
    node.supported_patterns.join(" "),
  ]
    .filter(Boolean)
    .some((value) => String(value).toLowerCase().includes(needle));
}
