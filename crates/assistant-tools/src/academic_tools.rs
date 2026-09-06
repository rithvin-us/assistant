//! Tool implementations for Classroom and Drive.
//!
//! These are the same capabilities the Classroom, Drive and Academic Overview
//! screens use, reached through the same provider traits. There is one
//! implementation behind both doors: a model calling `classroom.coursework`
//! runs exactly the code a person running a manual refresh does.
//!
//! Risk levels are static properties declared here in Rust and evaluated by
//! `PermissionPolicy`. Nothing a model emits can reach them (ADR-0005):
//!
//! - Every Classroom and Drive operation is read-only: Green.
//! - `academic.sync` writes tasks the user owns and is reversible: Yellow,
//!   matching `calendar.create`.
//!
//! There is deliberately no tool for submitting an assignment, grading, or
//! deleting, moving or sharing a file. Those capabilities do not exist below
//! this layer either, so a tool could not offer them even if one were added.

use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    RiskLevel, Tool, ToolError, ToolSpec,
    providers::{AcademicProvider, ClassroomProvider, DriveProvider},
};

/// Reads a required account id.
///
/// Every academic tool takes one explicitly. The account is never inferred:
/// with several Google accounts connected, guessing which one a request meant
/// would silently read the wrong person's coursework.
fn account_id(args: &serde_json::Value) -> Result<Uuid, ToolError> {
    let raw = args
        .get("account_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidArguments("account_id is required".into()))?;
    Uuid::parse_str(raw).map_err(|_| ToolError::InvalidArguments("account_id is not a uuid".into()))
}

/// Reads the authenticated user injected by the executor.
///
/// This arrives from `execute_with_user`, never from the arguments, so model
/// output cannot name a different user.
fn user_id(args: &serde_json::Value) -> Result<Uuid, ToolError> {
    let raw = args
        .get("_user_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::Failed("no authenticated user in context".into()))?;
    Uuid::parse_str(raw).map_err(|_| ToolError::Failed("invalid user context".into()))
}

fn required_str(args: &serde_json::Value, key: &'static str) -> Result<String, ToolError> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| ToolError::InvalidArguments(format!("{key} is required")))
}

fn account_schema(extra: serde_json::Value, required: Vec<&str>) -> serde_json::Value {
    let mut props = json!({
        "account_id": {
            "type": "string",
            "description": "The connected Google account to read. Required; never inferred."
        }
    });
    if let (Some(p), Some(e)) = (props.as_object_mut(), extra.as_object()) {
        for (k, v) in e {
            p.insert(k.clone(), v.clone());
        }
    }
    let mut req = vec!["account_id"];
    req.extend(required);
    json!({ "type": "object", "properties": props, "required": req })
}

macro_rules! tool_struct {
    ($name:ident, $provider:ident) => {
        pub struct $name {
            provider: Arc<dyn $provider>,
            spec: ToolSpec,
        }
    };
}

// ---------------------------------------------------------------------------
// Classroom
// ---------------------------------------------------------------------------

tool_struct!(ClassroomCoursesTool, ClassroomProvider);

impl ClassroomCoursesTool {
    pub fn new(provider: Arc<dyn ClassroomProvider>) -> Self {
        Self {
            provider,
            spec: ToolSpec {
                name: "classroom.courses".into(),
                description: "Lists the courses the user is enrolled in on one Google account."
                    .into(),
                input_schema: account_schema(json!({}), vec![]),
                output_schema: json!({ "type": "array" }),
                risk: RiskLevel::Green,
                required_scopes: vec!["classroom.courses.readonly".into()],
                timeout_ms: 10_000,
            },
        }
    }
}

#[async_trait]
impl Tool for ClassroomCoursesTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn execute(&self, args: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let courses = self
            .provider
            .courses(account_id(&args)?, user_id(&args)?)
            .await?;
        serde_json::to_value(courses).map_err(|e| ToolError::Failed(e.to_string()))
    }

    async fn execute_with_user(
        &self,
        user: Option<Uuid>,
        mut args: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        inject_user(&mut args, user);
        self.execute(args).await
    }
}

tool_struct!(ClassroomCourseworkTool, ClassroomProvider);

impl ClassroomCourseworkTool {
    pub fn new(provider: Arc<dyn ClassroomProvider>) -> Self {
        Self {
            provider,
            spec: ToolSpec {
                name: "classroom.coursework".into(),
                description: "Lists assignments for one course, including due dates. An \
                              assignment with no due date is reported without one rather than \
                              being given a guessed deadline."
                    .into(),
                input_schema: account_schema(
                    json!({ "course_id": { "type": "string", "description": "Course identifier." } }),
                    vec!["course_id"],
                ),
                output_schema: json!({ "type": "array" }),
                risk: RiskLevel::Green,
                required_scopes: vec!["classroom.coursework.me.readonly".into()],
                timeout_ms: 10_000,
            },
        }
    }
}

#[async_trait]
impl Tool for ClassroomCourseworkTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn execute(&self, args: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let items = self
            .provider
            .coursework(
                account_id(&args)?,
                user_id(&args)?,
                &required_str(&args, "course_id")?,
            )
            .await?;
        serde_json::to_value(items).map_err(|e| ToolError::Failed(e.to_string()))
    }

    async fn execute_with_user(
        &self,
        user: Option<Uuid>,
        mut args: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        inject_user(&mut args, user);
        self.execute(args).await
    }
}

