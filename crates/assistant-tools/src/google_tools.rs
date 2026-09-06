//! Authoritative Tool implementations for Gmail and Google Calendar.

//!

//! Risk levels are immutable static properties declared in Rust (ADR-0005):

//! - Read operations (`gmail.search`, `gmail.read`, `calendar.list`, `calendar.search`, `calendar.free_slots`): Green

//! - Low-risk modifications (`calendar.create`, `calendar.update`): Yellow

//! - Destructive operations (`calendar.delete`): Orange (Requires explicit human approval!)

use async_trait::async_trait;

use serde_json::json;

use std::sync::Arc;

use time::OffsetDateTime;

use uuid::Uuid;

use crate::{
    RiskLevel, Tool, ToolError, ToolSpec,
    providers::{CalendarProvider, CreateCalendarEvent, GmailProvider, UpdateCalendarEvent},
};

// ==========================================

// 1. Gmail Search Tool

// ==========================================

pub struct GmailSearchTool {
    provider: Arc<dyn GmailProvider>,

    spec: ToolSpec,
}

impl GmailSearchTool {
    pub fn new(provider: Arc<dyn GmailProvider>) -> Self {
        Self {

            provider,

            spec: ToolSpec {

                name: "gmail.search".into(),

                description: "Search emails across a connected Google account using Gmail query syntax (e.g. from:alice is:unread)".into(),

                input_schema: json!({

                    "type": "object",

                    "properties": {

                        "account_id": { "type": "string", "format": "uuid", "description": "Connected Google account UUID" },

                        "query": { "type": "string", "description": "Search query in Gmail syntax" },

                        "limit": { "type": "integer", "minimum": 1, "maximum": 50, "default": 10 }

                    },

                    "required": ["account_id", "query"]

                }),

                output_schema: json!({

                    "type": "array",

                    "items": { "type": "object" }

                }),

                risk: RiskLevel::Green,

                required_scopes: vec!["gmail.readonly".into()],

                timeout_ms: 10_000,

            },

        }
    }
}

#[async_trait]

impl Tool for GmailSearchTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn execute(&self, args: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let account_id_str = args
            .get("account_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::InvalidArguments("missing required string field 'account_id'".into())
            })?;

        let account_id = Uuid::parse_str(account_id_str)
            .map_err(|e| ToolError::InvalidArguments(format!("invalid account_id UUID: {e}")))?;

        let query = args.get("query").and_then(|v| v.as_str()).ok_or_else(|| {
            ToolError::InvalidArguments("missing required string field 'query'".into())
        })?;

        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as u32;

        let user_id = args
            .get("_user_id")
            .and_then(|v| v.as_str())
            .and_then(|s| Uuid::parse_str(s).ok())
            .unwrap_or_default();

        let emails = self
            .provider
            .search(account_id, user_id, query, limit)
            .await?;

        Ok(json!(emails))
    }

    async fn execute_with_user(
        &self,

        user_id: Option<Uuid>,

        mut args: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        if let (Some(uid), Some(obj)) = (user_id, args.as_object_mut()) {
            obj.insert("_user_id".into(), json!(uid.to_string()));
        }

        self.execute(args).await
    }
}

// ==========================================

// 2. Gmail Read Tool

// ==========================================

pub struct GmailReadTool {
    provider: Arc<dyn GmailProvider>,

    spec: ToolSpec,
}

impl GmailReadTool {
    pub fn new(provider: Arc<dyn GmailProvider>) -> Self {
        Self {
            provider,

            spec: ToolSpec {
                name: "gmail.read".into(),

                description: "Read full text content of a single email by message ID".into(),

                input_schema: json!({

                    "type": "object",

                    "properties": {

                        "account_id": { "type": "string", "format": "uuid", "description": "Connected Google account UUID" },

                        "message_id": { "type": "string", "description": "Message identifier" }

                    },

                    "required": ["account_id", "message_id"]

                }),

                output_schema: json!({

                    "type": "object"

                }),

                risk: RiskLevel::Green,

                required_scopes: vec!["gmail.readonly".into()],

                timeout_ms: 10_000,
            },
        }
    }
}

#[async_trait]

