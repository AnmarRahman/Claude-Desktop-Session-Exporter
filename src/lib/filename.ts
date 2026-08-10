const ILLEGAL_FILENAME_CHARS = /[<>:"/\\|?*\u0000-\u001F]/g;
const WHITESPACE = /\s+/g;
const TRAILING_DOTS_SPACES = /[. ]+$/g;

export function sanitizeFilenamePart(value: string, fallback = "Claude Session"): string {
  const sanitized = value
    .replace(ILLEGAL_FILENAME_CHARS, " ")
    .replace(WHITESPACE, " ")
    .trim()
    .replace(TRAILING_DOTS_SPACES, "");

  return sanitized.length > 0 ? sanitized : fallback;
}

export function defaultPdfFilename(title: string | undefined, date = new Date()): string {
  const yyyy = date.getFullYear();
  const mm = String(date.getMonth() + 1).padStart(2, "0");
  const dd = String(date.getDate()).padStart(2, "0");
  return `Claude - ${sanitizeFilenamePart(title ?? "")} - ${yyyy}-${mm}-${dd}.pdf`;
}
