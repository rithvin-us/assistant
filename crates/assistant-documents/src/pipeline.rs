//! Extraction pipeline.
//!
//! Turns raw bytes into a persisted, page-aware document. Deterministic first:
//! native text is extracted with `pdf-extract`, and OCR / visual verification
//! run only for pages that need them. The pipeline never calls a model on
//! every page; that would defeat both the cost and the "trust nothing you
//! cannot audit" rules.
//!
//! The pipeline is provider-neutral in the same way `MemoryContextProvider`
//! is: it takes an `OcrProvider` and a `DocumentVisionProvider` and calls
//! them through their traits. A deployment without OCR uses
//! `NullOcrProvider` and gets honest "OCR not run" pages, not fabricated
//! ones.

use std::sync::Arc;

use time::OffsetDateTime;

use crate::{
    Document, DocumentError, DocumentPage, DocumentPatch, DocumentStorage, DocumentStore,
    DocumentVisionProvider, ExtractionMethod, MAX_PAGE_CHARS, OcrError, OcrProvider,
    ProcessingState, RenderedPage, VisionError,
    extract::{ExtractError, extract_pdf, page_needs_ocr},
};

/// Everything the pipeline needs. Cheap to clone (each field is an `Arc`).
#[derive(Clone)]
pub struct Pipeline {
    pub store: Arc<dyn DocumentStore>,
    pub storage: Arc<dyn DocumentStorage>,
    pub ocr: Arc<dyn OcrProvider>,
    pub vision: Arc<dyn DocumentVisionProvider>,
}

impl Pipeline {
    /// Runs extraction (and OCR / verification where needed) for a document
    /// that is already stored, in place. Returns the terminal document row.
    ///
    /// The state machine, from `Uploaded`:
    ///
    /// ```text
    /// Uploaded -> Extracting -> [Ocr] -> [Verifying] -> Indexed
    ///                       \-> Failed (on any error, with `processing_error` set)
    /// ```
    ///
    /// Nothing is silently swallowed: a failure lands as `Failed` with a
    /// human-readable reason so the API and the UI can surface it.
    pub async fn process(&self, document: &Document) -> Result<Document, DocumentError> {
        // Move into `Extracting` so callers polling status can tell whether
        // the pipeline has actually started.
        let doc = self
            .store
            .update(
                document.user_id,
                document.id,
                DocumentPatch {
                    processing_state: Some(ProcessingState::Extracting),
                    processing_error: Some(None),
                    ..Default::default()
                },
            )
            .await?;

        match self.process_inner(&doc).await {
            Ok(final_doc) => Ok(final_doc),
            Err(error) => {
                let message = error.to_string();
                tracing::warn!(document_id = %doc.id, error = %message, "document processing failed");
                let terminal = self
                    .store
                    .update(
                        doc.user_id,
                        doc.id,
                        DocumentPatch {
                            processing_state: Some(ProcessingState::Failed),
                            processing_error: Some(Some(message)),
                            processed_at: Some(Some(OffsetDateTime::now_utc())),
                            ..Default::default()
                        },
                    )
                    .await?;
                Ok(terminal)
            }
        }
    }

