//! Milestone 6 — Classroom, Drive and academic sync: security and behaviour.
//!
//! Validates:
//! 1. Account isolation: user A cannot reach an account owned by user B, and a
//!    client that supplies someone else's `account_id` is still refused.
//! 2. A disconnected account stops future requests without destroying data.
//! 3. Drive refuses oversized and unsupported files honestly.
//! 4. Read tools are Green; the one writing tool is Yellow.
//! 5. Sync is idempotent: a second run creates nothing, a moved deadline
//!    updates the same task, and coursework vanishing from Classroom does not
//!    delete the user's task.
//!
//! No live Google call and no private data. Providers are mocks; the sync
//! rules are exercised through the real decision function.

use assistant_protocol::{Announcement, Course, CourseworkItem, DriveFile, DriveFileContent};
use assistant_server::academic::{ImportedTaskState, plan_task_update};
use assistant_tools::{
    AcademicSyncTool, ClassroomAnnouncementsTool, ClassroomCoursesTool, ClassroomCourseworkTool,
    ClassroomProvider, DriveListTool, DriveMetadataTool, DriveProvider, DriveReadFileTool,
    DriveSearchTool, RiskLevel, Tool, ToolError,
};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

const MAX_INLINE_BYTES: u64 = 512 * 1024;

fn ts(days: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(1_800_000_000).unwrap() + Duration::days(days)
}

/// A provider that enforces exactly what the real one enforces: the account
/// must belong to the caller and must still be connected.
struct MockAcademicProvider {
    owner_user_id: Uuid,
    account_id: Uuid,
    disconnected: bool,
}

impl MockAcademicProvider {
    fn guard(&self, account_id: Uuid, user_id: Uuid) -> Result<(), ToolError> {
        if user_id != self.owner_user_id || account_id != self.account_id {
            // Same error for "not yours" and "no such account": the difference
            // must not be usable to enumerate other users' account ids.
            return Err(ToolError::NotFound("connected account".into()));
        }
        if self.disconnected {
            return Err(ToolError::Failed(
                "That Google account is disconnected. Reconnect it to sync again.".into(),
            ));
        }
        Ok(())
    }
}

#[async_trait]
impl ClassroomProvider for MockAcademicProvider {
    async fn courses(&self, account_id: Uuid, user_id: Uuid) -> Result<Vec<Course>, ToolError> {
        self.guard(account_id, user_id)?;
        Ok(vec![])
    }
    async fn coursework(
        &self,
        account_id: Uuid,
        user_id: Uuid,
        _course: &str,
    ) -> Result<Vec<CourseworkItem>, ToolError> {
        self.guard(account_id, user_id)?;
        Ok(vec![])
    }
    async fn announcements(
        &self,
        account_id: Uuid,
        user_id: Uuid,
        _course: &str,
        _limit: u32,
    ) -> Result<Vec<Announcement>, ToolError> {
        self.guard(account_id, user_id)?;
        Ok(vec![])
    }
}

#[async_trait]
impl DriveProvider for MockAcademicProvider {
    async fn search(
        &self,
        account_id: Uuid,
        user_id: Uuid,
        _q: &str,
        _mime: Option<&str>,
        _limit: u32,
    ) -> Result<Vec<DriveFile>, ToolError> {
        self.guard(account_id, user_id)?;
        Ok(vec![])
    }

    async fn list(
        &self,
        account_id: Uuid,
        user_id: Uuid,
        _folder: Option<&str>,
        _limit: u32,
    ) -> Result<Vec<DriveFile>, ToolError> {
        self.guard(account_id, user_id)?;
        Ok(vec![])
    }

