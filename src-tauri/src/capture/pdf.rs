//! Polished, local HTML-to-PDF rendering for normalized Claude transcripts.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use printpdf::{
    Base64OrRaw, BuiltinFont, Color, GeneratePdfOptions, Op, PdfDocument, PdfFontHandle,
    PdfSaveOptions, Point, Pt, Rgb, TextItem,
};
use pulldown_cmark::{html, Event, Options, Parser};

use crate::capture::progress::{self, ProgressStage};
use crate::capture::CaptureError;
use crate::models::{ChatExportBlock, ChatExportMessage};

const PAGE_WIDTH_PT: f32 = 595.28;
const FOOTER_LEFT: f32 = 45.0;
const FOOTER_Y: f32 = 19.0;

pub struct PdfTranscript<'a> {
    pub title: &'a str,
    pub source_type: &'a str,
    pub session_id: &'a str,
    pub model: Option<&'a str>,
    pub messages: &'a [ChatExportMessage],
}

/// Layout cost grows faster than linearly with document size, so a whole
/// multi-megabyte transcript never finishes as one document. Messages are laid
/// out in batches near this size and the resulting pages are concatenated,
/// which keeps every individual layout small enough to complete.
const CHUNK_TARGET_CHARS: usize = 120_000;

/// Batched, parallel layout runs at roughly six seconds per megabyte of
/// formatted HTML on a ten-core machine: a 12 MB transcript renders in about 75
/// seconds and produces a 2300-page, 150 MB file. This ceiling is set at twice
/// that measured point — beyond it neither the time nor the peak memory has
/// been verified, so the PDF is declined and the export still delivers Markdown
/// and JSON promptly rather than appearing to hang.
const MAX_PDF_CHARS: usize = 24_000_000;