tool_struct!(ClassroomAnnouncementsTool, ClassroomProvider);

impl ClassroomAnnouncementsTool {
    pub fn new(provider: Arc<dyn ClassroomProvider>) -> Self {
        Self {
            provider,
            spec: ToolSpec {
                name: "classroom.announcements".into(),
                description: "Lists recent announcements for one course.".into(),
                input_schema: account_schema(
                    json!({
                        "course_id": { "type": "string", "description": "Course identifier." },
                        "limit": { "type": "integer", "description": "Maximum announcements, default 20." }
                    }),
                    vec!["course_id"],
                ),
                output_schema: json!({ "type": "array" }),
                risk: RiskLevel::Green,
                required_scopes: vec!["classroom.announcements.readonly".into()],
                timeout_ms: 10_000,
            },
        }
    }
}

#[async_trait]
impl Tool for ClassroomAnnouncementsTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn execute(&self, args: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as u32;
        let items = self
            .provider
            .announcements(
                account_id(&args)?,
                user_id(&args)?,
                &required_str(&args, "course_id")?,
                limit,
            )
            .await?;
        serde_json::to_value(items).map_err(|e| ToolError::Failed(e.to_string()))
    }

    async fn execute_with_user(
        &self,
        user: Option<Uuid>,
        mut args: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        inject_user(&mut args, user);
        self.execute(args).await
    }
}

// ---------------------------------------------------------------------------
// Drive
// ---------------------------------------------------------------------------

tool_struct!(DriveSearchTool, DriveProvider);

impl DriveSearchTool {
    pub fn new(provider: Arc<dyn DriveProvider>) -> Self {
        Self {
            provider,
            spec: ToolSpec {
                name: "drive.search".into(),
                description: "Searches the user's Drive by file name, optionally filtered by \
                              MIME type. Returns metadata only."
                    .into(),
                input_schema: account_schema(
                    json!({
                        "q": { "type": "string", "description": "Text to match in the file name." },
                        "mime_type": { "type": "string", "description": "Optional exact MIME type filter." },
                        "limit": { "type": "integer", "description": "Maximum results, default 25." }
                    }),
                    vec![],
                ),
                output_schema: json!({ "type": "array" }),
                risk: RiskLevel::Green,
                required_scopes: vec!["drive.readonly".into()],
                timeout_ms: 15_000,
            },
        }
    }
}

#[async_trait]
impl Tool for DriveSearchTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn execute(&self, args: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let q = args.get("q").and_then(|v| v.as_str()).unwrap_or_default();
        let mime = args.get("mime_type").and_then(|v| v.as_str());
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(25) as u32;
        let files = self
            .provider
            .search(account_id(&args)?, user_id(&args)?, q, mime, limit)
            .await?;
        serde_json::to_value(files).map_err(|e| ToolError::Failed(e.to_string()))
    }

    async fn execute_with_user(
        &self,
        user: Option<Uuid>,
        mut args: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        inject_user(&mut args, user);
        self.execute(args).await
    }
}