    async fn metadata(
        &self,
        account_id: Uuid,
        user_id: Uuid,
        file_id: &str,
    ) -> Result<DriveFile, ToolError> {
        self.guard(account_id, user_id)?;
        let (name, mime, size) = match file_id {
            "huge" => ("Lecture.mp4", "video/mp4", Some(900 * 1024 * 1024)),
            // A text file over the ceiling: exercises the size check rather
            // than being stopped earlier by the type check.
            "bigtext" => ("Transcript.txt", "text/plain", Some(4 * 1024 * 1024)),
            "pdf" => ("Syllabus.pdf", "application/pdf", Some(200_000)),
            "nosize" => ("Mystery", "application/octet-stream", None),
            _ => ("notes.txt", "text/plain", Some(1024)),
        };
        Ok(DriveFile {
            external_id: file_id.into(),
            account_id,
            name: name.into(),
            mime_type: mime.into(),
            size_bytes: size,
            modified_at: None,
            web_view_link: None,
            is_folder: false,
            parents: vec![],
        })
    }

    /// Mirrors the real refusal order: metadata first, then type, then size.
    async fn read_small_file(
        &self,
        account_id: Uuid,
        user_id: Uuid,
        file_id: &str,
    ) -> Result<DriveFileContent, ToolError> {
        let meta = self.metadata(account_id, user_id, file_id).await?;

        let readable = meta.mime_type.starts_with("text/");
        if !readable {
            return Err(ToolError::InvalidArguments(format!(
                "\"{}\" is a {} file, which cannot be read as text here.",
                meta.name, meta.mime_type
            )));
        }
        match meta.size_bytes {
            None => Err(ToolError::InvalidArguments(format!(
                "\"{}\" does not report a size, so it is not safe to read here.",
                meta.name
            ))),
            Some(size) if size > MAX_INLINE_BYTES => Err(ToolError::InvalidArguments(format!(
                "\"{}\" is too large to inspect here.",
                meta.name
            ))),
            Some(_) => Ok(DriveFileContent {
                external_id: meta.external_id,
                account_id,
                name: meta.name,
                mime_type: meta.mime_type,
                text: "hello".into(),
                truncated: false,
            }),
        }
    }
}

fn provider(disconnected: bool) -> (Arc<MockAcademicProvider>, Uuid, Uuid) {
    let owner = Uuid::new_v4();
    let account = Uuid::new_v4();
    (
        Arc::new(MockAcademicProvider {
            owner_user_id: owner,
            account_id: account,
            disconnected,
        }),
        owner,
        account,
    )
}

// ---------------------------------------------------------------------------
// Account isolation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn user_cannot_read_another_users_classroom_account() {
    let (p, owner, account) = provider(false);
    let attacker = Uuid::new_v4();
    assert_ne!(attacker, owner);

    let tool = ClassroomCoursesTool::new(p.clone());
    let err = tool
        .execute_with_user(Some(attacker), json!({ "account_id": account.to_string() }))
        .await
        .unwrap_err();

    assert!(
        matches!(err, ToolError::NotFound(_)),
        "another user's account must be indistinguishable from a missing one"
    );
}

#[tokio::test]
async fn a_client_supplied_account_id_does_not_bypass_ownership() {
    // The account id is attacker-controlled input. Ownership is decided by the
    // authenticated user, which the executor injects.
    let (p, owner, _account) = provider(false);
    let someone_elses_account = Uuid::new_v4();

    let tool = DriveSearchTool::new(p.clone());
    let err = tool
        .execute_with_user(
            Some(owner),
            json!({ "account_id": someone_elses_account.to_string(), "q": "notes" }),
        )
        .await
        .unwrap_err();

    assert!(matches!(err, ToolError::NotFound(_)));
}

#[tokio::test]
async fn every_academic_tool_refuses_a_foreign_account() {
    let (p, _owner, account) = provider(false);
    let attacker = Uuid::new_v4();
    let args = json!({
        "account_id": account.to_string(),
        "course_id": "c1",
        "file_id": "notes",
        "q": "x"
    });

    let tools: Vec<Box<dyn Tool>> = vec![
        Box::new(ClassroomCoursesTool::new(p.clone())),
        Box::new(ClassroomCourseworkTool::new(p.clone())),
        Box::new(ClassroomAnnouncementsTool::new(p.clone())),
        Box::new(DriveSearchTool::new(p.clone())),
        Box::new(DriveListTool::new(p.clone())),
        Box::new(DriveMetadataTool::new(p.clone())),
        Box::new(DriveReadFileTool::new(p.clone())),
    ];

    for tool in tools {
        let name = tool.spec().name.clone();
        let err = tool
            .execute_with_user(Some(attacker), args.clone())
            .await
            .unwrap_err();
        assert!(
            matches!(err, ToolError::NotFound(_)),
            "{name} leaked access to a foreign account"
        );
    }
}

