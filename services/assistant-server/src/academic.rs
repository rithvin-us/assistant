//! Unified academic context: caching, synchronisation and the overview.
//!
//! This is the one implementation. The Classroom screen, the Academic Overview
//! and the deterministic tools all reach the same functions here — there is no
//! separate path for the UI and a second one for a model. See ADR-0034.
//!
//! Nothing in this module calls a model. Counting overdue assignments and
//! turning coursework into a task are arithmetic and an upsert; routing either
//! through inference would cost money and could be wrong.

use assistant_protocol::{
    AcademicDeadline, AcademicOverview, AcademicSource, AcademicSyncResult, Announcement, Course,
    CourseworkItem, MaterialRef,
};
use assistant_tools::{ClassroomProvider, ToolError};
use sqlx::{PgPool, Row, postgres::PgRow};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

/// The provider string written to `tasks.external_provider`.
pub const CLASSROOM_PROVIDER: &str = "google_classroom";

// ---------------------------------------------------------------------------
// The source-versus-user decision
// ---------------------------------------------------------------------------

/// The fields of an imported task that a sync has to reason about.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedTaskState {
    /// What the task says now.
    pub title: String,
    pub due_at: Option<OffsetDateTime>,
    /// What the provider said last time. `None` on a row that predates
    /// provenance tracking.
    pub source_title: Option<String>,
    pub source_due_at: Option<OffsetDateTime>,
}

/// What a sync intends to write. `None` means "leave this column alone".
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TaskUpdatePlan {
    pub title: Option<String>,
    /// Double option: the outer says whether to write, the inner is the value,
    /// because clearing a due date is a legitimate write.
    pub due_at: Option<Option<OffsetDateTime>>,
    /// True when the provider changed something this sync refused to apply
    /// because the user had edited that field.
    pub skipped_user_edit: bool,
}

impl TaskUpdatePlan {
    pub fn writes_anything(&self) -> bool {
        self.title.is_some() || self.due_at.is_some()
    }
}

/// Decides, field by field, whether the provider or the user owns a value.
///
/// The rule is the same for every field: if the task still holds exactly what
/// the provider last sent, the provider still owns it and a change is applied.
/// If it differs, the user has edited it since, and their edit wins.
///
/// This is why `source_title` and `source_due_at` exist. Without them the only
/// available policies are "always overwrite", which silently destroys the note
/// a user retitled, and "never update", which leaves a moved deadline wrong.
/// Neither is acceptable for something the scheduler will act on.
pub fn plan_task_update(
    existing: &ImportedTaskState,
    incoming_title: &str,
    incoming_due_at: Option<OffsetDateTime>,
) -> TaskUpdatePlan {
    let mut plan = TaskUpdatePlan::default();

    // A row with no recorded source value predates provenance tracking. Treat
    // it as user-owned: assuming the provider owns it would let this sync
    // overwrite a title the user has been maintaining by hand.
    let title_is_source_owned = existing
        .source_title
        .as_deref()
        .is_some_and(|s| s == existing.title);

    if title_is_source_owned {
        if existing.title != incoming_title {
            plan.title = Some(incoming_title.to_string());
        }
    } else if existing.source_title.as_deref() != Some(incoming_title) {
        // The provider has a different title than it last sent, and the user
        // has edited theirs. Their edit stands; record that we held back.
        plan.skipped_user_edit = true;
    }

    let due_is_source_owned = existing.due_at == existing.source_due_at;

    if due_is_source_owned {
        if existing.due_at != incoming_due_at {
            plan.due_at = Some(incoming_due_at);
        }
    } else if existing.source_due_at != incoming_due_at {
        plan.skipped_user_edit = true;
    }

    plan
}

// ---------------------------------------------------------------------------
// Cache reads and writes
// ---------------------------------------------------------------------------

fn materials_from_json(value: serde_json::Value) -> Vec<MaterialRef> {
    serde_json::from_value(value).unwrap_or_default()
}

fn course_from_row(row: &PgRow) -> Course {
    Course {
        external_id: row.get("external_id"),
        account_id: row.get("account_id"),
        name: row.get("name"),
        section: row.get("section"),
        description: row.get("description"),
        room: row.get("room"),
        teacher_name: row.get("teacher_name"),
        state: row.get("course_state"),
        alternate_link: row.get("alternate_link"),
        source_updated_at: row.get("source_updated_at"),
        synced_at: row.get("synced_at"),
    }
}

