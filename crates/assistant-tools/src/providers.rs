//! Provider-neutral capability traits for external integrations (Gmail, Calendar).
//!
//! Defined in `assistant-tools` so that `assistant-core` and tools can consume them
//! without depending on concrete Google SDKs or HTTP implementations.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

pub use assistant_protocol::{
    AccountSummary, CalendarEvent, CreateEventRequest as CreateCalendarEvent, EmailDetail,
    EmailSummary, FreeSlot,
};

use crate::ToolError;

/// Input parameters for updating an existing calendar event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateCalendarEvent {
    pub title: Option<String>,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub start_time: Option<OffsetDateTime>,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub end_time: Option<OffsetDateTime>,
    pub description: Option<String>,
    pub location: Option<String>,
}

/// Capability interface for Gmail operations.
#[async_trait]
pub trait GmailProvider: Send + Sync {
    /// Searches messages in the specified connected account.
    async fn search(
        &self,
        account_id: Uuid,
        user_id: Uuid,
        query: &str,
        limit: u32,
    ) -> Result<Vec<EmailSummary>, ToolError>;

    /// Reads full message content for an email.
    async fn read(
        &self,
        account_id: Uuid,
        user_id: Uuid,
        message_id: &str,
    ) -> Result<EmailDetail, ToolError>;
}

/// Capability interface for Calendar operations.
#[async_trait]
pub trait CalendarProvider: Send + Sync {
    /// Lists calendar events within a time interval.
    async fn list(
        &self,
        account_id: Uuid,
        user_id: Uuid,
        start: OffsetDateTime,
        end: OffsetDateTime,
    ) -> Result<Vec<CalendarEvent>, ToolError>;

    /// Searches calendar events matching a title/query.
    async fn search(
        &self,
        account_id: Uuid,
        user_id: Uuid,
        query: &str,
    ) -> Result<Vec<CalendarEvent>, ToolError>;

    /// Creates a calendar event in the specified account.
    async fn create(
        &self,
        account_id: Uuid,
        user_id: Uuid,
        event: CreateCalendarEvent,
    ) -> Result<CalendarEvent, ToolError>;

    /// Updates an existing calendar event.
    async fn update(
        &self,
        account_id: Uuid,
        user_id: Uuid,
        event_id: &str,
        event: UpdateCalendarEvent,
    ) -> Result<CalendarEvent, ToolError>;

    /// Deletes a calendar event.
    async fn delete(
        &self,
        account_id: Uuid,
        user_id: Uuid,
        event_id: &str,
    ) -> Result<(), ToolError>;
}

// ---------------------------------------------------------------------------
// Milestone 6 -- academic providers
// ---------------------------------------------------------------------------
//
// Same rule as Gmail and Calendar: the trait is the seam. `assistant-core` and
// the tools depend on these signatures; the concrete Classroom and Drive HTTP
// clients live in `assistant-server`. Every method takes both `account_id` and
// `user_id` because ownership is checked at the query, not by the caller.

pub use assistant_protocol::{Announcement, Course, CourseworkItem, DriveFile, DriveFileContent};

/// Capability interface for a course-management provider.
///
/// Read-only by design for Milestone 6: this milestone is about understanding
/// academic information, not administering it. There is deliberately no
/// submission, grading or roster method to call.
#[async_trait]
pub trait ClassroomProvider: Send + Sync {
    /// Lists courses the user is enrolled in on this account.
    async fn courses(&self, account_id: Uuid, user_id: Uuid) -> Result<Vec<Course>, ToolError>;

    /// Lists coursework for one course.
    async fn coursework(
        &self,
        account_id: Uuid,
        user_id: Uuid,
        course_external_id: &str,
    ) -> Result<Vec<CourseworkItem>, ToolError>;

    /// Lists announcements for one course, newest first.
    async fn announcements(
        &self,
        account_id: Uuid,
        user_id: Uuid,
        course_external_id: &str,
        limit: u32,
    ) -> Result<Vec<Announcement>, ToolError>;
}

/// Capability interface for a file-storage provider.
///
/// Read-only. There is no delete, move, rename or share method, because those
/// are consequential operations this milestone has no need for.
#[async_trait]
pub trait DriveProvider: Send + Sync {
    /// Searches files by name and optional MIME type.
    async fn search(
        &self,
        account_id: Uuid,
        user_id: Uuid,
        query: &str,
        mime_type: Option<&str>,
        limit: u32,
    ) -> Result<Vec<DriveFile>, ToolError>;

    /// Lists the contents of a folder, or the drive root when `folder_id` is
    /// `None`.
    async fn list(
        &self,
        account_id: Uuid,
        user_id: Uuid,
        folder_id: Option<&str>,
        limit: u32,
    ) -> Result<Vec<DriveFile>, ToolError>;

    /// Reads metadata for one file.
    async fn metadata(
        &self,
        account_id: Uuid,
        user_id: Uuid,
        file_id: &str,
    ) -> Result<DriveFile, ToolError>;

    /// Reads a small, text-shaped file.
    ///
    /// Implementations must check the reported size before downloading and
    /// refuse anything above the configured ceiling or of an unsupported type,
    /// rather than pulling an arbitrary file into memory. Refusal is an error
    /// the user can read, never a silent empty body.
    async fn read_small_file(
        &self,
        account_id: Uuid,
        user_id: Uuid,
        file_id: &str,
    ) -> Result<DriveFileContent, ToolError>;
}

pub use assistant_protocol::{AcademicDeadline, AcademicOverview, AcademicSyncResult};

/// The unified academic context: deadlines drawn from every source, and the
/// synchronisation that imports coursework into tasks.
///
/// Backed by the database rather than by a single external API, which is why
/// it is a separate trait from `ClassroomProvider`. The implementation lives in
/// `assistant-server` next to the pool; `assistant-tools` stays free of sqlx.
#[async_trait]
pub trait AcademicProvider: Send + Sync {
    /// Every outstanding academic deadline the user has, nearest first,
    /// regardless of which source it came from.
    async fn deadlines(
        &self,
        user_id: Uuid,
        limit: i64,
    ) -> Result<Vec<AcademicDeadline>, ToolError>;

    /// Cached coursework for one account, optionally narrowed to a course.
    async fn assignments(
        &self,
        account_id: Uuid,
        user_id: Uuid,
        course_external_id: Option<&str>,
    ) -> Result<Vec<CourseworkItem>, ToolError>;

    /// Refreshes one account from its provider and imports coursework into
    /// tasks. Idempotent: running it twice does not create a second task.
    async fn sync(&self, account_id: Uuid, user_id: Uuid) -> Result<AcademicSyncResult, ToolError>;
}