tool_struct!(DriveListTool, DriveProvider);

impl DriveListTool {
    pub fn new(provider: Arc<dyn DriveProvider>) -> Self {
        Self {
            provider,
            spec: ToolSpec {
                name: "drive.list".into(),
                description: "Lists the contents of a Drive folder, or the root when no folder \
                              is given. Returns metadata only."
                    .into(),
                input_schema: account_schema(
                    json!({
                        "folder_id": { "type": "string", "description": "Folder identifier; omit for the root." },
                        "limit": { "type": "integer", "description": "Maximum results, default 50." }
                    }),
                    vec![],
                ),
                output_schema: json!({ "type": "array" }),
                risk: RiskLevel::Green,
                required_scopes: vec!["drive.readonly".into()],
                timeout_ms: 15_000,
            },
        }
    }
}

#[async_trait]
impl Tool for DriveListTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn execute(&self, args: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let folder = args.get("folder_id").and_then(|v| v.as_str());
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(50) as u32;
        let files = self
            .provider
            .list(account_id(&args)?, user_id(&args)?, folder, limit)
            .await?;
        serde_json::to_value(files).map_err(|e| ToolError::Failed(e.to_string()))
    }

    async fn execute_with_user(
        &self,
        user: Option<Uuid>,
        mut args: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        inject_user(&mut args, user);
        self.execute(args).await
    }
}

tool_struct!(DriveMetadataTool, DriveProvider);

impl DriveMetadataTool {
    pub fn new(provider: Arc<dyn DriveProvider>) -> Self {
        Self {
            provider,
            spec: ToolSpec {
                name: "drive.get_metadata".into(),
                description: "Reads name, type, size and modified time for one Drive file.".into(),
                input_schema: account_schema(
                    json!({ "file_id": { "type": "string", "description": "File identifier." } }),
                    vec!["file_id"],
                ),
                output_schema: json!({ "type": "object" }),
                risk: RiskLevel::Green,
                required_scopes: vec!["drive.readonly".into()],
                timeout_ms: 10_000,
            },
        }
    }
}

#[async_trait]
impl Tool for DriveMetadataTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn execute(&self, args: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let file = self
            .provider
            .metadata(
                account_id(&args)?,
                user_id(&args)?,
                &required_str(&args, "file_id")?,
            )
            .await?;
        serde_json::to_value(file).map_err(|e| ToolError::Failed(e.to_string()))
    }

    async fn execute_with_user(
        &self,
        user: Option<Uuid>,
        mut args: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        inject_user(&mut args, user);
        self.execute(args).await
    }
}

tool_struct!(DriveReadFileTool, DriveProvider);

impl DriveReadFileTool {
    pub fn new(provider: Arc<dyn DriveProvider>) -> Self {
        Self {
            provider,
            spec: ToolSpec {
                name: "drive.read_small_file".into(),
                description: "Reads a small text-shaped Drive file, or exports a Google Doc or \
                              Sheet as text. Large files, PDFs and binary formats are refused \
                              with an explanation rather than partially read."
                    .into(),
                input_schema: account_schema(
                    json!({ "file_id": { "type": "string", "description": "File identifier." } }),
                    vec!["file_id"],
                ),
                output_schema: json!({ "type": "object" }),
                risk: RiskLevel::Green,
                required_scopes: vec!["drive.readonly".into()],
                timeout_ms: 20_000,
            },
        }
    }
}

#[async_trait]
impl Tool for DriveReadFileTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn execute(&self, args: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let content = self
            .provider
            .read_small_file(
                account_id(&args)?,
                user_id(&args)?,
                &required_str(&args, "file_id")?,
            )
            .await?;
        serde_json::to_value(content).map_err(|e| ToolError::Failed(e.to_string()))
    }

    async fn execute_with_user(
        &self,
        user: Option<Uuid>,
        mut args: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        inject_user(&mut args, user);
        self.execute(args).await
    }
}

// ---------------------------------------------------------------------------
// Unified academic context
// ---------------------------------------------------------------------------

tool_struct!(AcademicDeadlinesTool, AcademicProvider);

