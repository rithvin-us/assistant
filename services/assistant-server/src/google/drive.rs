//! Google Drive, normalised and read-only.
//!
//! Only search, listing, metadata and a deliberately narrow "read a small text
//! file" path exist here. There is no delete, move, rename or share method:
//! those are consequential operations this milestone has no need for, and the
//! scope requested does not grant them.
//!
//! Nothing here is mirrored into Postgres. Drive is the user's storage, not
//! this application's data to keep; metadata is fetched on demand and cached on
//! the device. See ADR-0033.

use assistant_protocol::{DriveFile, DriveFileContent};
use assistant_tools::{DriveProvider, ToolError};
use async_trait::async_trait;
use serde::Deserialize;
use time::OffsetDateTime;
use uuid::Uuid;

use super::{GoogleClient, api_error, url_encode};

const API_BASE: &str = "https://www.googleapis.com/drive/v3";

/// The Drive MIME type for a folder.
const FOLDER_MIME: &str = "application/vnd.google-apps.folder";

/// Largest file this milestone will pull into memory, in bytes.
///
/// 512 KiB is far more than any plain-text document a student keeps and far
/// less than a video. The point of the ceiling is that a request can never turn
/// into an unbounded download; see ADR-0033.
pub const MAX_INLINE_BYTES: u64 = 512 * 1024;

/// How much text is returned once a file is accepted.
///
/// A file may be under the byte ceiling and still be longer than anything worth
/// putting on a phone screen, so the body is cut here and `truncated` says so.
pub const MAX_INLINE_CHARS: usize = 40_000;

/// The fields asked of Drive. Requesting explicitly keeps the response small
/// and means a new Drive field never silently starts arriving.
const FILE_FIELDS: &str = "id,name,mimeType,size,modifiedTime,webViewLink,parents";

/// MIME types that can honestly be rendered as text.
///
/// PDFs are deliberately absent. A PDF is discoverable through search and its
/// metadata is returned, but extracting its text needs OCR and page-level
/// handling that belongs to the later document-intelligence milestone. Claiming
/// to have read one here would be the kind of confident wrong answer this
/// project exists to avoid.
fn is_readable_text(mime: &str) -> bool {
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

/// Google's own editor formats, which have no bytes to download and must be
/// exported instead.
fn export_mime(mime: &str) -> Option<&'static str> {
    match mime {
        "application/vnd.google-apps.document" => Some("text/plain"),
        "application/vnd.google-apps.spreadsheet" => Some("text/csv"),
        _ => None,
    }
}

#[derive(Debug, Deserialize)]
struct RawFile {
    id: String,
    name: Option<String>,
    #[serde(rename = "mimeType")]
    mime_type: Option<String>,
    /// Drive sends size as a string, and omits it for native editor documents
    /// and folders.
    size: Option<String>,
    #[serde(rename = "modifiedTime")]
    modified_time: Option<String>,
    #[serde(rename = "webViewLink")]
    web_view_link: Option<String>,
    parents: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct FileListResponse {
    files: Option<Vec<RawFile>>,
}

fn normalize_file(raw: RawFile, account_id: Uuid) -> DriveFile {
    let mime = raw
        .mime_type
        .unwrap_or_else(|| "application/octet-stream".into());
    DriveFile {
        external_id: raw.id,
        account_id,
        name: raw.name.unwrap_or_else(|| "Untitled".into()),
        is_folder: mime == FOLDER_MIME,
        mime_type: mime,
        size_bytes: raw.size.and_then(|s| s.parse::<u64>().ok()),
        modified_at: raw.modified_time.and_then(|t| {
            OffsetDateTime::parse(&t, &time::format_description::well_known::Rfc3339).ok()
        }),
        web_view_link: raw.web_view_link,
        parents: raw.parents.unwrap_or_default(),
    }
}

/// Escapes a value for use inside a Drive `q` string literal.
///
/// Drive's query language delimits literals with single quotes, so an
/// unescaped apostrophe in a file name would terminate the literal and change
/// the meaning of the query. Backslash first, then quote, or the escapes escape
/// each other.
fn escape_query_literal(input: &str) -> String {
    input.replace('\\', "\\\\").replace('\'', "\\'")
}

impl GoogleClient {
    /// Runs one `files.list` call and normalises the result.
    async fn drive_list(
        &self,
        token: &str,
        account_id: Uuid,
        q: &str,
        limit: u32,
    ) -> Result<Vec<DriveFile>, ToolError> {
        let url = format!(
            "{API_BASE}/files?q={}&pageSize={}&fields={}&orderBy=modifiedTime%20desc\
             &supportsAllDrives=true&includeItemsFromAllDrives=true",
            url_encode(q),
            limit.clamp(1, 100),
            url_encode(&format!("files({FILE_FIELDS})"))
        );

        let resp = self
            .http
            .get(&url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|_| ToolError::Failed("Could not reach Google Drive.".into()))?;

        if !resp.status().is_success() {
            return Err(api_error("Drive", resp.status()));
        }

        let body: FileListResponse = resp
            .json()
            .await
            .map_err(|_| ToolError::Failed("Drive returned an unreadable response.".into()))?;

        Ok(body
            .files
            .unwrap_or_default()
            .into_iter()
            .map(|raw| normalize_file(raw, account_id))
            .collect())
    }
}

#[async_trait]
impl DriveProvider for GoogleClient {
    async fn search(
        &self,
        account_id: Uuid,
        user_id: Uuid,
        query: &str,
        mime_type: Option<&str>,
        limit: u32,
    ) -> Result<Vec<DriveFile>, ToolError> {
        let token = self.get_access_token(user_id, account_id).await?;

        let mut q = String::from("trashed = false");
        if !query.trim().is_empty() {
            q.push_str(&format!(
                " and name contains '{}'",
                escape_query_literal(query.trim())
            ));
        }
        if let Some(mime) = mime_type.filter(|m| !m.trim().is_empty()) {
            q.push_str(&format!(
                " and mimeType = '{}'",
                escape_query_literal(mime.trim())
            ));
        }

        self.drive_list(&token, account_id, &q, limit).await
    }

