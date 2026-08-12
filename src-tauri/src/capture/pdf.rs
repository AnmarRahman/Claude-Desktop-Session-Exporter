//! Native, local PDF rendering for normalized Claude transcripts.

use std::fs;
use std::path::PathBuf;

use printpdf::{
    BuiltinFont, Color, Mm, Op, ParsedFont, PdfDocument, PdfFontHandle, PdfPage, PdfSaveOptions,
    Point, Pt, Rgb, TextItem,
};

use crate::capture::CaptureError;
use crate::models::{ChatExportBlock, ChatExportMessage};

const PAGE_WIDTH_PT: f32 = 595.28;
const PAGE_HEIGHT_PT: f32 = 841.89;
const LEFT: f32 = 48.0;
const RIGHT: f32 = 48.0;
const TOP: f32 = 70.0;
const BOTTOM: f32 = 48.0;

pub struct PdfTranscript<'a> {
    pub title: &'a str,
    pub source_type: &'a str,
    pub session_id: &'a str,
    pub model: Option<&'a str>,
    pub messages: &'a [ChatExportMessage],
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LineStyle {
    Title,
    Metadata,
    User,
    Claude,
    Body,
    Thinking,
    Tool,
    Reference,
}

#[derive(Clone)]
struct StyledLine {
    text: String,
    style: LineStyle,
    space_before: f32,
}

pub fn render_pdf(document: &PdfTranscript<'_>) -> Result<(Vec<u8>, Vec<String>), CaptureError> {
    let mut pdf = PdfDocument::new(document.title);
    let now = printpdf::date::OffsetDateTime::now();
    pdf.metadata.info.creation_date = now;
    pdf.metadata.info.modification_date = now;
    pdf.metadata.info.metadata_date = now;
    pdf.metadata.info.document_title = document.title.to_string();
    pdf.metadata.info.creator = "Claude Session Exporter".to_string();
    pdf.metadata.info.producer = "Claude Session Exporter".to_string();
    pdf.metadata.info.subject = "Local Claude transcript export".to_string();
    let (font, mut warnings) = load_font(&mut pdf);
    let lines = transcript_lines(document);
    let pages = paginate(&lines);
    let total_pages = pages.len();
    let pdf_pages = pages
        .into_iter()
        .enumerate()
        .map(|(index, page)| render_page(&font, document.title, &page, index + 1, total_pages))
        .collect();
    let options = PdfSaveOptions {
        subset_fonts: true,
        optimize: true,
        ..PdfSaveOptions::default()
    };
    let mut pdf_warnings = Vec::new();
    let bytes = pdf.with_pages(pdf_pages).save(&options, &mut pdf_warnings);
    if bytes.is_empty() {
        return Err(CaptureError::Diagnostic(
            "PDF renderer returned an empty document.".to_string(),
        ));
    }
    if !pdf_warnings.is_empty() {
        warnings.push(format!(
            "PDF renderer reported {} non-fatal warning(s).",
            pdf_warnings.len()
        ));
    }
    Ok((bytes, warnings))
}