/// Groups pre-rendered message HTML into batches near the target size. An
/// oversized single message still forms its own batch; splitting inside a
/// message would break the layout it depends on.
fn chunk_messages(messages: &[ChatExportMessage]) -> Vec<String> {
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();

    for message in messages {
        let html = message_html(message);
        if !current.is_empty() && current.len() + html.len() > CHUNK_TARGET_CHARS {
            chunks.push(std::mem::take(&mut current));
        }
        current.push_str(&html);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    if chunks.is_empty() {
        chunks.push(String::new());
    }
    chunks
}

pub fn render_pdf(document: &PdfTranscript<'_>) -> Result<(Vec<u8>, Vec<String>), CaptureError> {
    let images = BTreeMap::new();
    let mut fonts = BTreeMap::new();
    let mut warnings = Vec::new();

    if let Some((name, bytes)) = load_html_font() {
        fonts.insert(name, Base64OrRaw::Raw(bytes));
    } else {
        warnings.push(
            "No preferred system font was available; the PDF used the renderer's sans-serif fallback."
                .to_string(),
        );
    }

    let options = GeneratePdfOptions {
        font_embedding: Some(true),
        page_width: Some(210.0),
        page_height: Some(297.0),
        margin_top: Some(14.0),
        margin_right: Some(15.0),
        margin_bottom: Some(17.0),
        margin_left: Some(15.0),
        image_optimization: None,
        // We draw a quieter custom footer after layout so it can omit the cover.
        show_page_numbers: Some(false),
        header_text: None,
        footer_text: None,
        skip_first_page: Some(true),
    };

    let batches = chunk_messages(document.messages);
    let total: usize = batches.iter().map(String::len).sum();
    if total > MAX_PDF_CHARS {
        return Err(CaptureError::PdfTooLarge(format!(
            "This transcript is about {:.0} MB of formatted text, which is too large to lay out as a PDF in reasonable time. Markdown and JSON were still written.",
            total as f64 / 1_000_000.0
        )));
    }

    // Batches are independent layouts, so they are rendered across the machine's
    // cores and merged in order afterwards. Ordering comes from the slot index,
    // never from completion order.
    let batch_count = batches.len();
    let mut slots: Vec<Option<(PdfDocument, Vec<printpdf::PdfWarnMsg>)>> =
        (0..batch_count).map(|_| None).collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let completed = std::sync::atomic::AtomicUsize::new(0);
    let slots_cell: Vec<std::sync::Mutex<Option<(PdfDocument, Vec<printpdf::PdfWarnMsg>)>>> =
        (0..batch_count).map(|_| std::sync::Mutex::new(None)).collect();
    let failure: std::sync::Mutex<Option<CaptureError>> = std::sync::Mutex::new(None);

    let workers = std::thread::available_parallelism()
        .map(|value| value.get())
        .unwrap_or(4)
        .min(batch_count.max(1));

    progress::report(ProgressStage::RenderingPdf, 0, batch_count);

    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let index = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if index >= batch_count || failure.lock().unwrap().is_some() {
                    break;
                }
                let html = transcript_html_with_body(document, &batches[index], index == 0);
                let mut options = options.clone();
                // Only the merged document's very first page is the cover.
                options.skip_first_page = Some(index == 0);

                let mut warnings = Vec::new();
                match PdfDocument::from_html(&html, &images, &fonts, &options, &mut warnings) {
                    Ok(part) => {
                        *slots_cell[index].lock().unwrap() = Some((part, warnings));
                        let done = completed.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                        progress::report(ProgressStage::RenderingPdf, done, batch_count);
                    }
                    Err(error) => {
                        *failure.lock().unwrap() = Some(CaptureError::Diagnostic(format!(
                            "Could not lay out PDF (batch {} of {batch_count}): {error}",
                            index + 1
                        )));
                        break;
                    }
                }
            });
        }
    });

    if let Some(error) = failure.into_inner().unwrap() {
        return Err(error);
    }
    for (slot, cell) in slots.iter_mut().zip(slots_cell) {
        *slot = cell.into_inner().unwrap();
    }

    let mut render_warnings = Vec::new();
    let mut merged: Option<PdfDocument> = None;
    for slot in slots {
        let (part, part_warnings) = slot.ok_or_else(|| {
            CaptureError::Diagnostic("A PDF batch did not finish rendering.".to_string())
        })?;
        render_warnings.extend(part_warnings);
        match merged.as_mut() {
            Some(document) => document.append_document(part),
            None => merged = Some(part),
        }
    }

    let mut pdf = merged
        .ok_or_else(|| CaptureError::Diagnostic("PDF renderer produced no pages.".to_string()))?;

    let now = printpdf::date::OffsetDateTime::now();
    pdf.metadata.info.creation_date = now;
    pdf.metadata.info.modification_date = now;
    pdf.metadata.info.metadata_date = now;
    pdf.metadata.info.document_title = clean_display_title(document.title);
    pdf.metadata.info.creator = "Claude Session Exporter".to_string();
    pdf.metadata.info.producer = "Claude Session Exporter".to_string();
    pdf.metadata.info.subject = "Local Claude transcript export".to_string();

    let total_pages = pdf.pages.len();
    for (index, page) in pdf.pages.iter_mut().enumerate() {
        if index == 0 {
            continue;
        }
        add_footer(&mut page.ops, index + 1, total_pages);
    }

    if !render_warnings.is_empty() {
        // Carry the first few messages through: a bare count gives the user
        // nothing to act on and hides real layout failures.
        warnings.push(format!(
            "PDF layout reported {} non-fatal warning(s): {}",
            render_warnings.len(),
            summarize_warnings(&render_warnings)
        ));
    }

    let save_options = PdfSaveOptions {
        subset_fonts: true,
        optimize: true,
        ..PdfSaveOptions::default()
    };
    let mut save_warnings = Vec::new();
    let bytes = pdf.save(&save_options, &mut save_warnings);
    if bytes.is_empty() {
        return Err(CaptureError::Diagnostic(
            "PDF renderer returned an empty document.".to_string(),
        ));
    }
    if !save_warnings.is_empty() {
        warnings.push(format!(
            "PDF writer reported {} non-fatal warning(s): {}",
            save_warnings.len(),
            summarize_warnings(&save_warnings)
        ));
    }
    Ok((bytes, warnings))
}

/// Keeps the first few renderer messages so a failed layout is diagnosable
/// from the app instead of only reporting how many warnings occurred.
fn summarize_warnings(warnings: &[printpdf::PdfWarnMsg]) -> String {
    const SHOWN: usize = 3;
    let mut summary = warnings
        .iter()
        .take(SHOWN)
        .map(|warning| warning.msg.trim().to_string())
        .collect::<Vec<_>>()
        .join("; ");
    if warnings.len() > SHOWN {
        summary.push_str(&format!(" (+{} more)", warnings.len() - SHOWN));
    }
    summary
}