impl Tool for GmailReadTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn execute(&self, args: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let account_id_str = args
            .get("account_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::InvalidArguments("missing required string field 'account_id'".into())
            })?;

        let account_id = Uuid::parse_str(account_id_str)
            .map_err(|e| ToolError::InvalidArguments(format!("invalid account_id UUID: {e}")))?;

        let message_id = args
            .get("message_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::InvalidArguments("missing required string field 'message_id'".into())
            })?;

        let user_id = args
            .get("_user_id")
            .and_then(|v| v.as_str())
            .and_then(|s| Uuid::parse_str(s).ok())
            .unwrap_or_default();

        let email = self.provider.read(account_id, user_id, message_id).await?;

        Ok(json!(email))
    }

    async fn execute_with_user(
        &self,

        user_id: Option<Uuid>,

        mut args: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        if let (Some(uid), Some(obj)) = (user_id, args.as_object_mut()) {
            obj.insert("_user_id".into(), json!(uid.to_string()));
        }

        self.execute(args).await
    }
}

// ==========================================

// 3. Calendar List Tool

// ==========================================

pub struct CalendarListTool {
    provider: Arc<dyn CalendarProvider>,

    spec: ToolSpec,
}

impl CalendarListTool {
    pub fn new(provider: Arc<dyn CalendarProvider>) -> Self {
        Self {
            provider,

            spec: ToolSpec {
                name: "calendar.list".into(),

                description: "List events in a Google Calendar between start and end timestamps"
                    .into(),

                input_schema: json!({

                    "type": "object",

                    "properties": {

                        "account_id": { "type": "string", "format": "uuid" },

                        "start_time": { "type": "string", "format": "date-time" },

                        "end_time": { "type": "string", "format": "date-time" }

                    },

                    "required": ["account_id", "start_time", "end_time"]

                }),

                output_schema: json!({

                    "type": "array",

                    "items": { "type": "object" }

                }),

                risk: RiskLevel::Green,

                required_scopes: vec!["calendar.readonly".into()],

                timeout_ms: 10_000,
            },
        }
    }
}

#[async_trait]

impl Tool for CalendarListTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn execute(&self, args: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let account_id_str = args
            .get("account_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::InvalidArguments("missing required string field 'account_id'".into())
            })?;

        let account_id = Uuid::parse_str(account_id_str)
            .map_err(|e| ToolError::InvalidArguments(format!("invalid account_id UUID: {e}")))?;

        let start_str = args
            .get("start_time")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::InvalidArguments("missing required string field 'start_time'".into())
            })?;

        let start =
            OffsetDateTime::parse(start_str, &time::format_description::well_known::Rfc3339)
                .map_err(|e| {
                    ToolError::InvalidArguments(format!("invalid RFC3339 start_time: {e}"))
                })?;

        let end_str = args
            .get("end_time")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::InvalidArguments("missing required string field 'end_time'".into())
            })?;

        let end = OffsetDateTime::parse(end_str, &time::format_description::well_known::Rfc3339)
            .map_err(|e| ToolError::InvalidArguments(format!("invalid RFC3339 end_time: {e}")))?;

        let user_id = args
            .get("_user_id")
            .and_then(|v| v.as_str())
            .and_then(|s| Uuid::parse_str(s).ok())
            .unwrap_or_default();

        let events = self.provider.list(account_id, user_id, start, end).await?;

        Ok(json!(events))
    }

    async fn execute_with_user(
        &self,

        user_id: Option<Uuid>,

        mut args: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        if let (Some(uid), Some(obj)) = (user_id, args.as_object_mut()) {
            obj.insert("_user_id".into(), json!(uid.to_string()));
        }

        self.execute(args).await
    }
}

// ==========================================

// 4. Calendar Search Tool

// ==========================================

pub struct CalendarSearchTool {
    provider: Arc<dyn CalendarProvider>,

    spec: ToolSpec,
}

impl CalendarSearchTool {
    pub fn new(provider: Arc<dyn CalendarProvider>) -> Self {
        Self {
            provider,

            spec: ToolSpec {
                name: "calendar.search".into(),

                description: "Search calendar events matching a title or keyword".into(),

                input_schema: json!({

                    "type": "object",

                    "properties": {

                        "account_id": { "type": "string", "format": "uuid" },

                        "query": { "type": "string" }

                    },

                    "required": ["account_id", "query"]

                }),

                output_schema: json!({

                    "type": "array",

                    "items": { "type": "object" }

                }),

                risk: RiskLevel::Green,

                required_scopes: vec!["calendar.readonly".into()],

                timeout_ms: 10_000,
            },
        }
    }
}

#[async_trait]