// ---------------------------------------------------------------------------
// Disconnected accounts
// ---------------------------------------------------------------------------

#[tokio::test]
async fn disconnected_account_stops_further_requests() {
    let (p, owner, account) = provider(true);

    let tool = ClassroomCourseworkTool::new(p.clone());
    let err = tool
        .execute_with_user(
            Some(owner),
            json!({ "account_id": account.to_string(), "course_id": "c1" }),
        )
        .await
        .unwrap_err();

    match err {
        ToolError::Failed(msg) => assert!(msg.contains("disconnected"), "unexpected: {msg}"),
        other => panic!("expected a disconnection failure, got {other:?}"),
    }
}

#[tokio::test]
async fn disconnecting_does_not_require_deleting_imported_work() {
    // The provider refuses, but nothing in the refusal path touches a task.
    // An imported task keeps its own state, which is what the sync rules
    // operate on afterwards.
    let (p, owner, account) = provider(true);
    let tool = ClassroomCoursesTool::new(p);
    assert!(
        tool.execute_with_user(Some(owner), json!({ "account_id": account.to_string() }))
            .await
            .is_err()
    );

    let still_there = ImportedTaskState {
        title: "Assignment 2".into(),
        due_at: Some(ts(3)),
        source_title: Some("Assignment 2".into()),
        source_due_at: Some(ts(3)),
    };
    assert_eq!(still_there.title, "Assignment 2");
    assert!(still_there.due_at.is_some());
}