fn transcript_html(document: &PdfTranscript<'_>) -> String {
    let mut messages = String::new();
    for message in document.messages {
        messages.push_str(&message_html(message));
    }
    transcript_html_with_body(document, &messages, true)
}

/// Builds one renderable document. `with_cover` is false for continuation
/// batches, whose pages are appended to the first batch's document.
fn transcript_html_with_body(
    document: &PdfTranscript<'_>,
    messages: &str,
    with_cover: bool,
) -> String {
    let cover = if with_cover {
        cover_section(document)
    } else {
        String::new()
    };

    format!(
        r#"<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8" />
<style>
  * {{ box-sizing: border-box; }}
  body {{
    margin: 0;
    color: #272522;
    font-family: ChatSans, Arial, sans-serif;
    font-size: 10.25pt;
    line-height: 1.52;
  }}
  .cover {{
    min-height: 252mm;
    page-break-after: always;
    position: relative;
    padding: 18mm 10mm 12mm;
  }}
  .cover-rule {{ width: 17mm; height: 3px; background: #d97757; margin-bottom: 17mm; }}
  .cover-kicker {{ color: #6f6a62; font-size: 9pt; font-weight: 700; letter-spacing: 1.4px; text-transform: uppercase; }}
  .cover h1 {{
    color: #22201e;
    font-size: 28pt;
    font-weight: 600;
    line-height: 1.13;
    margin: 8mm 0 16mm;
  }}
  .cover-summary {{ color: #5f5a53; font-size: 11pt; margin-bottom: 13mm; max-width: 135mm; }}
  .metadata {{ border-top: 1px solid #dedbd5; padding-top: 7mm; width: 100%; }}
  .metadata-row {{ margin-bottom: 3.5mm; }}
  .metadata-label {{ color: #8a847b; display: inline-block; font-size: 8pt; font-weight: 700; letter-spacing: .7px; text-transform: uppercase; width: 26mm; }}
  .metadata-value {{ color: #3e3a36; font-size: 9.5pt; }}
  .cover-count {{ color: #d97757; font-size: 9pt; font-weight: 700; margin-top: 10mm; }}
  .conversation {{ padding: 2mm 0 0; }}
  .message {{ margin: 0 0 10mm; }}
  .assistant-header {{
    color: #2d2a27;
    font-size: 9.5pt;
    font-weight: 700;
    margin: 0 0 4mm;
    page-break-after: avoid;
  }}
  .claude-mark {{ color: #d97757; font-size: 18pt; line-height: .5; margin-right: 2.5mm; vertical-align: -1pt; }}
  .message-time {{ color: #98938c; font-size: 7.5pt; font-weight: 400; margin-left: 3mm; }}
  .assistant-body {{ padding-left: 1mm; }}
  .user {{ break-inside: avoid; page-break-inside: avoid; margin: 7mm 0 11mm 24%; text-align: left; }}
  .user-meta {{ color: #7c7770; font-size: 7.5pt; margin: 0 2mm 2mm 0; page-break-after: avoid; text-align: right; }}
  .user-name {{ color: #2f2c28; font-size: 9pt; font-weight: 700; margin-left: 2mm; }}
  .user-bubble {{
    background: #f2f3f4;
    border: 1px solid #e8e9ea;
    border-radius: 7px;
    break-inside: avoid;
    color: #292724;
    display: block;
    page-break-inside: avoid;
    padding: 4mm 5mm;
    text-align: left;
    width: 100%;
  }}
  p {{ margin: 0 0 4.3mm; orphans: 3; widows: 3; }}
  h1, h2, h3, h4 {{ color: #282522; font-weight: 700; line-height: 1.28; page-break-after: avoid; }}
  h1 {{ font-size: 17pt; margin: 8mm 0 4mm; }}
  h2 {{ font-size: 13.5pt; margin: 7mm 0 3mm; }}
  h3 {{ font-size: 11.5pt; margin: 6mm 0 2.5mm; }}
  h4 {{ font-size: 10.5pt; margin: 5mm 0 2mm; }}
  strong {{ font-weight: 700; color: #1f1d1b; }}
  em {{ color: #56514b; }}
  ul, ol {{ margin: 1mm 0 5mm 7mm; padding-left: 5mm; }}
  ol {{ list-style-type: none; }}
  li {{ margin-bottom: 2mm; padding-left: 1mm; }}
  blockquote {{ border-left: 3px solid #d9a18f; color: #59544e; margin: 4mm 0; padding: 2mm 0 2mm 5mm; }}
  a {{ color: #3176a8; text-decoration: underline; }}
  code {{ background: #f2f2f1; color: #b4543c; font-family: monospace; font-size: 8.5pt; padding: 1px 3px; }}
  pre {{
    background: #f4f4f3;
    border: 1px solid #e7e5e2;
    color: #48444e;
    font-family: monospace;
    font-size: 8.1pt;
    line-height: 1.46;
    margin: 4mm 0 6mm;
    padding: 5mm;
    white-space: pre-wrap;
  }}
  pre code {{ background: transparent; color: inherit; padding: 0; }}
  table {{ border-collapse: collapse; font-size: 8.5pt; margin: 4mm 0 6mm; width: 100%; }}
  th {{ background: #eeeae5; color: #37332f; font-weight: 700; padding: 2.5mm; text-align: left; }}
  td {{ border-bottom: 1px solid #e1ddd8; padding: 2.5mm; vertical-align: top; }}
  hr {{ border: 0; border-top: 1px solid #dedbd6; margin: 7mm 0; }}
  .activity {{
    background: #f6f4f0;
    border-left: 3px solid #b7a890;
    margin: 4mm 0 6mm;
    padding: 4mm 5mm;
  }}
  .activity.error {{ background: #fbefec; border-left-color: #bf674e; }}
  .activity-label {{ color: #77634e; font-size: 8pt; font-weight: 700; letter-spacing: .5px; margin-bottom: 2mm; text-transform: uppercase; }}
  .activity pre {{ background: #eeece8; border: 0; margin: 2mm 0 0; padding: 3mm; }}
  .thinking {{ background: #f7f6f4; border-left: 3px solid #c9c3ba; color: #656059; margin: 4mm 0; padding: 4mm 5mm; }}
  .thinking-label {{ color: #8b857c; font-size: 8pt; font-weight: 700; letter-spacing: .6px; text-transform: uppercase; }}
  .file-card {{ background: #f3f5f6; border: 1px solid #e3e6e8; margin: 3mm 0; padding: 3mm 4mm; }}
  .file-kind {{ color: #7d7871; font-size: 7.5pt; font-weight: 700; text-transform: uppercase; }}
  .references {{ color: #4d6577; font-size: 8.5pt; margin-top: 3mm; }}
  .raw-label {{ color: #8b857c; font-size: 7.5pt; font-weight: 700; margin-top: 3mm; text-transform: uppercase; }}
</style>
</head>
<body>
  {cover}
  <main class="conversation">{messages}</main>
</body>
</html>"#,
        cover = cover,
    )
}

/// The cover belongs to the first rendered batch only, so the merged document
/// carries exactly one.
fn cover_section(document: &PdfTranscript<'_>) -> String {
    let model = document
        .model
        .map(|value| metadata_row("Model", value))
        .unwrap_or_default();
    format!(
        r#"<section class="cover">
    <div class="cover-rule"></div>
    <div class="cover-kicker">Claude Session Export</div>
    <h1>{title}</h1>
    <p class="cover-summary">A complete local transcript, formatted for comfortable reading and long-term reference.</p>
    <div class="metadata">
      {source_row}
      {session_row}
      {model}
    </div>
    <div class="cover-count">{count} messages archived</div>
  </section>"#,
        title = escape_html(&clean_display_title(document.title)),
        source_row = metadata_row("Source", &escape_html(document.source_type)),
        session_row = metadata_row("Session", &escape_html(document.session_id)),
        model = model,
        count = document.messages.len(),
    )
}

fn metadata_row(label: &str, value: &str) -> String {
    format!(
        "<div class=\"metadata-row\"><span class=\"metadata-label\">{}</span><span class=\"metadata-value\">{}</span></div>",
        escape_html(label),
        escape_html(value)
    )
}

fn clean_display_title(value: &str) -> String {
    value
        .trim()
        .trim_matches(|character| matches!(character, '*' | '#' | '_' | '`'))
        .trim()
        .to_string()
}

fn message_html(message: &ChatExportMessage) -> String {
    let contents = if message.blocks.is_empty() {
        markdown_html(&message.text)
    } else {
        message.blocks.iter().map(block_html).collect()
    };
    let assistant_timestamp = message
        .timestamp
        .as_deref()
        .map(|value| {
            format!(
                "<span class=\"message-time\"> · {}</span>",
                escape_html(value)
            )
        })
        .unwrap_or_default();
    let user_timestamp = message
        .timestamp
        .as_deref()
        .map(|value| format!("{} · ", escape_html(value)))
        .unwrap_or_default();

    if contents.trim().is_empty() {
        return String::new();
    }

    if message.role == "user" {
        format!(
            "<section class=\"message user\"><div class=\"user-meta\">{user_timestamp}<span class=\"user-name\">You asked</span></div><div class=\"user-bubble\">{contents}</div></section>"
        )
    } else {
        format!(
            "<section class=\"message assistant\"><div class=\"assistant-header\"><span class=\"claude-mark\">✦</span>Claude{assistant_timestamp}</div><div class=\"assistant-body\">{contents}</div></section>"
        )
    }
}

fn block_html(block: &ChatExportBlock) -> String {
    let text = block.text.as_deref().unwrap_or("").trim();
    let mut rendered = match block.kind.as_str() {
        "text" => markdown_html(text),
        "thinking" => format!(
            "<aside class=\"thinking\"><div class=\"thinking-label\">Thinking</div>{}</aside>",
            markdown_html(text)
        ),
        "tool_use" | "tool_result" => {
            let error_class = if block.is_error == Some(true) {
                " error"
            } else {
                ""
            };
            let label = if block.kind == "tool_use" {
                "Tool"
            } else {
                "Tool result"
            };
            let name = escape_html(block.tool_name.as_deref().unwrap_or("activity"));
            let mut body = if text.is_empty() {
                String::new()
            } else {
                markdown_html(text)
            };
            if let Some(input) = &block.tool_input {
                body.push_str(&format!("<pre>{}</pre>", escape_html(input)));
            }
            format!(
                "<aside class=\"activity{error_class}\"><div class=\"activity-label\">{label} - {name}</div>{body}</aside>"
            )
        }
        "attachment" | "file" => {
            let label = block
                .references
                .first()
                .and_then(|reference| reference.label.as_deref())
                .unwrap_or(block.kind.as_str());
            format!(
                "<div class=\"file-card\"><span class=\"file-kind\">{}</span><br />{}</div>",
                escape_html(&block.kind),
                escape_html(label)
            )
        }
        other => {
            let prose = if text.is_empty() {
                String::new()
            } else {
                markdown_html(text)
            };
            format!(
                "<aside class=\"activity\"><div class=\"activity-label\">{}</div>{prose}</aside>",
                escape_html(other)
            )
        }
    };

    if let Some(raw) = &block.raw {
        rendered.push_str(&format!(
            "<div class=\"raw-label\">Preserved source data</div><pre>{}</pre>",
            escape_html(raw)
        ));
    }
    if !block.references.is_empty() && !matches!(block.kind.as_str(), "attachment" | "file") {
        rendered.push_str("<ul class=\"references\">");
        for reference in &block.references {
            let label = reference
                .label
                .as_deref()
                .or(reference.url.as_deref())
                .unwrap_or(&reference.kind);
            let value = match &reference.url {
                Some(url) if url != label => format!("{} - {}", label, url),
                _ => label.to_string(),
            };
            rendered.push_str(&format!("<li>{}</li>", escape_html(&value)));
        }
        rendered.push_str("</ul>");
    }
    rendered
}

fn markdown_html(markdown: &str) -> String {
    if markdown.trim().is_empty() {
        return String::new();
    }
    let markdown = normalize_ordered_list_markers(markdown);
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_TASKLISTS);
    // Raw HTML in a message must never reach the renderer as markup. A chat can
    // legitimately contain a whole `<!DOCTYPE html>` document, and printpdf's HTML
    // bridge treats a nested document as a replacement for the real one, so the
    // export collapses to an empty page. Showing the source as text keeps the
    // content and the layout. Fenced code is unaffected: it arrives as `Text`.
    let parser = Parser::new_ext(&markdown, options).map(|event| match event {
        Event::Html(raw) => Event::Text(raw),
        Event::InlineHtml(raw) => Event::Text(raw),
        other => other,
    });
    let mut output = String::new();
    html::push_html(&mut output, parser);
    output
}

/// printpdf's HTML bridge currently paints ordered-list markers twice. Turning
/// them into ordinary Markdown paragraphs keeps the visible numbering and all
/// inline emphasis without asking the bridge to synthesize list counters.
fn normalize_ordered_list_markers(markdown: &str) -> String {
    let mut normalized = String::with_capacity(markdown.len());
    for line in markdown.lines() {
        let trimmed = line.trim_start();
        let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
        let ordered = digits > 0
            && trimmed
                .get(digits..)
                .is_some_and(|remainder| remainder.starts_with(". "));
        if ordered {
            let number = &trimmed[..digits];
            let content = &trimmed[digits + 2..];
            normalized.push_str(&format!("**{number}.** {content}\n\n"));
        } else {
            normalized.push_str(line);
            normalized.push('\n');
        }
    }
    normalized
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn load_html_font() -> Option<(String, Vec<u8>)> {
    font_candidates()
        .into_iter()
        .find_map(|path| fs::read(path).ok())
        .map(|bytes| ("ChatSans".to_string(), bytes))
}

fn font_candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    #[cfg(target_os = "macos")]
    {
        paths.push(PathBuf::from(
            "/System/Library/Fonts/Supplemental/Arial.ttf",
        ));
        paths.push(PathBuf::from(
            "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
        ));
    }
    #[cfg(windows)]
    {
        if let Some(windir) = std::env::var_os("WINDIR") {
            let fonts = PathBuf::from(windir).join("Fonts");
            paths.push(fonts.join("arial.ttf"));
            paths.push(fonts.join("segoeui.ttf"));
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        paths.push(PathBuf::from(
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        ));
        paths.push(PathBuf::from(
            "/usr/share/fonts/truetype/liberation2/LiberationSans-Regular.ttf",
        ));
    }
    paths
}

fn add_footer(ops: &mut Vec<Op>, page_number: usize, total_pages: usize) {
    let font = PdfFontHandle::Builtin(BuiltinFont::Helvetica);
    push_text_op(
        ops,
        &font,
        "Claude Session Exporter",
        7.5,
        FOOTER_LEFT,
        FOOTER_Y,
        rgb(0.20, 0.57, 0.49),
    );
    push_text_op(
        ops,
        &font,
        &format!("{page_number} / {total_pages}"),
        8.0,
        PAGE_WIDTH_PT - 73.0,
        FOOTER_Y,
        rgb(0.38, 0.36, 0.33),
    );
}

fn push_text_op(
    ops: &mut Vec<Op>,
    font: &PdfFontHandle,
    text: &str,
    size: f32,
    x: f32,
    y: f32,
    color: Color,
) {
    ops.push(Op::StartTextSection);
    ops.push(Op::SetTextCursor {
        pos: Point { x: Pt(x), y: Pt(y) },
    });
    ops.push(Op::SetFont {
        font: font.clone(),
        size: Pt(size),
    });
    ops.push(Op::SetFillColor { col: color });
    ops.push(Op::ShowText {
        items: vec![TextItem::Text(text.to_string())],
    });
    ops.push(Op::EndTextSection);
}

fn rgb(r: f32, g: f32, b: f32) -> Color {
    Color::Rgb(Rgb {
        r,
        g,
        b,
        icc_profile: None,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        chunk_messages, clean_display_title, escape_html, markdown_html,
        normalize_ordered_list_markers, render_pdf, transcript_html,
        transcript_html_with_body, PdfTranscript, MAX_PDF_CHARS,
    };
    use crate::capture::CaptureError;
    use crate::models::ChatExportMessage;

    fn fixture() -> (String, Vec<ChatExportMessage>) {
        let title = "MSAccess vs SQL data - PDF layout fixture".to_string();
        let messages = vec![
            ChatExportMessage::plain(
                "user",
                "Please compare **Access** and **SQL**, then preserve `order_id`.".to_string(),
                Some("2026-08-12 12:00".to_string()),
            ),
            ChatExportMessage::plain(
                "claude",
                "## Recommendation\n\nUse SQL for the shared system.\n\n1. Better concurrency\n2. Central backups\n\n```sql\nSELECT order_id FROM orders;\n```\n\nThe longer explanation should wrap naturally without clipping at the right margin.\n\n"
                    .repeat(9),
                None,
            ),
        ];
        (title, messages)
    }

    #[test]
    fn escapes_cover_metadata() {
        assert_eq!(escape_html("A & <B>"), "A &amp; &lt;B&gt;");
    }

    #[test]
    fn removes_markdown_markers_from_cover_title() {
        assert_eq!(
            clean_display_title("***MSAccess audit***"),
            "MSAccess audit"
        );
    }

    #[test]
    fn renders_markdown_hierarchy() {
        let html = markdown_html("## Heading\n\n**Bold** and `code`\n\n- one");
        assert!(html.contains("<h2>Heading</h2>"));
        assert!(html.contains("<strong>Bold</strong>"));
        assert!(html.contains("<code>code</code>"));
        assert!(html.contains("<li>one</li>"));
    }

    #[test]
    fn preserves_ordered_numbers_without_html_list_counters() {
        let normalized = normalize_ordered_list_markers("1. First\n2. Second");
        assert_eq!(normalized, "**1.** First\n\n**2.** Second\n\n");
        let html = markdown_html("1. First\n2. Second");
        assert!(!html.contains("<ol>"));
        assert!(html.contains("<strong>1.</strong> First"));
        assert!(html.contains("<strong>2.</strong> Second"));
    }

    #[test]
    fn keeps_a_dedicated_cover_before_messages() {
        let (title, messages) = fixture();
        let html = transcript_html(&PdfTranscript {
            title: &title,
            source_type: "Claude Desktop Cowork",
            session_id: "fixture-session",
            model: Some("claude-fable-5"),
            messages: &messages,
        });
        assert!(
            html.find("class=\"cover\"").unwrap() < html.find("class=\"conversation\"").unwrap()
        );
        assert!(html.contains("page-break-after: always"));
    }

    #[test]
    fn creates_a_multi_page_pdf() {
        let (title, messages) = fixture();
        let (bytes, _) = render_pdf(&PdfTranscript {
            title: &title,
            source_type: "Claude Desktop Cowork",
            session_id: "fixture-session",
            model: Some("claude-fable-5"),
            messages: &messages,
        })
        .unwrap();
        assert!(bytes.starts_with(b"%PDF-"));
        assert!(bytes.len() > 1_000);
    }

    #[test]
    #[ignore = "writes a synthetic PDF for visual QA"]
    fn writes_visual_fixture() {
        let path =
            std::env::var("CSE_PDF_FIXTURE_PATH").expect("set CSE_PDF_FIXTURE_PATH for visual QA");
        let (title, messages) = fixture();
        let (bytes, _) = render_pdf(&PdfTranscript {
            title: &title,
            source_type: "Claude Desktop Cowork",
            session_id: "fixture-session",
            model: Some("claude-fable-5"),
            messages: &messages,
        })
        .unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    #[ignore = "renders a previously exported JSON transcript for visual QA"]
    fn writes_json_visual_fixture() {
        let source = std::env::var("CSE_PDF_JSON_FIXTURE")
            .expect("set CSE_PDF_JSON_FIXTURE to an exported transcript JSON");
        let output =
            std::env::var("CSE_PDF_FIXTURE_PATH").expect("set CSE_PDF_FIXTURE_PATH for visual QA");
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(source).unwrap()).unwrap();
        let messages: Vec<ChatExportMessage> =
            serde_json::from_value(value["messages"].clone()).unwrap();
        let title = value["title"].as_str().unwrap_or("Claude Session");
        let source_type = value["source_type"].as_str().unwrap_or("Claude");
        let session_id = value["session_id"].as_str().unwrap_or("unknown");
        let model = value["model"].as_str();
        let (bytes, _) = render_pdf(&PdfTranscript {
            title,
            source_type,
            session_id,
            model,
            messages: &messages,
        })
        .unwrap();
        std::fs::write(output, bytes).unwrap();
    }

    /// A chat can contain a whole HTML document. Passed through as markup it
    /// replaces the real document and the export collapses to a blank page.
    #[test]
    fn renders_a_message_containing_a_full_html_document() {
        let messages = vec![
            ChatExportMessage::plain("user", "Show me a page.".to_string(), None),
            ChatExportMessage::plain(
                "claude",
                "<!DOCTYPE html><html><body><p>hi</p></body></html>".to_string(),
                None,
            ),
            ChatExportMessage::plain(
                "claude",
                "Tail content that must survive the payload.".repeat(40),
                None,
            ),
        ];
        let (bytes, warnings) = render_pdf(&PdfTranscript {
            title: "Raw HTML",
            source_type: "Claude Home",
            session_id: "raw-html",
            model: None,
            messages: &messages,
        })
        .unwrap();

        assert!(bytes.starts_with(b"%PDF-"));
        assert!(
            bytes.len() > 20_000,
            "a full-document payload blanked the PDF: {} bytes",
            bytes.len()
        );
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
    }

    /// Raw HTML becomes visible source text rather than markup.
    #[test]
    fn escapes_raw_html_instead_of_emitting_it() {
        let html = markdown_html("Inline <div class=\"x\"> and <br> tags.");
        assert!(html.contains("&lt;div"));
        assert!(html.contains("&lt;br&gt;"));
        assert!(!html.contains("<div"));
    }

    /// Fenced code must keep working: it never arrives as an HTML event.
    #[test]
    fn still_renders_fenced_code_blocks() {
        let html = markdown_html("```html\n<p>x</p>\n```");
        assert!(html.contains("<pre>"));
        assert!(html.contains("&lt;p&gt;x&lt;/p&gt;"));
    }

    /// A transcript large enough to need several batches must still carry one
    /// cover, not one per batch.
    #[test]
    fn splits_large_transcripts_into_batches_with_a_single_cover() {
        let long = "Paragraph text that occupies space in the batch. ".repeat(400);
        let messages: Vec<ChatExportMessage> = (0..40)
            .map(|i| {
                let role = if i % 2 == 0 { "user" } else { "claude" };
                ChatExportMessage::plain(role, long.clone(), None)
            })
            .collect();

        let batches = chunk_messages(&messages);
        assert!(batches.len() > 1, "expected several batches, got {}", batches.len());
        assert!(batches.iter().all(|batch| !batch.is_empty()));

        let document = PdfTranscript {
            title: "Batched",
            source_type: "Claude Home",
            session_id: "batched",
            model: None,
            messages: &messages,
        };
        let first = transcript_html_with_body(&document, &batches[0], true);
        let second = transcript_html_with_body(&document, &batches[1], false);
        assert!(first.contains("class=\"cover\""));
        assert!(!second.contains("class=\"cover\""));
    }

    /// Every message must land in exactly one batch.
    #[test]
    fn batching_preserves_every_message() {
        let messages: Vec<ChatExportMessage> = (0..25)
            .map(|i| ChatExportMessage::plain("user", format!("marker-{i} ").repeat(900), None))
            .collect();
        let joined = chunk_messages(&messages).concat();
        for i in 0..25 {
            assert!(joined.contains(&format!("marker-{i}")), "lost message {i}");
        }
    }

    /// A transcript beyond the ceiling declines the PDF with a clear reason
    /// rather than running for many minutes.
    #[test]
    fn declines_a_transcript_beyond_the_size_ceiling() {
        let huge = "x".repeat(MAX_PDF_CHARS / 8 + 1_000);
        let messages: Vec<ChatExportMessage> = (0..9)
            .map(|_| ChatExportMessage::plain("claude", huge.clone(), None))
            .collect();
        let error = render_pdf(&PdfTranscript {
            title: "Huge",
            source_type: "Claude Home",
            session_id: "huge",
            model: None,
            messages: &messages,
        })
        .unwrap_err();
        assert!(
            matches!(error, CaptureError::PdfTooLarge(_)),
            "expected PdfTooLarge, got {error:?}"
        );
        assert!(error.to_string().contains("Markdown and JSON"));
    }
}