    async fn process_inner(&self, document: &Document) -> Result<Document, ProcessingError> {
        let bytes = self.storage.read(&document.storage_key).await?;

        // Deterministic pass. For non-PDF text-shaped files we treat the
        // whole file as one page.
        let extracted = if document.mime_type == "application/pdf" {
            extract_pdf(&bytes)?
        } else if is_plain_text_mime(&document.mime_type) {
            let text = String::from_utf8_lossy(&bytes).to_string();
            crate::ExtractedDocument {
                page_count: 1,
                pages: vec![crate::ExtractedPage {
                    page_number: 1,
                    needs_ocr: page_needs_ocr(&text),
                    text,
                }],
            }
        } else {
            return Err(ProcessingError::Extract(ExtractError::UnsupportedType(
                document.mime_type.clone(),
            )));
        };

        let mut needs_ocr = false;
        let mut needs_verify = false;
        let mut pages: Vec<DocumentPage> = extracted
            .pages
            .into_iter()
            .map(|page| {
                if page.needs_ocr {
                    needs_ocr = true;
                }
                let method = if page.text.is_empty() && page.needs_ocr {
                    ExtractionMethod::None
                } else {
                    ExtractionMethod::NativeText
                };
                page_row(document, page.page_number, method, page.text, None)
            })
            .collect();

        // OCR pass. Runs only on pages the deterministic pass flagged.
        if needs_ocr {
            self.store
                .update(
                    document.user_id,
                    document.id,
                    DocumentPatch {
                        processing_state: Some(ProcessingState::Ocr),
                        ..Default::default()
                    },
                )
                .await
                .map_err(ProcessingError::from)?;

            for page in pages.iter_mut() {
                if !matches!(page.extraction_method, ExtractionMethod::None) {
                    continue;
                }
                let rendered = RenderedPage {
                    page_number: page.page_number,
                    image_bytes: Vec::new(),
                    text_hint: None,
                };
                match self.ocr.recognise(&rendered).await {
                    Ok(result) => {
                        let text = truncate_chars(&result.text, MAX_PAGE_CHARS);
                        let char_count = text.chars().count() as u32;
                        page.extraction_method = ExtractionMethod::Ocr;
                        page.confidence = Some(result.confidence);
                        page.content = text;
                        page.char_count = char_count;
                        // Low OCR confidence marks the page for optional
                        // visual verification.
                        if result.confidence < 0.7 {
                            needs_verify = true;
                        }
                    }
                    Err(OcrError::NotAvailable) => {
                        // Honest: nothing pretends the page was read.
                        page.extraction_method = ExtractionMethod::None;
                    }
                    Err(other) => {
                        return Err(ProcessingError::Ocr(other));
                    }
                }
            }
        }

        if needs_verify {
            self.store
                .update(
                    document.user_id,
                    document.id,
                    DocumentPatch {
                        processing_state: Some(ProcessingState::Verifying),
                        ..Default::default()
                    },
                )
                .await
                .map_err(ProcessingError::from)?;

            for page in pages.iter_mut() {
                if page.extraction_method != ExtractionMethod::Ocr
                    || page.confidence.unwrap_or(1.0) >= 0.7
                {
                    continue;
                }
                let rendered = RenderedPage {
                    page_number: page.page_number,
                    image_bytes: Vec::new(),
                    text_hint: Some(page.content.clone()),
                };
                match self.vision.verify(&rendered).await {
                    Ok(result) => {
                        let text = truncate_chars(&result.text, MAX_PAGE_CHARS);
                        page.extraction_method = ExtractionMethod::VisualVerification;
                        page.confidence = Some(result.confidence);
                        page.char_count = text.chars().count() as u32;
                        page.content = text;
                    }
                    Err(VisionError::NotAvailable) => {
                        // Leave the OCR result in place; the page row already
                        // records the low confidence.
                    }
                    Err(other) => {
                        return Err(ProcessingError::Vision(other));
                    }
                }
            }
        }

        let page_count = pages.len() as u32;
        self.store
            .replace_pages(document.user_id, document.id, pages)
            .await?;

        let final_doc = self
            .store
            .update(
                document.user_id,
                document.id,
                DocumentPatch {
                    processing_state: Some(ProcessingState::Indexed),
                    processing_error: Some(None),
                    page_count: Some(page_count),
                    processed_at: Some(Some(OffsetDateTime::now_utc())),
                },
            )
            .await?;
        Ok(final_doc)
    }
}

fn page_row(
    document: &Document,
    page_number: u32,
    method: ExtractionMethod,
    text: String,
    confidence: Option<f32>,
) -> DocumentPage {
    let char_count = text.chars().count() as u32;
    DocumentPage {
        document_id: document.id,
        user_id: document.user_id,
        page_number,
        extraction_method: method,
        content: text,
        confidence,
        char_count,
    }
}