fn coursework_from_row(row: &PgRow) -> CourseworkItem {
    CourseworkItem {
        external_id: row.get("external_id"),
        course_external_id: row.get("course_external_id"),
        account_id: row.get("account_id"),
        title: row.get("title"),
        description: row.get("description"),
        state: row.get("state"),
        alternate_link: row.get("alternate_link"),
        due_at: row.get("due_at"),
        max_points: row.get("max_points"),
        work_type: row.get("work_type"),
        materials: materials_from_json(row.get("materials")),
        source_updated_at: row.get("source_updated_at"),
        synced_at: row.get("synced_at"),
    }
}

fn announcement_from_row(row: &PgRow) -> Announcement {
    Announcement {
        external_id: row.get("external_id"),
        course_external_id: row.get("course_external_id"),
        account_id: row.get("account_id"),
        text: row.get("text_content"),
        author_name: row.get("author_name"),
        alternate_link: row.get("alternate_link"),
        materials: materials_from_json(row.get("materials")),
        source_created_at: row.get("source_created_at"),
        source_updated_at: row.get("source_updated_at"),
        synced_at: row.get("synced_at"),
    }
}

/// Confirms the account belongs to this user and is usable.
///
/// Every read and write below goes through here first. An account belonging to
/// another user must be indistinguishable from one that does not exist, so the
/// same error is returned for "not yours" and "no such row" — the caller
/// cannot use the difference to probe for account ids.
pub async fn assert_account_active(
    pool: &PgPool,
    user_id: Uuid,
    account_id: Uuid,
) -> Result<(), ToolError> {
    let row = sqlx::query(
        "select status from connected_accounts \
         where id = $1 and user_id = $2 and provider = 'google'",
    )
    .bind(account_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| ToolError::Failed(format!("database error: {e}")))?;

    let Some(row) = row else {
        return Err(ToolError::NotFound("connected account".into()));
    };

    let status: String = row.get("status");
    if status != "active" {
        // Disconnected accounts stop future requests but leave imported data
        // in place. See ADR-0034.
        return Err(ToolError::Failed(
            "That Google account is disconnected. Reconnect it to sync again.".into(),
        ));
    }

    Ok(())
}

/// Reads cached courses. Never calls Google.
pub async fn cached_courses(
    pool: &PgPool,
    user_id: Uuid,
    account_id: Uuid,
) -> Result<Vec<Course>, ToolError> {
    let rows = sqlx::query(
        "select external_id, account_id, name, section, description, room, teacher_name, \
                course_state, alternate_link, source_updated_at, synced_at \
         from classroom_courses where user_id = $1 and account_id = $2 order by name",
    )
    .bind(user_id)
    .bind(account_id)
    .fetch_all(pool)
    .await
    .map_err(|e| ToolError::Failed(format!("database error: {e}")))?;

    Ok(rows.iter().map(course_from_row).collect())
}

pub async fn cached_coursework(
    pool: &PgPool,
    user_id: Uuid,
    account_id: Uuid,
    course_external_id: Option<&str>,
) -> Result<Vec<CourseworkItem>, ToolError> {
    let rows = sqlx::query(
        "select external_id, course_external_id, account_id, title, description, state, \
                alternate_link, due_at, max_points, work_type, materials, source_updated_at, synced_at \
         from classroom_coursework \
         where user_id = $1 and account_id = $2 \
           and ($3::text is null or course_external_id = $3) \
         order by due_at nulls last, title",
    )
    .bind(user_id)
    .bind(account_id)
    .bind(course_external_id)
    .fetch_all(pool)
    .await
    .map_err(|e| ToolError::Failed(format!("database error: {e}")))?;

    Ok(rows.iter().map(coursework_from_row).collect())
}