fn load_font(pdf: &mut PdfDocument) -> (PdfFontHandle, Vec<String>) {
    let candidates = font_candidates();
    for path in &candidates {
        let Ok(bytes) = fs::read(path) else {
            continue;
        };
        let mut parse_warnings = Vec::new();
        if let Some(parsed) = ParsedFont::from_bytes(&bytes, 0, &mut parse_warnings) {
            let id = pdf.add_font(&parsed);
            return (PdfFontHandle::External(id), Vec::new());
        }
    }
    (
        PdfFontHandle::Builtin(BuiltinFont::Helvetica),
        vec![
            "No Unicode system font was available; the PDF used Helvetica and may replace unsupported characters."
                .to_string(),
        ],
    )
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

fn transcript_lines(document: &PdfTranscript<'_>) -> Vec<StyledLine> {
    let mut lines = Vec::new();
    push_wrapped(&mut lines, document.title, LineStyle::Title, 48, 0.0);
    push_wrapped(
        &mut lines,
        &format!("Source: {}", document.source_type),
        LineStyle::Metadata,
        92,
        12.0,
    );
    push_wrapped(
        &mut lines,
        &format!("Session: {}", document.session_id),
        LineStyle::Metadata,
        92,
        2.0,
    );
    if let Some(model) = document.model {
        push_wrapped(
            &mut lines,
            &format!("Model: {model}"),
            LineStyle::Metadata,
            92,
            2.0,
        );
    }

    for message in document.messages {
        let style = if message.role == "user" {
            LineStyle::User
        } else {
            LineStyle::Claude
        };
        let heading = match &message.timestamp {
            Some(timestamp) => format!("{}  |  {timestamp}", message.role.to_uppercase()),
            None => message.role.to_uppercase(),
        };
        push_wrapped(&mut lines, &heading, style, 82, 18.0);
        if message.blocks.is_empty() {
            push_paragraphs(&mut lines, &message.text, LineStyle::Body, 94, 7.0);
        } else {
            for block in &message.blocks {
                push_block(&mut lines, block);
            }
        }
    }
    lines
}

fn push_block(lines: &mut Vec<StyledLine>, block: &ChatExportBlock) {
    let text = block.text.as_deref().unwrap_or("").trim();
    match block.kind.as_str() {
        "text" => push_paragraphs(lines, text, LineStyle::Body, 94, 7.0),
        "thinking" => {
            push_wrapped(lines, "THINKING", LineStyle::Thinking, 90, 8.0);
            push_paragraphs(lines, text, LineStyle::Thinking, 90, 4.0);
        }
        "tool_use" | "tool_result" => {
            let label = format!(
                "{}: {}{}",
                if block.kind == "tool_use" {
                    "TOOL"
                } else {
                    "TOOL RESULT"
                },
                block.tool_name.as_deref().unwrap_or("unknown"),
                if block.is_error == Some(true) {
                    " (error)"
                } else {
                    ""
                }
            );
            push_wrapped(lines, &label, LineStyle::Tool, 86, 8.0);
            if !text.is_empty() {
                push_paragraphs(lines, text, LineStyle::Tool, 88, 4.0);
            }
            if let Some(input) = &block.tool_input {
                push_paragraphs(lines, input, LineStyle::Tool, 88, 4.0);
            }
            if let Some(raw) = &block.raw {
                push_paragraphs(lines, raw, LineStyle::Tool, 88, 4.0);
            }
        }
        "attachment" | "file" => {
            let label = block
                .references
                .first()
                .and_then(|reference| reference.label.as_deref())
                .unwrap_or(block.kind.as_str());
            push_wrapped(
                lines,
                &format!("{}: {label}", block.kind.to_uppercase()),
                LineStyle::Tool,
                86,
                8.0,
            );
        }
        _ => {
            push_wrapped(lines, &block.kind.to_uppercase(), LineStyle::Tool, 86, 8.0);
            push_paragraphs(lines, text, LineStyle::Body, 94, 4.0);
        }
    }
    for reference in &block.references {
        let label = reference
            .label
            .as_deref()
            .or(reference.url.as_deref())
            .unwrap_or(&reference.kind);
        let text = match &reference.url {
            Some(url) if url != label => format!("- {label}: {url}"),
            _ => format!("- {label}"),
        };
        push_wrapped(lines, &text, LineStyle::Reference, 88, 3.0);
    }
}

fn push_paragraphs(
    lines: &mut Vec<StyledLine>,
    text: &str,
    style: LineStyle,
    width: usize,
    first_space: f32,
) {
    let mut first = true;
    for paragraph in text.lines() {
        let spacing = if first { first_space } else { 3.0 };
        if paragraph.trim().is_empty() {
            lines.push(StyledLine {
                text: String::new(),
                style,
                space_before: spacing,
            });
        } else {
            push_wrapped(lines, paragraph, style, width, spacing);
        }
        first = false;
    }
}

fn push_wrapped(
    lines: &mut Vec<StyledLine>,
    text: &str,
    style: LineStyle,
    width: usize,
    first_space: f32,
) {
    for (index, line) in wrap_text(text, width).into_iter().enumerate() {
        lines.push(StyledLine {
            text: line,
            style,
            space_before: if index == 0 { first_space } else { 0.0 },
        });
    }
}

fn wrap_text(text: &str, max_width: usize) -> Vec<String> {
    let cleaned = text.replace('\t', "    ");
    if cleaned.is_empty() {
        return vec![String::new()];
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_width = 0;
    for word in cleaned.split_whitespace() {
        let word_width = display_width(word);
        if word_width > max_width {
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
            }
            let mut chunk = String::new();
            let mut chunk_width = 0;
            for character in word.chars() {
                let width = character_width(character);
                if chunk_width + width > max_width && !chunk.is_empty() {
                    lines.push(std::mem::take(&mut chunk));
                    chunk_width = 0;
                }
                chunk.push(character);
                chunk_width += width;
            }
            current = chunk;
            current_width = chunk_width;
            continue;
        }
        let separator = usize::from(!current.is_empty());
        if current_width + separator + word_width > max_width {
            lines.push(std::mem::take(&mut current));
            current_width = 0;
        }
        if !current.is_empty() {
            current.push(' ');
            current_width += 1;
        }
        current.push_str(word);
        current_width += word_width;
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

fn display_width(text: &str) -> usize {
    text.chars().map(character_width).sum()
}

fn character_width(character: char) -> usize {
    if character.is_ascii() {
        1
    } else {
        2
    }
}

fn paginate(lines: &[StyledLine]) -> Vec<Vec<StyledLine>> {
    let usable = PAGE_HEIGHT_PT - TOP - BOTTOM;
    let mut pages = vec![Vec::new()];
    let mut used = 0.0;
    for line in lines {
        let height = line_height(line.style) + line.space_before;
        let keep_with_next = matches!(line.style, LineStyle::User | LineStyle::Claude);
        let required = height + if keep_with_next { 18.0 } else { 0.0 };
        if used + required > usable && !pages.last().is_some_and(Vec::is_empty) {
            pages.push(Vec::new());
            used = 0.0;
        }
        pages.last_mut().unwrap().push(line.clone());
        used += height;
    }
    pages
}

fn render_page(
    font: &PdfFontHandle,
    title: &str,
    lines: &[StyledLine],
    page_number: usize,
    total_pages: usize,
) -> PdfPage {
    let mut ops = Vec::new();
    push_text_op(
        &mut ops,
        font,
        "CLAUDE SESSION EXPORT",
        8.0,
        LEFT,
        PAGE_HEIGHT_PT - 28.0,
        rgb(0.43, 0.40, 0.35),
    );
    if page_number > 1 {
        let running_title = wrap_text(title, 74).into_iter().next().unwrap_or_default();
        push_text_op(
            &mut ops,
            font,
            &running_title,
            8.0,
            LEFT,
            PAGE_HEIGHT_PT - 41.0,
            rgb(0.25, 0.24, 0.22),
        );
    }
    let mut y = PAGE_HEIGHT_PT - TOP;
    for line in lines {
        y -= line.space_before;
        let (size, color, indent) = style_properties(line.style);
        push_text_op(&mut ops, font, &line.text, size, LEFT + indent, y, color);
        y -= line_height(line.style);
    }
    push_text_op(
        &mut ops,
        font,
        &format!("Page {page_number} of {total_pages}"),
        8.0,
        PAGE_WIDTH_PT - RIGHT - 62.0,
        24.0,
        rgb(0.43, 0.40, 0.35),
    );
    PdfPage::new(Mm(210.0), Mm(297.0), ops)
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
    // `SetTextCursor` serializes to a relative PDF text move. A fresh text
    // section gives every line a stable page-relative origin instead of letting
    // successive absolute-looking coordinates accumulate off the page.
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

fn style_properties(style: LineStyle) -> (f32, Color, f32) {
    match style {
        LineStyle::Title => (22.0, rgb(0.14, 0.13, 0.12), 0.0),
        LineStyle::Metadata => (9.0, rgb(0.38, 0.36, 0.32), 0.0),
        LineStyle::User => (11.0, rgb(0.60, 0.28, 0.12), 0.0),
        LineStyle::Claude => (11.0, rgb(0.18, 0.43, 0.29), 0.0),
        LineStyle::Body => (10.0, rgb(0.12, 0.12, 0.11), 0.0),
        LineStyle::Thinking => (9.0, rgb(0.38, 0.36, 0.34), 12.0),
        LineStyle::Tool => (8.5, rgb(0.35, 0.28, 0.20), 12.0),
        LineStyle::Reference => (8.5, rgb(0.20, 0.35, 0.52), 12.0),
    }
}

fn line_height(style: LineStyle) -> f32 {
    match style {
        LineStyle::Title => 27.0,
        LineStyle::Metadata => 12.0,
        LineStyle::User | LineStyle::Claude => 15.0,
        LineStyle::Body => 14.0,
        LineStyle::Thinking | LineStyle::Tool | LineStyle::Reference => 12.0,
    }
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
    use super::{render_pdf, wrap_text, PdfTranscript};
    use crate::models::ChatExportMessage;

    fn fixture() -> (String, Vec<ChatExportMessage>) {
        let title = "MSAccess vs SQL data - PDF layout fixture".to_string();
        let messages = vec![
            ChatExportMessage::plain(
                "user",
                "Please compare the two approaches and preserve a long identifier: 5121dabb-5e27-4f85-ad51-812569de176a. This paragraph tests wrapping without clipping at the right margin."
                    .to_string(),
                Some("2026-08-12 12:00".to_string()),
            ),
            ChatExportMessage::plain(
                "claude",
                "The PDF export is generated locally.\n\nIt supports multiple paragraphs, automatic page breaks, accented text such as café and résumé, and stable page numbering."
                    .repeat(18),
                None,
            ),
        ];
        (title, messages)
    }

    #[test]
    fn wraps_long_unbroken_tokens() {
        let wrapped = wrap_text(&"x".repeat(25), 10);
        assert_eq!(
            wrapped.iter().map(String::len).collect::<Vec<_>>(),
            [10, 10, 5]
        );
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
}