impl Tool for CalendarSearchTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn execute(&self, args: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let account_id_str = args
            .get("account_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::InvalidArguments("missing required string field 'account_id'".into())
            })?;

        let account_id = Uuid::parse_str(account_id_str)
            .map_err(|e| ToolError::InvalidArguments(format!("invalid account_id UUID: {e}")))?;

        let query = args.get("query").and_then(|v| v.as_str()).ok_or_else(|| {
            ToolError::InvalidArguments("missing required string field 'query'".into())
        })?;

        let user_id = args
            .get("_user_id")
            .and_then(|v| v.as_str())
            .and_then(|s| Uuid::parse_str(s).ok())
            .unwrap_or_default();

        let events = self.provider.search(account_id, user_id, query).await?;

        Ok(json!(events))
    }

    async fn execute_with_user(
        &self,

        user_id: Option<Uuid>,

        mut args: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        if let (Some(uid), Some(obj)) = (user_id, args.as_object_mut()) {
            obj.insert("_user_id".into(), json!(uid.to_string()));
        }

        self.execute(args).await
    }
}

// ==========================================

// 5. Calendar Create Tool

// ==========================================

pub struct CalendarCreateTool {
    provider: Arc<dyn CalendarProvider>,

    spec: ToolSpec,
}

impl CalendarCreateTool {
    pub fn new(provider: Arc<dyn CalendarProvider>) -> Self {
        Self {
            provider,

            spec: ToolSpec {
                name: "calendar.create".into(),

                description: "Create a new event in Google Calendar".into(),

                input_schema: json!({

                    "type": "object",

                    "properties": {

                        "account_id": { "type": "string", "format": "uuid" },

                        "title": { "type": "string" },

                        "start_time": { "type": "string", "format": "date-time" },

                        "end_time": { "type": "string", "format": "date-time" },

                        "description": { "type": "string" },

                        "location": { "type": "string" }

                    },

                    "required": ["account_id", "title", "start_time", "end_time"]

                }),

                output_schema: json!({ "type": "object" }),

                risk: RiskLevel::Yellow,

                required_scopes: vec!["calendar.events".into()],

                timeout_ms: 10_000,
            },
        }
    }
}

#[async_trait]

impl Tool for CalendarCreateTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn execute(&self, args: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let account_id_str = args
            .get("account_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::InvalidArguments("missing required string field 'account_id'".into())
            })?;

        let account_id = Uuid::parse_str(account_id_str)
            .map_err(|e| ToolError::InvalidArguments(format!("invalid account_id UUID: {e}")))?;

        let title = args.get("title").and_then(|v| v.as_str()).ok_or_else(|| {
            ToolError::InvalidArguments("missing required string field 'title'".into())
        })?;

        let start_str = args
            .get("start_time")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::InvalidArguments("missing required string field 'start_time'".into())
            })?;

        let start =
            OffsetDateTime::parse(start_str, &time::format_description::well_known::Rfc3339)
                .map_err(|e| {
                    ToolError::InvalidArguments(format!("invalid RFC3339 start_time: {e}"))
                })?;

        let end_str = args
            .get("end_time")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::InvalidArguments("missing required string field 'end_time'".into())
            })?;

        let end = OffsetDateTime::parse(end_str, &time::format_description::well_known::Rfc3339)
            .map_err(|e| ToolError::InvalidArguments(format!("invalid RFC3339 end_time: {e}")))?;

        let description = args
            .get("description")
            .and_then(|v| v.as_str())
            .map(String::from);

        let location = args
            .get("location")
            .and_then(|v| v.as_str())
            .map(String::from);

        let user_id = args
            .get("_user_id")
            .and_then(|v| v.as_str())
            .and_then(|s| Uuid::parse_str(s).ok())
            .unwrap_or_default();

        let created = self
            .provider
            .create(
                account_id,
                user_id,
                CreateCalendarEvent {
                    account_id,

                    title: title.into(),

                    start_time: start,

                    end_time: end,

                    description,

                    location,
                },
            )
            .await?;

        Ok(json!(created))
    }

    async fn execute_with_user(
        &self,

        user_id: Option<Uuid>,

        mut args: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        if let (Some(uid), Some(obj)) = (user_id, args.as_object_mut()) {
            obj.insert("_user_id".into(), json!(uid.to_string()));
        }

        self.execute(args).await
    }
}

// ==========================================

// 6. Calendar Update Tool

// ==========================================

pub struct CalendarUpdateTool {
    provider: Arc<dyn CalendarProvider>,

    spec: ToolSpec,
}