pub async fn cached_announcements(
    pool: &PgPool,
    user_id: Uuid,
    account_id: Uuid,
    course_external_id: Option<&str>,
    limit: i64,
) -> Result<Vec<Announcement>, ToolError> {
    let rows = sqlx::query(
        "select external_id, course_external_id, account_id, text_content, author_name, \
                alternate_link, materials, source_created_at, source_updated_at, synced_at \
         from classroom_announcements \
         where user_id = $1 and account_id = $2 \
           and ($3::text is null or course_external_id = $3) \
         order by source_created_at desc nulls last limit $4",
    )
    .bind(user_id)
    .bind(account_id)
    .bind(course_external_id)
    .bind(limit.clamp(1, 100))
    .fetch_all(pool)
    .await
    .map_err(|e| ToolError::Failed(format!("database error: {e}")))?;

    Ok(rows.iter().map(announcement_from_row).collect())
}

async fn upsert_course(
    pool: &PgPool,
    user_id: Uuid,
    account_id: Uuid,
    course: &Course,
) -> Result<(), ToolError> {
    sqlx::query(
        "insert into classroom_courses \
            (user_id, account_id, external_id, name, section, description, room, teacher_name, \
             course_state, alternate_link, source_updated_at, synced_at) \
         values ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12) \
         on conflict (user_id, account_id, external_id) do update set \
            name = excluded.name, section = excluded.section, \
            description = excluded.description, room = excluded.room, \
            teacher_name = excluded.teacher_name, course_state = excluded.course_state, \
            alternate_link = excluded.alternate_link, \
            source_updated_at = excluded.source_updated_at, synced_at = excluded.synced_at",
    )
    .bind(user_id)
    .bind(account_id)
    .bind(&course.external_id)
    .bind(&course.name)
    .bind(&course.section)
    .bind(&course.description)
    .bind(&course.room)
    .bind(&course.teacher_name)
    .bind(&course.state)
    .bind(&course.alternate_link)
    .bind(course.source_updated_at)
    .bind(course.synced_at)
    .execute(pool)
    .await
    .map_err(|e| ToolError::Failed(format!("database error: {e}")))?;
    Ok(())
}

async fn upsert_coursework(
    pool: &PgPool,
    user_id: Uuid,
    account_id: Uuid,
    item: &CourseworkItem,
) -> Result<(), ToolError> {
    let materials = serde_json::to_value(&item.materials).unwrap_or(serde_json::json!([]));
    sqlx::query(
        "insert into classroom_coursework \
            (user_id, account_id, course_external_id, external_id, title, description, state, \
             alternate_link, due_at, max_points, work_type, materials, source_updated_at, synced_at) \
         values ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14) \
         on conflict (user_id, account_id, external_id) do update set \
            course_external_id = excluded.course_external_id, title = excluded.title, \
            description = excluded.description, state = excluded.state, \
            alternate_link = excluded.alternate_link, due_at = excluded.due_at, \
            max_points = excluded.max_points, work_type = excluded.work_type, \
            materials = excluded.materials, source_updated_at = excluded.source_updated_at, \
            synced_at = excluded.synced_at",
    )
    .bind(user_id)
    .bind(account_id)
    .bind(&item.course_external_id)
    .bind(&item.external_id)
    .bind(&item.title)
    .bind(&item.description)
    .bind(&item.state)
    .bind(&item.alternate_link)
    .bind(item.due_at)
    .bind(item.max_points)
    .bind(&item.work_type)
    .bind(materials)
    .bind(item.source_updated_at)
    .bind(item.synced_at)
    .execute(pool)
    .await
    .map_err(|e| ToolError::Failed(format!("database error: {e}")))?;
    Ok(())
}

