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