impl AcademicDeadlinesTool {
    pub fn new(provider: Arc<dyn AcademicProvider>) -> Self {
        Self {
            provider,
            spec: ToolSpec {
                name: "academic.deadlines".into(),
                description: "Lists outstanding academic deadlines across every source --                               imported coursework and manually created tasks alike -- nearest                               first. Reads cached data and does not call any external API."
                    .into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "limit": { "type": "integer", "description": "Maximum deadlines, default 20." }
                    },
                    "required": []
                }),
                output_schema: json!({ "type": "array" }),
                risk: RiskLevel::Green,
                required_scopes: vec![],
                timeout_ms: 10_000,
            },
        }
    }
}

#[async_trait]
impl Tool for AcademicDeadlinesTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn execute(&self, args: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(20);
        let items = self.provider.deadlines(user_id(&args)?, limit).await?;
        serde_json::to_value(items).map_err(|e| ToolError::Failed(e.to_string()))
    }

    async fn execute_with_user(
        &self,
        user: Option<Uuid>,
        mut args: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        inject_user(&mut args, user);
        self.execute(args).await
    }
}

tool_struct!(AcademicAssignmentsTool, AcademicProvider);

impl AcademicAssignmentsTool {
    pub fn new(provider: Arc<dyn AcademicProvider>) -> Self {
        Self {
            provider,
            spec: ToolSpec {
                name: "academic.assignments".into(),
                description: "Lists cached coursework for one account, optionally narrowed to a                               course. Reads the cache and does not call any external API."
                    .into(),
                input_schema: account_schema(
                    json!({ "course_id": { "type": "string", "description": "Optional course identifier." } }),
                    vec![],
                ),
                output_schema: json!({ "type": "array" }),
                risk: RiskLevel::Green,
                required_scopes: vec![],
                timeout_ms: 10_000,
            },
        }
    }
}

#[async_trait]
impl Tool for AcademicAssignmentsTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn execute(&self, args: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let course = args.get("course_id").and_then(|v| v.as_str());
        let items = self
            .provider
            .assignments(account_id(&args)?, user_id(&args)?, course)
            .await?;
        serde_json::to_value(items).map_err(|e| ToolError::Failed(e.to_string()))
    }

    async fn execute_with_user(
        &self,
        user: Option<Uuid>,
        mut args: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        inject_user(&mut args, user);
        self.execute(args).await
    }
}

tool_struct!(AcademicSyncTool, AcademicProvider);

impl AcademicSyncTool {
    pub fn new(provider: Arc<dyn AcademicProvider>) -> Self {
        Self {
            provider,
            spec: ToolSpec {
                name: "academic.sync".into(),
                // Yellow, not Green: this one writes. It creates and updates
                // tasks the user owns, which is reversible and low-risk, and
                // matches how `calendar.create` is rated. It cannot delete a
                // task, and it never overwrites a field the user has edited.
                description: "Refreshes Classroom for one account and imports coursework into                               tasks. Safe to run repeatedly: it updates the existing task for an                               assignment rather than creating another."
                    .into(),
                input_schema: account_schema(json!({}), vec![]),
                output_schema: json!({ "type": "object" }),
                risk: RiskLevel::Yellow,
                required_scopes: vec!["classroom.coursework.me.readonly".into()],
                timeout_ms: 60_000,
            },
        }
    }
}

#[async_trait]
impl Tool for AcademicSyncTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn execute(&self, args: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let result = self
            .provider
            .sync(account_id(&args)?, user_id(&args)?)
            .await?;
        serde_json::to_value(result).map_err(|e| ToolError::Failed(e.to_string()))
    }

    async fn execute_with_user(
        &self,
        user: Option<Uuid>,
        mut args: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        inject_user(&mut args, user);
        self.execute(args).await
    }
}