    async fn list(
        &self,
        account_id: Uuid,
        user_id: Uuid,
        folder_id: Option<&str>,
        limit: u32,
    ) -> Result<Vec<DriveFile>, ToolError> {
        let token = self.get_access_token(user_id, account_id).await?;

        let parent = folder_id.filter(|f| !f.trim().is_empty()).unwrap_or("root");
        let q = format!(
            "trashed = false and '{}' in parents",
            escape_query_literal(parent)
        );

        self.drive_list(&token, account_id, &q, limit).await
    }

    async fn metadata(
        &self,
        account_id: Uuid,
        user_id: Uuid,
        file_id: &str,
    ) -> Result<DriveFile, ToolError> {
        let token = self.get_access_token(user_id, account_id).await?;

        let url = format!(
            "{API_BASE}/files/{}?fields={}&supportsAllDrives=true",
            url_encode(file_id),
            url_encode(FILE_FIELDS)
        );

        let resp = self
            .http
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|_| ToolError::Failed("Could not reach Google Drive.".into()))?;

        if !resp.status().is_success() {
            return Err(api_error("Drive", resp.status()));
        }

        let raw: RawFile = resp
            .json()
            .await
            .map_err(|_| ToolError::Failed("Drive returned an unreadable response.".into()))?;

        Ok(normalize_file(raw, account_id))
    }

    async fn read_small_file(
        &self,
        account_id: Uuid,
        user_id: Uuid,
        file_id: &str,
    ) -> Result<DriveFileContent, ToolError> {
        // Metadata first, always. The size and type decide whether a download
        // is allowed to happen at all, so asking afterwards would defeat the
        // point of having a limit.
        let meta = self.metadata(account_id, user_id, file_id).await?;

        if meta.is_folder {
            return Err(ToolError::InvalidArguments(
                "That is a folder, not a file.".into(),
            ));
        }

        let token = self.get_access_token(user_id, account_id).await?;

        let url = if let Some(target) = export_mime(&meta.mime_type) {
            // Native Google documents report no size, so the ceiling cannot be
            // checked in advance. The export is capped after the fact by
            // MAX_INLINE_CHARS instead.
            format!(
                "{API_BASE}/files/{}/export?mimeType={}",
                url_encode(file_id),
                url_encode(target)
            )
        } else {
            if !is_readable_text(&meta.mime_type) {
                return Err(ToolError::InvalidArguments(format!(
                    "\"{}\" is a {} file, which cannot be read as text here.",
                    meta.name, meta.mime_type
                )));
            }
            if let Some(size) = meta.size_bytes {
                if size > MAX_INLINE_BYTES {
                    return Err(ToolError::InvalidArguments(format!(
                        "\"{}\" is too large to inspect here ({} KB; the limit is {} KB).",
                        meta.name,
                        size / 1024,
                        MAX_INLINE_BYTES / 1024
                    )));
                }
            } else {
                // No size and not an exportable Google document: refuse rather
                // than start a download of unknown length.
                return Err(ToolError::InvalidArguments(format!(
                    "\"{}\" does not report a size, so it is not safe to read here.",
                    meta.name
                )));
            }
            format!("{API_BASE}/files/{}?alt=media", url_encode(file_id))
        };

        let resp = self
            .http
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|_| ToolError::Failed("Could not reach Google Drive.".into()))?;

