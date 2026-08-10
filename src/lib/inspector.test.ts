import { describe, expect, it } from "vitest";
import type { AccessibilityNode } from "../types";
import { nodeMatchesSearch } from "./inspector";

describe("nodeMatchesSearch", () => {
  it("matches across useful accessibility properties", () => {
    const node = makeNode({
      control_type: "Document",
      framework_id: "Chrome",
      supported_patterns: ["Text", "Scroll"],
    });

    expect(nodeMatchesSearch(node, "scroll")).toBe(true);
    expect(nodeMatchesSearch(node, "chrome")).toBe(true);
    expect(nodeMatchesSearch(node, "button")).toBe(false);
  });

  it("ignores empty searches", () => {
    expect(nodeMatchesSearch(makeNode({ name: "Claude" }), "  ")).toBe(false);
  });
});

function makeNode(partial: Partial<AccessibilityNode>): AccessibilityNode {
  return {
    id: 1,
    depth: 0,
    control_type: undefined,
    localized_control_type: undefined,
    name: undefined,
    automation_id: undefined,
    class_name: undefined,
    framework_id: undefined,
    value: undefined,
    bounds: undefined,
    enabled: undefined,
    has_keyboard_focus: undefined,
    offscreen: undefined,
    supported_patterns: [],
    child_count: 0,
    children: [],
    ...partial,
  };
}