async fn upsert_announcement(
    pool: &PgPool,
    user_id: Uuid,
    account_id: Uuid,
    item: &Announcement,
) -> Result<(), ToolError> {
    let materials = serde_json::to_value(&item.materials).unwrap_or(serde_json::json!([]));
    sqlx::query(
        "insert into classroom_announcements \
            (user_id, account_id, course_external_id, external_id, text_content, author_name, \
             alternate_link, materials, source_created_at, source_updated_at, synced_at) \
         values ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11) \
         on conflict (user_id, account_id, external_id) do update set \
            text_content = excluded.text_content, author_name = excluded.author_name, \
            alternate_link = excluded.alternate_link, materials = excluded.materials, \
            source_updated_at = excluded.source_updated_at, synced_at = excluded.synced_at",
    )
    .bind(user_id)
    .bind(account_id)
    .bind(&item.course_external_id)
    .bind(&item.external_id)
    .bind(&item.text)
    .bind(&item.author_name)
    .bind(&item.alternate_link)
    .bind(materials)
    .bind(item.source_created_at)
    .bind(item.source_updated_at)
    .execute(pool)
    .await
    .map_err(|e| ToolError::Failed(format!("database error: {e}")))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Coursework -> task
// ---------------------------------------------------------------------------

/// Creates or updates the task backing one piece of coursework.
///
/// Identity is `(user_id, external_provider, external_id)`, enforced by a
/// unique index, so running this twice cannot produce two tasks for one
/// assignment however many times a sync is triggered.
async fn sync_one_task(
    pool: &PgPool,
    user_id: Uuid,
    account_id: Uuid,
    item: &CourseworkItem,
    result: &mut AcademicSyncResult,
) -> Result<(), ToolError> {
    let existing = sqlx::query(
        "select id, title, due_at, source_title, source_due_at from tasks \
         where user_id = $1 and external_provider = $2 and external_id = $3",
    )
    .bind(user_id)
    .bind(CLASSROOM_PROVIDER)
    .bind(&item.external_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| ToolError::Failed(format!("database error: {e}")))?;

    let now = OffsetDateTime::now_utc();

    let Some(row) = existing else {
        sqlx::query(
            "insert into tasks \
                (user_id, title, description, due_at, source, external_provider, external_id, \
                 external_account_id, source_title, source_due_at, source_synced_at) \
             values ($1,$2,'',$3,$4,$5,$6,$7,$8,$9,$10) \
             on conflict (user_id, external_provider, external_id) do nothing",
        )
        .bind(user_id)
        .bind(&item.title)
        .bind(item.due_at)
        .bind(CLASSROOM_PROVIDER)
        .bind(CLASSROOM_PROVIDER)
        .bind(&item.external_id)
        .bind(account_id)
        .bind(&item.title)
        .bind(item.due_at)
        .bind(now)
        .execute(pool)
        .await
        .map_err(|e| ToolError::Failed(format!("database error: {e}")))?;

        result.tasks_created += 1;
        return Ok(());
    };

    let task_id: Uuid = row.get("id");
    let state = ImportedTaskState {
        title: row.get("title"),
        due_at: row.get("due_at"),
        source_title: row.get("source_title"),
        source_due_at: row.get("source_due_at"),
    };

    let plan = plan_task_update(&state, &item.title, item.due_at);

    if plan.skipped_user_edit {
        result.tasks_skipped_user_edited += 1;
    }

    // The source columns are always brought up to date, even when nothing
    // user-facing changed. They record what the provider last said, and a
    // stale value there would make the next sync misjudge ownership.
    sqlx::query(
        "update tasks set \
            title = coalesce($1, title), \
            due_at = case when $2 then $3 else due_at end, \
            source_title = $4, source_due_at = $5, source_synced_at = $6, \
            external_account_id = $7 \
         where id = $8 and user_id = $9",
    )
    .bind(plan.title.as_ref())
    .bind(plan.due_at.is_some())
    .bind(plan.due_at.flatten())
    .bind(&item.title)
    .bind(item.due_at)
    .bind(now)
    .bind(account_id)
    .bind(task_id)
    .bind(user_id)
    .execute(pool)
    .await
    .map_err(|e| ToolError::Failed(format!("database error: {e}")))?;

    if plan.writes_anything() {
        result.tasks_updated += 1;
    }

    Ok(())
}

/// Pulls Classroom for one account, refreshes the cache, and imports
/// coursework into tasks.
///
/// Coursework that disappears from Classroom is deliberately left alone: the
/// task belongs to the user, not to Google, and deleting their work because a
/// teacher tidied a course would be destroying data this application does not
/// own. See ADR-0034.
pub async fn sync_account(
    pool: &PgPool,
    provider: &dyn ClassroomProvider,
    user_id: Uuid,
    account_id: Uuid,
    announcements_per_course: u32,
) -> Result<AcademicSyncResult, ToolError> {
    assert_account_active(pool, user_id, account_id).await?;

    let mut result = AcademicSyncResult::default();

    let courses = provider.courses(account_id, user_id).await?;
    for course in &courses {
        upsert_course(pool, user_id, account_id, course).await?;
        result.courses_synced += 1;

        let coursework = provider
            .coursework(account_id, user_id, &course.external_id)
            .await?;
        for item in &coursework {
            upsert_coursework(pool, user_id, account_id, item).await?;
            result.coursework_synced += 1;
            sync_one_task(pool, user_id, account_id, item, &mut result).await?;
        }

        if announcements_per_course > 0 {
            let announcements = provider
                .announcements(
                    account_id,
                    user_id,
                    &course.external_id,
                    announcements_per_course,
                )
                .await?;
            for a in &announcements {
                upsert_announcement(pool, user_id, account_id, a).await?;
                result.announcements_synced += 1;
            }
        }
    }

    record_sync(pool, user_id, account_id, "courses", None).await?;
    record_sync(pool, user_id, account_id, "coursework", None).await?;
    if announcements_per_course > 0 {
        record_sync(pool, user_id, account_id, "announcements", None).await?;
    }

    Ok(result)
}

async fn record_sync(
    pool: &PgPool,
    user_id: Uuid,
    account_id: Uuid,
    resource: &str,
    error: Option<&str>,
) -> Result<(), ToolError> {
    sqlx::query(
        "insert into academic_sync_state (user_id, account_id, resource, last_synced_at, last_error) \
         values ($1,$2,$3,now(),$4) \
         on conflict (user_id, account_id, resource) do update set \
            last_synced_at = excluded.last_synced_at, last_error = excluded.last_error",
    )
    .bind(user_id)
    .bind(account_id)
    .bind(resource)
    .bind(error)
    .execute(pool)
    .await
    .map_err(|e| ToolError::Failed(format!("database error: {e}")))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Overview
// ---------------------------------------------------------------------------

/// Splits deadlines into the counts the overview shows.
///
/// Separated from the query so the arithmetic can be tested directly. "This
/// week" is the next seven days from `now`, not the calendar week, because an
/// assignment due in six days matters the same on a Sunday as on a Tuesday.
pub fn summarize(deadlines: &[AcademicDeadline], now: OffsetDateTime) -> (usize, usize) {
    let week = now + Duration::days(7);
    let mut due_this_week = 0;
    let mut overdue = 0;

    for d in deadlines {
        if d.is_completed {
            continue;
        }
        let Some(due) = d.due_at else { continue };
        if due < now {
            overdue += 1;
        } else if due <= week {
            due_this_week += 1;
        }
    }

    (due_this_week, overdue)
}

/// Builds the Academic Overview from cached rows and imported tasks.
///
/// Reads only. This never calls Google, so opening the screen offline shows
/// the cache with its `synced_at` rather than an error or a lie about being
/// live.
pub async fn overview(
    pool: &PgPool,
    user_id: Uuid,
    upcoming_limit: i64,
) -> Result<AcademicOverview, ToolError> {
    let now = OffsetDateTime::now_utc();

    let course_count: i64 =
        sqlx::query("select count(*) as n from classroom_courses where user_id = $1")
            .bind(user_id)
            .fetch_one(pool)
            .await
            .map_err(|e| ToolError::Failed(format!("database error: {e}")))?
            .get("n");

    // Deadlines come from tasks, not from the coursework cache. The task is
    // the obligation; the coursework row is where it came from. Reading tasks
    // means a manually created deadline and an imported one appear in one
    // list, and it means a completed assignment drops out because the user
    // ticked the task.
    let rows = sqlx::query(
        "select t.id, t.title, t.due_at, t.status, t.source, t.external_id, \
                t.external_account_id, c.name as course_name, cw.alternate_link \
         from tasks t \
         left join classroom_coursework cw \
                on cw.user_id = t.user_id and cw.external_id = t.external_id \
         left join classroom_courses c \
                on c.user_id = t.user_id and c.external_id = cw.course_external_id \
         where t.user_id = $1 and t.status <> 'archived'",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .map_err(|e| ToolError::Failed(format!("database error: {e}")))?;

    let mut deadlines: Vec<AcademicDeadline> = rows
        .iter()
        .map(|row| {
            let status: String = row.get("status");
            let due_at: Option<OffsetDateTime> = row.get("due_at");
            let source: String = row.get("source");
            let is_completed = status == "completed";
            AcademicDeadline {
                task_id: Some(row.get("id")),
                source: match source.as_str() {
                    CLASSROOM_PROVIDER => AcademicSource::GoogleClassroom,
                    "gmail" => AcademicSource::Gmail,
                    "calendar" => AcademicSource::Calendar,
                    "drive" => AcademicSource::Drive,
                    _ => AcademicSource::Manual,
                },
                external_id: row.get("external_id"),
                account_id: row.get("external_account_id"),
                title: row.get("title"),
                context: row.get("course_name"),
                due_at,
                is_overdue: !is_completed && due_at.is_some_and(|d| d < now),
                is_completed,
                alternate_link: row.get("alternate_link"),
            }
        })
        .collect();

    let (due_this_week, overdue) = summarize(&deadlines, now);

    // Nearest first; anything without a deadline sorts last rather than being
    // dropped, because an assignment with no due date is still outstanding.
    deadlines.retain(|d| !d.is_completed);
    deadlines.sort_by_key(|d| (d.due_at.is_none(), d.due_at));
    deadlines.truncate(upcoming_limit.clamp(1, 50) as usize);

    let announcement_rows = sqlx::query(
        "select external_id, course_external_id, account_id, text_content, author_name, \
                alternate_link, materials, source_created_at, source_updated_at, synced_at \
         from classroom_announcements where user_id = $1 \
         order by source_created_at desc nulls last limit 5",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .map_err(|e| ToolError::Failed(format!("database error: {e}")))?;

    let recent_announcements: Vec<Announcement> = announcement_rows
        .iter()
        .map(announcement_from_row)
        .collect();

    let oldest_synced_at: Option<OffsetDateTime> =
        sqlx::query("select min(last_synced_at) as t from academic_sync_state where user_id = $1")
            .bind(user_id)
            .fetch_one(pool)
            .await
            .map_err(|e| ToolError::Failed(format!("database error: {e}")))?
            .get("t");

    Ok(AcademicOverview {
        course_count: course_count as usize,
        due_this_week,
        overdue,
        recent_announcement_count: recent_announcements.len(),
        upcoming: deadlines,
        recent_announcements,
        oldest_synced_at,
    })
}

// ---------------------------------------------------------------------------
// The tool-facing service
// ---------------------------------------------------------------------------

/// Binds the pool and the Classroom provider together so the `academic.*`
/// tools reach exactly the functions above.
///
/// This exists so `assistant-tools` never needs sqlx: the trait is declared
/// there, the implementation lives here next to the pool. See ADR-0034.
pub struct AcademicService {
    pool: PgPool,
    classroom: std::sync::Arc<dyn ClassroomProvider>,
    announcements_per_course: u32,
}

impl AcademicService {
    pub fn new(pool: PgPool, classroom: std::sync::Arc<dyn ClassroomProvider>) -> Self {
        Self {
            pool,
            classroom,
            announcements_per_course: 5,
        }
    }
}

#[async_trait::async_trait]
impl assistant_tools::AcademicProvider for AcademicService {
    async fn deadlines(
        &self,
        user_id: Uuid,
        limit: i64,
    ) -> Result<Vec<AcademicDeadline>, ToolError> {
        Ok(overview(&self.pool, user_id, limit).await?.upcoming)
    }

    async fn assignments(
        &self,
        account_id: Uuid,
        user_id: Uuid,
        course_external_id: Option<&str>,
    ) -> Result<Vec<CourseworkItem>, ToolError> {
        assert_account_active(&self.pool, user_id, account_id).await?;
        cached_coursework(&self.pool, user_id, account_id, course_external_id).await
    }

    async fn sync(&self, account_id: Uuid, user_id: Uuid) -> Result<AcademicSyncResult, ToolError> {
        sync_account(
            &self.pool,
            &*self.classroom,
            user_id,
            account_id,
            self.announcements_per_course,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(offset_days: i64) -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(1_800_000_000).unwrap() + Duration::days(offset_days)
    }

    fn imported(title: &str, due: Option<OffsetDateTime>) -> ImportedTaskState {
        ImportedTaskState {
            title: title.into(),
            due_at: due,
            source_title: Some(title.into()),
            source_due_at: due,
        }
    }

    #[test]
    fn unchanged_coursework_writes_nothing() {
        let state = imported("Assignment 2", Some(t(3)));
        let plan = plan_task_update(&state, "Assignment 2", Some(t(3)));
        assert!(!plan.writes_anything());
        assert!(!plan.skipped_user_edit);
    }

    #[test]
    fn moved_deadline_updates_the_same_task() {
        // Friday -> Wednesday. The task must move, not be duplicated; there is
        // no insert path here at all.
        let state = imported("Assignment 2", Some(t(5)));
        let plan = plan_task_update(&state, "Assignment 2", Some(t(3)));
        assert_eq!(plan.due_at, Some(Some(t(3))));
        assert!(plan.title.is_none());
    }

    #[test]
    fn a_deadline_can_be_removed() {
        let state = imported("Assignment 2", Some(t(5)));
        let plan = plan_task_update(&state, "Assignment 2", None);
        assert_eq!(plan.due_at, Some(None), "clearing is a real write");
    }

    #[test]
    fn user_edited_title_is_not_overwritten() {
        let state = ImportedTaskState {
            title: "Assignment 2 (start early!)".into(),
            due_at: Some(t(5)),
            source_title: Some("Assignment 2".into()),
            source_due_at: Some(t(5)),
        };
        let plan = plan_task_update(&state, "Assignment 2 revised", Some(t(5)));
        assert!(plan.title.is_none(), "the user's title stands");
        assert!(plan.skipped_user_edit);
    }

    #[test]
    fn user_edited_title_still_allows_the_deadline_to_move() {
        // The two fields are decided independently: editing the title must not
        // freeze the due date, which is the field the scheduler depends on.
        let state = ImportedTaskState {
            title: "My own title".into(),
            due_at: Some(t(5)),
            source_title: Some("Assignment 2".into()),
            source_due_at: Some(t(5)),
        };
        let plan = plan_task_update(&state, "Assignment 2", Some(t(2)));
        assert!(plan.title.is_none());
        assert_eq!(plan.due_at, Some(Some(t(2))));
    }

    #[test]
    fn user_edited_due_date_is_not_overwritten() {
        let state = ImportedTaskState {
            title: "Assignment 2".into(),
            due_at: Some(t(1)),
            source_title: Some("Assignment 2".into()),
            source_due_at: Some(t(5)),
        };
        let plan = plan_task_update(&state, "Assignment 2", Some(t(6)));
        assert!(plan.due_at.is_none(), "the user's date stands");
        assert!(plan.skipped_user_edit);
    }

    #[test]
    fn row_without_provenance_is_treated_as_user_owned() {
        // A task that predates provenance tracking has no source_title. It must
        // not be assumed provider-owned, or the first sync would overwrite a
        // title the user wrote themselves.
        let state = ImportedTaskState {
            title: "Hand written".into(),
            due_at: None,
            source_title: None,
            source_due_at: None,
        };
        let plan = plan_task_update(&state, "From Classroom", None);
        assert!(plan.title.is_none());
    }

    #[test]
    fn summarize_counts_overdue_and_the_next_seven_days() {
        let now = t(0);
        let mk = |days: i64, completed: bool| AcademicDeadline {
            task_id: None,
            source: AcademicSource::GoogleClassroom,
            external_id: None,
            account_id: None,
            title: "x".into(),
            context: None,
            due_at: Some(t(days)),
            is_overdue: days < 0,
            is_completed: completed,
            alternate_link: None,
        };
        let items = vec![
            mk(-2, false), // overdue
            mk(1, false),  // this week
            mk(6, false),  // this week
            mk(30, false), // later
            mk(-5, true),  // overdue but done -> not counted
        ];
        let (week, overdue) = summarize(&items, now);
        assert_eq!(overdue, 1);
        assert_eq!(week, 2);
    }

    #[test]
    fn deadlineless_items_are_not_counted_but_are_not_lost() {
        let now = t(0);
        let item = AcademicDeadline {
            task_id: None,
            source: AcademicSource::GoogleClassroom,
            external_id: None,
            account_id: None,
            title: "No due date".into(),
            context: None,
            due_at: None,
            is_overdue: false,
            is_completed: false,
            alternate_link: None,
        };
        let (week, overdue) = summarize(std::slice::from_ref(&item), now);
        assert_eq!((week, overdue), (0, 0));
    }
}