impl CalendarUpdateTool {
    pub fn new(provider: Arc<dyn CalendarProvider>) -> Self {
        Self {
            provider,

            spec: ToolSpec {
                name: "calendar.update".into(),

                description: "Update an existing event in Google Calendar".into(),

                input_schema: json!({

                    "type": "object",

                    "properties": {

                        "account_id": { "type": "string", "format": "uuid" },

                        "event_id": { "type": "string" },

                        "title": { "type": "string" },

                        "start_time": { "type": "string", "format": "date-time" },

                        "end_time": { "type": "string", "format": "date-time" },

                        "description": { "type": "string" },

                        "location": { "type": "string" }

                    },

                    "required": ["account_id", "event_id"]

                }),

                output_schema: json!({ "type": "object" }),

                risk: RiskLevel::Yellow,

                required_scopes: vec!["calendar.events".into()],

                timeout_ms: 10_000,
            },
        }
    }
}

#[async_trait]

impl Tool for CalendarUpdateTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn execute(&self, args: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let account_id_str = args
            .get("account_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::InvalidArguments("missing required string field 'account_id'".into())
            })?;

        let account_id = Uuid::parse_str(account_id_str)
            .map_err(|e| ToolError::InvalidArguments(format!("invalid account_id UUID: {e}")))?;

        let event_id = args
            .get("event_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::InvalidArguments("missing required string field 'event_id'".into())
            })?;

        let title = args.get("title").and_then(|v| v.as_str()).map(String::from);

        let start_time = if let Some(s) = args.get("start_time").and_then(|v| v.as_str()) {
            Some(
                OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339)
                    .map_err(|e| ToolError::InvalidArguments(format!("invalid start_time: {e}")))?,
            )
        } else {
            None
        };

        let end_time = if let Some(s) = args.get("end_time").and_then(|v| v.as_str()) {
            Some(
                OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339)
                    .map_err(|e| ToolError::InvalidArguments(format!("invalid end_time: {e}")))?,
            )
        } else {
            None
        };

        let description = args
            .get("description")
            .and_then(|v| v.as_str())
            .map(String::from);

        let location = args
            .get("location")
            .and_then(|v| v.as_str())
            .map(String::from);

        let user_id = args
            .get("_user_id")
            .and_then(|v| v.as_str())
            .and_then(|s| Uuid::parse_str(s).ok())
            .unwrap_or_default();

        let updated = self
            .provider
            .update(
                account_id,
                user_id,
                event_id,
                UpdateCalendarEvent {
                    title,

                    start_time,

                    end_time,

                    description,

                    location,
                },
            )
            .await?;

        Ok(json!(updated))
    }

    async fn execute_with_user(
        &self,

        user_id: Option<Uuid>,

        mut args: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        if let (Some(uid), Some(obj)) = (user_id, args.as_object_mut()) {
            obj.insert("_user_id".into(), json!(uid.to_string()));
        }

        self.execute(args).await
    }
}

// ==========================================

// 7. Calendar Delete Tool (Orange -> Requires Approval)

// ==========================================

pub struct CalendarDeleteTool {
    provider: Arc<dyn CalendarProvider>,

    spec: ToolSpec,
}

impl CalendarDeleteTool {
    pub fn new(provider: Arc<dyn CalendarProvider>) -> Self {
        Self {

            provider,

            spec: ToolSpec {

                name: "calendar.delete".into(),

                description: "Delete an event from Google Calendar. Consequential action requiring human approval.".into(),

                input_schema: json!({

                    "type": "object",

                    "properties": {

                        "account_id": { "type": "string", "format": "uuid" },

                        "event_id": { "type": "string" }

                    },

                    "required": ["account_id", "event_id"]

                }),

                output_schema: json!({ "type": "object" }),

                risk: RiskLevel::Orange,

                required_scopes: vec!["calendar.events".into()],

                timeout_ms: 10_000,

            },

        }
    }
}

#[async_trait]

impl Tool for CalendarDeleteTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn execute(&self, args: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let account_id_str = args
            .get("account_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::InvalidArguments("missing required string field 'account_id'".into())
            })?;

        let account_id = Uuid::parse_str(account_id_str)
            .map_err(|e| ToolError::InvalidArguments(format!("invalid account_id UUID: {e}")))?;

        let event_id = args
            .get("event_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::InvalidArguments("missing required string field 'event_id'".into())
            })?;

        let user_id = args
            .get("_user_id")
            .and_then(|v| v.as_str())
            .and_then(|s| Uuid::parse_str(s).ok())
            .unwrap_or_default();

        self.provider.delete(account_id, user_id, event_id).await?;

        Ok(json!({ "deleted": true, "event_id": event_id }))
    }

    async fn execute_with_user(
        &self,

        user_id: Option<Uuid>,

        mut args: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        if let (Some(uid), Some(obj)) = (user_id, args.as_object_mut()) {
            obj.insert("_user_id".into(), json!(uid.to_string()));
        }

        self.execute(args).await
    }
}