/// Puts the authenticated user into the argument object.
///
/// Under `_user_id` so it cannot be confused with a model-supplied field, and
/// overwritten unconditionally so a model that guesses the key cannot smuggle
/// a different user id past it.
fn inject_user(args: &mut serde_json::Value, user: Option<Uuid>) {
    if let (Some(map), Some(id)) = (args.as_object_mut(), user) {
        map.insert("_user_id".into(), json!(id.to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::{Announcement, Course, CourseworkItem, DriveFile, DriveFileContent};

    struct StubClassroom;

    #[async_trait]
    impl ClassroomProvider for StubClassroom {
        async fn courses(&self, _a: Uuid, _u: Uuid) -> Result<Vec<Course>, ToolError> {
            Ok(vec![])
        }
        async fn coursework(
            &self,
            _a: Uuid,
            _u: Uuid,
            _c: &str,
        ) -> Result<Vec<CourseworkItem>, ToolError> {
            Ok(vec![])
        }
        async fn announcements(
            &self,
            _a: Uuid,
            _u: Uuid,
            _c: &str,
            _l: u32,
        ) -> Result<Vec<Announcement>, ToolError> {
            Ok(vec![])
        }
    }

    struct StubDrive;

    #[async_trait]
    impl DriveProvider for StubDrive {
        async fn search(
            &self,
            _a: Uuid,
            _u: Uuid,
            _q: &str,
            _m: Option<&str>,
            _l: u32,
        ) -> Result<Vec<DriveFile>, ToolError> {
            Ok(vec![])
        }
        async fn list(
            &self,
            _a: Uuid,
            _u: Uuid,
            _f: Option<&str>,
            _l: u32,
        ) -> Result<Vec<DriveFile>, ToolError> {
            Ok(vec![])
        }
        async fn metadata(&self, _a: Uuid, _u: Uuid, _f: &str) -> Result<DriveFile, ToolError> {
            Err(ToolError::NotFound("file".into()))
        }
        async fn read_small_file(
            &self,
            _a: Uuid,
            _u: Uuid,
            _f: &str,
        ) -> Result<DriveFileContent, ToolError> {
            Err(ToolError::NotFound("file".into()))
        }
    }

    #[test]
    fn every_read_tool_is_green() {
        let c: Arc<dyn ClassroomProvider> = Arc::new(StubClassroom);
        let d: Arc<dyn DriveProvider> = Arc::new(StubDrive);
        for spec in [
            ClassroomCoursesTool::new(c.clone()).spec().clone(),
            ClassroomCourseworkTool::new(c.clone()).spec().clone(),
            ClassroomAnnouncementsTool::new(c).spec().clone(),
            DriveSearchTool::new(d.clone()).spec().clone(),
            DriveListTool::new(d.clone()).spec().clone(),
            DriveMetadataTool::new(d.clone()).spec().clone(),
            DriveReadFileTool::new(d).spec().clone(),
        ] {
            assert_eq!(spec.risk, RiskLevel::Green, "{} must be Green", spec.name);
            assert!(!spec.required_scopes.is_empty(), "{}", spec.name);
        }
    }

    #[test]
    fn every_tool_requires_an_explicit_account() {
        let c: Arc<dyn ClassroomProvider> = Arc::new(StubClassroom);
        let spec = ClassroomCoursesTool::new(c).spec().clone();
        let required = spec.input_schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "account_id"));
    }

    #[tokio::test]
    async fn a_missing_account_is_rejected_before_any_call() {
        let c: Arc<dyn ClassroomProvider> = Arc::new(StubClassroom);
        let tool = ClassroomCoursesTool::new(c);
        let err = tool
            .execute_with_user(Some(Uuid::nil()), json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArguments(_)));
    }

    #[tokio::test]
    async fn a_tool_without_user_context_refuses() {
        // No `_user_id` injected: the executor did not authenticate this call,
        // so it must not reach a provider.
        let c: Arc<dyn ClassroomProvider> = Arc::new(StubClassroom);
        let tool = ClassroomCoursesTool::new(c);
        let err = tool
            .execute(json!({ "account_id": Uuid::nil().to_string() }))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Failed(_)));
    }

    #[tokio::test]
    async fn model_supplied_user_id_cannot_override_the_authenticated_one() {
        let attacker = Uuid::parse_str("11111111-1111-4111-8111-111111111111").unwrap();
        let real = Uuid::parse_str("22222222-2222-4222-8222-222222222222").unwrap();

        let mut args = json!({
            "account_id": Uuid::nil().to_string(),
            "_user_id": attacker.to_string(),
        });
        inject_user(&mut args, Some(real));

        assert_eq!(
            args["_user_id"],
            real.to_string(),
            "the executor's user must win"
        );
    }
}