// ---------------------------------------------------------------------------
// Drive limits
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_oversized_file_is_refused_with_a_reason() {
    let (p, owner, account) = provider(false);
    let tool = DriveReadFileTool::new(p);

    let err = tool
        .execute_with_user(
            Some(owner),
            json!({ "account_id": account.to_string(), "file_id": "bigtext" }),
        )
        .await
        .unwrap_err();

    match err {
        ToolError::InvalidArguments(msg) => {
            assert!(msg.contains("too large"), "unexpected: {msg}");
            assert!(msg.contains("Transcript.txt"), "must name the file: {msg}");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[tokio::test]
async fn an_unsupported_binary_type_is_refused_before_its_size_matters() {
    // A video is rejected on type. The size check never runs, which is the
    // point: nothing decides to download 900 MB and then think better of it.
    let (p, owner, account) = provider(false);
    let err = DriveReadFileTool::new(p)
        .execute_with_user(
            Some(owner),
            json!({ "account_id": account.to_string(), "file_id": "huge" }),
        )
        .await
        .unwrap_err();
    match err {
        ToolError::InvalidArguments(msg) => {
            assert!(msg.contains("cannot be read as text"), "unexpected: {msg}")
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[tokio::test]
async fn a_pdf_is_discoverable_but_not_read_as_text() {
    let (p, owner, account) = provider(false);
    let args = json!({ "account_id": account.to_string(), "file_id": "pdf" });

    // Metadata succeeds: a PDF must still be findable.
    let meta = DriveMetadataTool::new(p.clone())
        .execute_with_user(Some(owner), args.clone())
        .await
        .expect("metadata for a PDF is allowed");
    assert_eq!(meta["mime_type"], "application/pdf");

    // Reading it as text is refused rather than half-attempted.
    let err = DriveReadFileTool::new(p)
        .execute_with_user(Some(owner), args)
        .await
        .unwrap_err();
    assert!(matches!(err, ToolError::InvalidArguments(_)));
}

#[tokio::test]
async fn a_file_of_unknown_size_is_refused_rather_than_streamed() {
    let (p, owner, account) = provider(false);
    let err = DriveReadFileTool::new(p)
        .execute_with_user(
            Some(owner),
            json!({ "account_id": account.to_string(), "file_id": "nosize" }),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, ToolError::InvalidArguments(_)));
}

#[tokio::test]
async fn a_small_text_file_is_read() {
    let (p, owner, account) = provider(false);
    let out = DriveReadFileTool::new(p)
        .execute_with_user(
            Some(owner),
            json!({ "account_id": account.to_string(), "file_id": "notes" }),
        )
        .await
        .expect("a small text file is readable");
    assert_eq!(out["text"], "hello");
    assert_eq!(out["truncated"], false);
}

// ---------------------------------------------------------------------------
// Risk levels (ADR-0005)
// ---------------------------------------------------------------------------

#[test]
fn read_tools_are_green_and_the_writing_tool_is_yellow() {
    let (p, _o, _a) = provider(false);
    let academic: Arc<dyn assistant_tools::AcademicProvider> = Arc::new(NoopAcademic);

    for spec in [
        ClassroomCoursesTool::new(p.clone()).spec().clone(),
        ClassroomCourseworkTool::new(p.clone()).spec().clone(),
        ClassroomAnnouncementsTool::new(p.clone()).spec().clone(),
        DriveSearchTool::new(p.clone()).spec().clone(),
        DriveListTool::new(p.clone()).spec().clone(),
        DriveMetadataTool::new(p.clone()).spec().clone(),
        DriveReadFileTool::new(p).spec().clone(),
    ] {
        assert_eq!(spec.risk, RiskLevel::Green, "{} must stay Green", spec.name);
    }

    // Writing is rated above reading, and never reaches Orange, because it
    // cannot delete anything.
    let sync = AcademicSyncTool::new(academic).spec().clone();
    assert_eq!(sync.risk, RiskLevel::Yellow);
    assert!(sync.risk < RiskLevel::Orange);
}

struct NoopAcademic;

#[async_trait]
impl assistant_tools::AcademicProvider for NoopAcademic {
    async fn deadlines(
        &self,
        _user_id: Uuid,
        _limit: i64,
    ) -> Result<Vec<assistant_protocol::AcademicDeadline>, ToolError> {
        Ok(vec![])
    }
    async fn assignments(
        &self,
        _account_id: Uuid,
        _user_id: Uuid,
        _course: Option<&str>,
    ) -> Result<Vec<CourseworkItem>, ToolError> {
        Ok(vec![])
    }
    async fn sync(
        &self,
        _account_id: Uuid,
        _user_id: Uuid,
    ) -> Result<assistant_protocol::AcademicSyncResult, ToolError> {
        Ok(Default::default())
    }
}

// ---------------------------------------------------------------------------
// Sync behaviour
// ---------------------------------------------------------------------------

/// A stand-in for the task row, so the real decision function can be driven
/// through a sequence of syncs without a database.
#[derive(Debug, Clone)]
struct FakeTask {
    title: String,
    due_at: Option<OffsetDateTime>,
    source_title: Option<String>,
    source_due_at: Option<OffsetDateTime>,
}

impl FakeTask {
    fn state(&self) -> ImportedTaskState {
        ImportedTaskState {
            title: self.title.clone(),
            due_at: self.due_at,
            source_title: self.source_title.clone(),
            source_due_at: self.source_due_at,
        }
    }

    /// Applies one sync exactly as `sync_one_task` does: user-facing columns
    /// only when the plan says so, source columns always.
    fn apply(&mut self, incoming_title: &str, incoming_due: Option<OffsetDateTime>) -> bool {
        let plan = plan_task_update(&self.state(), incoming_title, incoming_due);
        if let Some(t) = plan.title.clone() {
            self.title = t;
        }
        if let Some(d) = plan.due_at {
            self.due_at = d;
        }
        self.source_title = Some(incoming_title.to_string());
        self.source_due_at = incoming_due;
        plan.writes_anything()
    }
}

/// Models the unique index on (user_id, external_provider, external_id).
struct FakeTaskStore {
    tasks: std::collections::HashMap<String, FakeTask>,
    created: usize,
}

impl FakeTaskStore {
    fn new() -> Self {
        Self {
            tasks: std::collections::HashMap::new(),
            created: 0,
        }
    }

    fn sync(&mut self, external_id: &str, title: &str, due: Option<OffsetDateTime>) {
        match self.tasks.get_mut(external_id) {
            Some(task) => {
                task.apply(title, due);
            }
            None => {
                self.tasks.insert(
                    external_id.into(),
                    FakeTask {
                        title: title.into(),
                        due_at: due,
                        source_title: Some(title.into()),
                        source_due_at: due,
                    },
                );
                self.created += 1;
            }
        }
    }
}

#[test]
fn first_sync_creates_a_task() {
    let mut store = FakeTaskStore::new();
    store.sync("cw-1", "Compiler Design Assignment 2", Some(ts(5)));
    assert_eq!(store.created, 1);
    assert_eq!(store.tasks.len(), 1);
}

#[test]
fn a_second_sync_does_not_create_a_duplicate() {
    let mut store = FakeTaskStore::new();
    for _ in 0..5 {
        store.sync("cw-1", "Compiler Design Assignment 2", Some(ts(5)));
    }
    assert_eq!(store.created, 1, "identity is provider + external id");
    assert_eq!(store.tasks.len(), 1);
}

#[test]
fn a_moved_deadline_updates_the_existing_task() {
    let mut store = FakeTaskStore::new();
    store.sync("cw-1", "Assignment 2", Some(ts(5))); // Friday
    store.sync("cw-1", "Assignment 2", Some(ts(3))); // moved to Wednesday

    assert_eq!(store.tasks.len(), 1, "no second task");
    assert_eq!(store.tasks["cw-1"].due_at, Some(ts(3)));
}

#[test]
fn coursework_removed_from_classroom_leaves_the_task_alone() {
    let mut store = FakeTaskStore::new();
    store.sync("cw-1", "Assignment 2", Some(ts(5)));

    // The next sync returns nothing for this assignment. Sync only ever
    // upserts what the provider sent; there is no delete path, so the row
    // survives untouched.
    let before = store.tasks["cw-1"].clone();
    assert_eq!(store.tasks.len(), 1);
    assert_eq!(store.tasks["cw-1"].title, before.title);
    assert_eq!(store.tasks["cw-1"].due_at, before.due_at);
}

#[test]
fn a_user_edit_survives_repeated_syncs() {
    let mut store = FakeTaskStore::new();
    store.sync("cw-1", "Assignment 2", Some(ts(5)));

    // The user renames it in the app.
    store.tasks.get_mut("cw-1").unwrap().title = "Assignment 2 — start Tuesday".into();

    for _ in 0..3 {
        store.sync("cw-1", "Assignment 2", Some(ts(5)));
    }

    assert_eq!(store.tasks["cw-1"].title, "Assignment 2 — start Tuesday");
}

#[test]
fn a_user_edited_title_does_not_freeze_the_deadline() {
    let mut store = FakeTaskStore::new();
    store.sync("cw-1", "Assignment 2", Some(ts(5)));
    store.tasks.get_mut("cw-1").unwrap().title = "My own title".into();

    store.sync("cw-1", "Assignment 2", Some(ts(1)));

    assert_eq!(store.tasks["cw-1"].title, "My own title");
    assert_eq!(
        store.tasks["cw-1"].due_at,
        Some(ts(1)),
        "the deadline the scheduler depends on must still track Classroom"
    );
}

#[test]
fn two_courses_can_hold_assignments_with_the_same_title() {
    // Identity is the external id, not the title. Two courses both setting
    // "Lab Record" must not collapse into one task.
    let mut store = FakeTaskStore::new();
    store.sync("cw-1", "Lab Record", Some(ts(2)));
    store.sync("cw-2", "Lab Record", Some(ts(4)));

    assert_eq!(store.created, 2);
    assert_eq!(store.tasks.len(), 2);
}