        if !resp.status().is_success() {
            return Err(api_error("Drive", resp.status()));
        }

        let body = resp
            .text()
            .await
            .map_err(|_| ToolError::Failed("Drive returned an unreadable file.".into()))?;

        let truncated = body.chars().count() > MAX_INLINE_CHARS;
        let text = if truncated {
            body.chars().take(MAX_INLINE_CHARS).collect()
        } else {
            body
        };

        Ok(DriveFileContent {
            external_id: meta.external_id,
            account_id,
            name: meta.name,
            mime_type: meta.mime_type,
            text,
            truncated,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(json: &str) -> RawFile {
        serde_json::from_str(json).expect("payload parses")
    }

    #[test]
    fn file_normalizes_size_from_string() {
        let f = normalize_file(
            raw(
                r#"{"id":"f1","name":"notes.txt","mimeType":"text/plain","size":"2048",
                    "modifiedTime":"2026-09-01T12:00:00Z",
                    "webViewLink":"https://drive.google.com/file/d/f1"}"#,
            ),
            Uuid::nil(),
        );
        assert_eq!(f.name, "notes.txt");
        assert_eq!(f.size_bytes, Some(2048));
        assert!(!f.is_folder);
        assert!(f.modified_at.is_some());
    }

    #[test]
    fn folder_is_flagged_by_mime_type() {
        let f = normalize_file(
            raw(
                r#"{"id":"d1","name":"Semester 5","mimeType":"application/vnd.google-apps.folder"}"#,
            ),
            Uuid::nil(),
        );
        assert!(f.is_folder);
        assert!(f.size_bytes.is_none());
    }

    #[test]
    fn missing_size_stays_absent_rather_than_zero() {
        // A Google Doc reports no size. Defaulting to 0 would make it look
        // like an empty file and would pass a size check it never took.
        let f = normalize_file(
            raw(
                r#"{"id":"g1","name":"Lab Record","mimeType":"application/vnd.google-apps.document"}"#,
            ),
            Uuid::nil(),
        );
        assert!(f.size_bytes.is_none());
    }

    #[test]
    fn pdf_is_discoverable_but_not_readable_as_text() {
        assert!(!is_readable_text("application/pdf"));
        assert!(export_mime("application/pdf").is_none());
        // Metadata still normalises, so a PDF can be found and opened in Drive.
        let f = normalize_file(
            raw(r#"{"id":"p1","name":"Syllabus.pdf","mimeType":"application/pdf","size":"90000"}"#),
            Uuid::nil(),
        );
        assert_eq!(f.mime_type, "application/pdf");
    }

    #[test]
    fn text_types_are_readable_and_binary_types_are_not() {
        assert!(is_readable_text("text/plain"));
        assert!(is_readable_text("text/markdown"));
        assert!(is_readable_text("application/json"));
        assert!(!is_readable_text("image/png"));
        assert!(!is_readable_text("application/zip"));
    }

    #[test]
    fn google_editor_types_export_to_text() {
        assert_eq!(
            export_mime("application/vnd.google-apps.document"),
            Some("text/plain")
        );
        assert_eq!(
            export_mime("application/vnd.google-apps.spreadsheet"),
            Some("text/csv")
        );
        assert!(export_mime("application/vnd.google-apps.presentation").is_none());
    }

    #[test]
    fn query_literals_are_escaped() {
        // Without escaping, this apostrophe would close the literal and the
        // rest would be parsed as query syntax.
        assert_eq!(escape_query_literal("Rithvin's notes"), "Rithvin\\'s notes");
        assert_eq!(escape_query_literal(r"a\b"), r"a\\b");
    }

    #[test]
    fn size_ceiling_is_below_a_megabyte() {
        assert!(MAX_INLINE_BYTES < 1024 * 1024);
    }
}
