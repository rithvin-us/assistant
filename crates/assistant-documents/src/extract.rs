//! Deterministic PDF extraction.
//!
//! The pipeline calls into this module first, before anything expensive. It
//! wraps `pdf-extract` (which itself wraps `lopdf`) so metadata and per-page
//! text arrive together and page boundaries are preserved.
//!
//! Nothing here calls a language model. A page whose native extraction
//! returns too little is flagged with `needs_ocr = true`; deciding whether to
//! actually run OCR is the pipeline's job, not this file's.

use crate::{ExtractedDocument, ExtractedPage, MAX_PAGE_CHARS};

/// Extracts page-aware text from a PDF blob.
///
/// The return value has `pages.len() == page_count`. A page that returned
/// nothing usable still appears in the list with `text = ""` and `needs_ocr =
/// true`, so downstream code has one page per physical page to reason about.
pub fn extract_pdf(bytes: &[u8]) -> Result<ExtractedDocument, ExtractError> {
    let raw_pages = pdf_extract::extract_text_from_mem_by_pages(bytes)
        .map_err(|error| ExtractError::Failed(format!("pdf-extract: {error}")))?;

    let page_count = raw_pages.len() as u32;
    let pages: Vec<ExtractedPage> = raw_pages
        .into_iter()
        .enumerate()
        .map(|(idx, raw)| {
            let text = normalise_page_text(&raw);
            let text = truncate_chars(&text, MAX_PAGE_CHARS);
            let needs_ocr = page_needs_ocr(&text);
            ExtractedPage {
                page_number: (idx as u32) + 1,
                text,
                needs_ocr,
            }
        })
        .collect();

    Ok(ExtractedDocument { page_count, pages })
}

/// Whether a page's native-text output is thin enough that OCR should run.
///
/// The rule is deliberately simple: a page with fewer than
/// `OCR_THRESHOLD_CHARS` printable characters is treated as scanned. Real
/// scanned pages return zero or near-zero from `pdf-extract`; text-only
/// pages easily clear the threshold.
pub const OCR_THRESHOLD_CHARS: usize = 40;

pub fn page_needs_ocr(text: &str) -> bool {
    text.chars().filter(|c| !c.is_whitespace()).count() < OCR_THRESHOLD_CHARS
}

fn normalise_page_text(raw: &str) -> String {
    // pdf-extract sometimes emits form-feeds and stray control characters
    // that would make the search snippet ugly. Strip them; keep everything a
    // human might legitimately have typed, whitespace included.
    let mut cleaned = String::with_capacity(raw.len());
    for c in raw.chars() {
        if c == '\u{000C}' {
            cleaned.push('\n');
        } else if c.is_control() && c != '\n' && c != '\t' && c != '\r' {
            continue;
        } else {
            cleaned.push(c);
        }
    }
    // Collapse runs of blank lines that `pdf-extract` sometimes produces from
    // heavily formatted layouts.
    let mut out = String::with_capacity(cleaned.len());
    let mut blank_run = 0;
    for line in cleaned.split('\n') {
        if line.trim().is_empty() {
            blank_run += 1;
            if blank_run <= 1 {
                out.push('\n');
            }
        } else {
            blank_run = 0;
            out.push_str(line);
            out.push('\n');
        }
    }
    out.trim().to_string()
}

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push('\u{2026}');
    out
}

#[derive(Debug, thiserror::Error)]
pub enum ExtractError {
    #[error("PDF extraction failed: {0}")]
    Failed(String),
    #[error("unsupported document type: {0}")]
    UnsupportedType(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_needs_ocr_flags_empty_and_near_empty_pages() {
        assert!(page_needs_ocr(""));
        assert!(page_needs_ocr("     \n\n"));
        assert!(page_needs_ocr("shortish"));
        assert!(!page_needs_ocr(
            "This page has plenty of native text so OCR is not required at all."
        ));
    }
}
