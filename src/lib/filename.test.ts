import { describe, expect, it } from "vitest";
import { defaultPdfFilename, sanitizeFilenamePart } from "./filename";

describe("sanitizeFilenamePart", () => {
  it("removes characters that are illegal on Windows and awkward on macOS", () => {
    expect(sanitizeFilenamePart('Condor: Workflow / "Analysis"?')).toBe(
      "Condor Workflow Analysis",
    );
  });

  it("uses a fallback when a title has no safe content", () => {
    expect(sanitizeFilenamePart(":/\\*?", "Untitled")).toBe("Untitled");
  });
});

describe("defaultPdfFilename", () => {
  it("builds the requested default export name", () => {
    const date = new Date("2026-08-10T12:00:00-04:00");
    expect(defaultPdfFilename("Condor Workflow Analysis", date)).toBe(
      "Claude - Condor Workflow Analysis - 2026-08-10.pdf",
    );
  });
});