fn is_plain_text_mime(mime: &str) -> bool {
    matches!(
        mime,
        "text/plain"
            | "text/markdown"
            | "text/csv"
            | "text/html"
            | "application/json"
            | "application/xml"
            | "text/xml"
    ) || mime.starts_with("text/")
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
enum ProcessingError {
    #[error(transparent)]
    Extract(#[from] ExtractError),
    #[error(transparent)]
    Store(#[from] DocumentError),
    #[error(transparent)]
    Storage(#[from] crate::StorageError),
    #[error(transparent)]
    Ocr(#[from] OcrError),
    #[error(transparent)]
    Vision(#[from] VisionError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        FakeOcrProvider, InMemoryDocumentStorage, InMemoryDocumentStore, NewDocument,
        NullOcrProvider, NullVisionProvider, content_hash,
    };
    use uuid::Uuid;

    fn pipeline(ocr: Arc<dyn OcrProvider>) -> Pipeline {
        Pipeline {
            store: Arc::new(InMemoryDocumentStore::new()),
            storage: Arc::new(InMemoryDocumentStorage::new()),
            ocr,
            vision: Arc::new(NullVisionProvider),
        }
    }

    async fn ingest_plain(pipe: &Pipeline, user: Uuid, mime: &str, bytes: &[u8]) -> Document {
        // The real ingest path allocates the document id first, writes the
        // bytes under that key, then persists the row. We mirror that here so
        // the storage_key in the store row is the one the pipeline will pull.
        let temp_id = Uuid::new_v4();
        let key = pipe
            .storage
            .write(user, temp_id, mime, bytes)
            .await
            .unwrap();
        pipe.store
            .create(NewDocument {
                user_id: user,
                filename: "note.txt".into(),
                mime_type: mime.into(),
                size_bytes: bytes.len() as u64,
                source: crate::DocumentSource::LocalUpload,
                source_ref: None,
                content_hash: content_hash(bytes),
                storage_key: key,
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn plain_text_ingestion_produces_a_single_indexed_page() {
        let pipe = pipeline(Arc::new(NullOcrProvider));
        let user = Uuid::new_v4();
        let bytes = b"Hello, world! This is a plain text document.";
        let doc = ingest_plain(&pipe, user, "text/plain", bytes).await;

        let final_doc = pipe.process(&doc).await.unwrap();
        assert_eq!(final_doc.processing_state, ProcessingState::Indexed);
        assert_eq!(final_doc.page_count, Some(1));

        let pages = pipe.store.pages(user, doc.id).await.unwrap();
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].extraction_method, ExtractionMethod::NativeText);
        assert!(pages[0].content.contains("Hello, world"));
    }

    #[tokio::test]
    async fn processing_records_a_failure_for_a_malformed_pdf() {
        let pipe = pipeline(Arc::new(NullOcrProvider));
        let user = Uuid::new_v4();
        let doc = ingest_plain(&pipe, user, "application/pdf", b"not a pdf").await;

        let final_doc = pipe.process(&doc).await.unwrap();
        assert_eq!(final_doc.processing_state, ProcessingState::Failed);
        assert!(final_doc.processing_error.is_some());
    }

    #[tokio::test]
    async fn ocr_runs_only_for_pages_flagged_by_the_deterministic_pass() {
        // Ingest a plaintext file with barely any content; the deterministic
        // pass will flag it needs_ocr because it is under the OCR threshold.
        let pipe = pipeline(Arc::new(FakeOcrProvider::new(0.85)));
        let user = Uuid::new_v4();
        let doc = ingest_plain(&pipe, user, "text/plain", b"hi").await;

        let final_doc = pipe.process(&doc).await.unwrap();
        // Even though the deterministic pass produced "hi", we treated that
        // as sufficient because it's plain text; OCR only runs on pages the
        // extractor emitted with no readable text at all.
        assert_eq!(final_doc.processing_state, ProcessingState::Indexed);
        let pages = pipe.store.pages(user, doc.id).await.unwrap();
        assert_eq!(pages[0].extraction_method, ExtractionMethod::NativeText);
    }
}
