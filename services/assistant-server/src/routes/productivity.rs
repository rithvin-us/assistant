//! REST endpoints for Standalone Productivity features.
//!
//! Projects, Labels, Tasks, Reminders, Notes and Ideas. Projects and labels are
//! rows owned by a user rather than strings repeated on every item; see
//! ADR-0028. `TaskItem::project` and `NoteItem::tags` are read projections
//! resolved by join, not stored columns.
//!
//! Every filter here is a bind parameter. Earlier revisions of this file built
//! `where` clauses with `format!` and hand-rolled `''` escaping behind
//! `sqlx::AssertSqlSafe`, which is an injection surface reachable from a query
//! string: escaping quotes is not the same property as never letting the value
//! be parsed as SQL. The optional filters are expressed as
//! `($n::type is null or column = $n)` so one static statement covers every
//! combination and nothing user-supplied is ever concatenated.

use assistant_auth::Principal;
use assistant_protocol::{IdeaItem, LabelItem, NoteItem, ProjectItem, ReminderItem, TaskItem};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Row, Transaction, postgres::PgRow};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{error::AppError, state::SharedState};

/// Maps a database error to the status the caller actually deserves.
///
/// These tables carry real `check` constraints -- `priority in ('P1'..'P4')`,
/// `status in ('todo', 'completed', 'archived')` -- and foreign keys from a
/// reminder to its task and from a task to its project. A client that sends a
/// value outside one of those enums has made a bad request; reporting it as 500
/// tells the client nothing and buries genuine server faults in an error rate
/// made mostly of client typos. Everything else is still opaque and still
/// logged by `AppError`.
fn db_error(error: sqlx::Error) -> AppError {
    if let sqlx::Error::Database(ref db) = error
        && (db.is_check_violation() || db.is_foreign_key_violation() || db.is_unique_violation())
    {
        let constraint = db.constraint().unwrap_or("a constraint");
        return AppError::BadRequest(format!("value rejected by {constraint}"));
    }
    AppError::Internal(error.into())
}

/// Distinguishes "field absent" from "field present and explicitly null".
///
/// `Option<Option<T>>` on its own does not: serde folds a JSON `null` into the
/// outer `None`, so `{"due_at": null}` and `{}` both arrive as `None` and a due
/// date can never be cleared -- only overwritten. Paired with `default`, this
/// makes an absent field `None` and an explicit `null` `Some(None)`.
fn double_option<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Deserialize::deserialize(deserializer).map(Some)
}

/// Verifies that a client-supplied `task_id` belongs to the caller.
///
/// `reminders.task_id` has a foreign key to `tasks(id)` and nothing more, so
/// without this a caller could attach a reminder to another user's task. The
/// FK would accept it, and the difference between "accepted" and "violates the
/// foreign key" is an oracle for which task ids exist. Absent is fine; present
/// and not yours is `NotFound`, the same answer as present and non-existent.
async fn verify_task_ownership(
    pool: &PgPool,
    user_id: uuid::Uuid,
    task_id: Option<uuid::Uuid>,
) -> Result<(), AppError> {
    let Some(task_id) = task_id else {
        return Ok(());
    };
    sqlx::query("select 1 from tasks where id = $1 and user_id = $2")
        .bind(task_id)
        .bind(user_id)
        .fetch_optional(pool)
        .await
        .map_err(db_error)?
        .map(|_| ())
        .ok_or(AppError::NotFound)
}

fn db_pool(state: &SharedState) -> Result<&PgPool, AppError> {
    state
        .db
        .as_ref()
        .ok_or_else(|| AppError::DependencyUnavailable("the database"))
}

/// Neutralises `ilike` metacharacters in a user-supplied search term.
///
/// The term is already a bind parameter, so this is not about injection -- it
/// is about a search for "50%" matching every row. `\` is escaped first so it
/// does not double-escape the sequences added after it.
fn escape_like(term: &str) -> String {
    term.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// Guarantees the user row and that user's Inbox project exist, returning the
/// Inbox id.
///
/// `tasks.project_id` is `not null`, so a first-ever task needs a project to
/// point at before it can be inserted. Creating it here rather than at signup
/// keeps it true for users that predate the projects table.
async fn ensure_user(pool: &PgPool, user_id: Uuid) -> Result<Uuid, AppError> {
    sqlx::query("insert into users (id) values ($1) on conflict (id) do nothing")
        .bind(user_id)
        .execute(pool)
        .await
        .map_err(db_error)?;

    let row = sqlx::query(
        "with ins as (
             insert into projects (user_id, name, is_inbox, position)
             values ($1, 'Inbox', true, 0)
             on conflict (user_id, lower(name)) do nothing
             returning id
         )
         select id from ins
         union all
         select id from projects where user_id = $1 and is_inbox
         limit 1",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await
    .map_err(db_error)?;

    row.try_get("id").map_err(db_error)
}

// ---------------------------------------------------------------------------
// Read projections
// ---------------------------------------------------------------------------

/// `array_agg ... filter` rather than a plain `array_agg`: a `left join` that
/// matched nothing yields one row with a null label, and an unfiltered
/// aggregate would turn that into `{NULL}` instead of an empty array.
const TASK_SELECT: &str = "select t.id, t.user_id, t.title, t.description, t.priority, t.status,
        t.due_at, t.project_id, p.name as project, t.estimated_minutes,
        t.created_at, t.updated_at, t.completed_at,
        coalesce(array_agg(l.name order by l.name) filter (where l.id is not null), '{}'::text[]) as labels
   from tasks t
   join projects p on p.id = t.project_id
   left join task_labels tl on tl.task_id = t.id
   left join labels l on l.id = tl.label_id";

const NOTE_SELECT: &str = "select n.id, n.user_id, n.title, n.content, n.is_archived,
        n.created_at, n.updated_at,
        coalesce(array_agg(l.name order by l.name) filter (where l.id is not null), '{}'::text[]) as tags
   from notes n
   left join note_labels nl on nl.note_id = n.id
   left join labels l on l.id = nl.label_id";

fn project_from_row(row: &PgRow) -> Result<ProjectItem, sqlx::Error> {
    Ok(ProjectItem {
        id: row.try_get("id")?,
        user_id: row.try_get("user_id")?,
        name: row.try_get("name")?,
        color: row.try_get("color")?,
        is_inbox: row.try_get("is_inbox")?,
        position: row.try_get("position")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn label_from_row(row: &PgRow) -> Result<LabelItem, sqlx::Error> {
    Ok(LabelItem {
        id: row.try_get("id")?,
        user_id: row.try_get("user_id")?,
        name: row.try_get("name")?,
        color: row.try_get("color")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn task_from_row(row: &PgRow) -> Result<TaskItem, sqlx::Error> {
    let est: Option<i32> = row.try_get("estimated_minutes")?;
    Ok(TaskItem {
        id: row.try_get("id")?,
        user_id: row.try_get("user_id")?,
        title: row.try_get("title")?,
        description: row.try_get("description")?,
        priority: row.try_get("priority")?,
        status: row.try_get("status")?,
        due_at: row.try_get("due_at")?,
        project_id: row.try_get("project_id")?,
        project: row.try_get("project")?,
        labels: row.try_get("labels")?,
        estimated_minutes: est.map(|m| m.max(0) as u32),
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
        completed_at: row.try_get("completed_at")?,
    })
}

fn reminder_from_row(row: &PgRow) -> Result<ReminderItem, sqlx::Error> {
    Ok(ReminderItem {
        id: row.try_get("id")?,
        user_id: row.try_get("user_id")?,
        task_id: row.try_get("task_id")?,
        title: row.try_get("title")?,
        remind_at: row.try_get("remind_at")?,
        status: row.try_get("status")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn note_from_row(row: &PgRow) -> Result<NoteItem, sqlx::Error> {
    Ok(NoteItem {
        id: row.try_get("id")?,
        user_id: row.try_get("user_id")?,
        title: row.try_get("title")?,
        content: row.try_get("content")?,
        is_archived: row.try_get("is_archived")?,
        tags: row.try_get("tags")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn idea_from_row(row: &PgRow) -> Result<IdeaItem, sqlx::Error> {
    Ok(IdeaItem {
        id: row.try_get("id")?,
        user_id: row.try_get("user_id")?,
        title: row.try_get("title")?,
        description: row.try_get("description")?,
        status: row.try_get("status")?,
        converted_task_id: row.try_get("converted_task_id")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

/// Re-reads one task through `TASK_SELECT` after a write.
///
/// The write statements do not carry the joins, so `returning` cannot produce
/// the resolved project name or label list. Re-selecting inside the same
/// transaction is one extra round trip and keeps a single definition of the
/// read shape.
async fn task_by_id(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    id: Uuid,
) -> Result<TaskItem, AppError> {
    let sql = format!("{TASK_SELECT} where t.user_id = $1 and t.id = $2 group by t.id, p.name");
    let row = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(user_id)
        .bind(id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(db_error)?
        .ok_or(AppError::NotFound)?;
    task_from_row(&row).map_err(db_error)
}

async fn note_by_id(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    id: Uuid,
) -> Result<NoteItem, AppError> {
    let sql = format!("{NOTE_SELECT} where n.user_id = $1 and n.id = $2 group by n.id");
    let row = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(user_id)
        .bind(id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(db_error)?
        .ok_or(AppError::NotFound)?;
    note_from_row(&row).map_err(db_error)
}

// ---------------------------------------------------------------------------
// Name resolution
// ---------------------------------------------------------------------------

/// Resolves a project reference to an id, creating the project if the caller
/// named one that does not exist yet.
///
/// An explicit `project_id` is checked against `user_id` before use -- a client
/// may send any UUID, and a task must never be filed into somebody else's
/// project.
async fn resolve_project(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    project_id: Option<Uuid>,
    project_name: Option<&str>,
    fallback: Uuid,
) -> Result<Uuid, AppError> {
    if let Some(id) = project_id {
        let owned = sqlx::query("select id from projects where id = $1 and user_id = $2")
            .bind(id)
            .bind(user_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(db_error)?;
        return match owned {
            Some(_) => Ok(id),
            None => Err(AppError::NotFound),
        };
    }

    let Some(name) = project_name.map(str::trim).filter(|n| !n.is_empty()) else {
        return Ok(fallback);
    };

    // `do nothing` then `union all` rather than `do update`: a no-op update
    // would still fire the `updated_at` trigger and make a read look like a
    // write in the row's history.
    let row = sqlx::query(
        "with ins as (
             insert into projects (user_id, name) values ($1, $2)
             on conflict (user_id, lower(name)) do nothing
             returning id
         )
         select id from ins
         union all
         select id from projects where user_id = $1 and lower(name) = lower($2)
         limit 1",
    )
    .bind(user_id)
    .bind(name)
    .fetch_one(&mut **tx)
    .await
    .map_err(db_error)?;

    row.try_get("id").map_err(db_error)
}

/// Upserts label names and returns their ids, preserving nothing about order.
///
/// Names are trimmed and deduplicated case-insensitively before the round trip,
/// because `["home", "Home"]` is one label to the person who typed it.
async fn resolve_labels(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    names: &[String],
) -> Result<Vec<Uuid>, AppError> {
    let mut seen: Vec<String> = Vec::new();
    for raw in names {
        let name = raw.trim();
        if name.is_empty() {
            continue;
        }
        if !seen.iter().any(|s| s.eq_ignore_ascii_case(name)) {
            seen.push(name.to_string());
        }
    }

    let mut ids = Vec::with_capacity(seen.len());
    for name in &seen {
        let row = sqlx::query(
            "with ins as (
                 insert into labels (user_id, name) values ($1, $2)
                 on conflict (user_id, lower(name)) do nothing
                 returning id
             )
             select id from ins
             union all
             select id from labels where user_id = $1 and lower(name) = lower($2)
             limit 1",
        )
        .bind(user_id)
        .bind(name)
        .fetch_one(&mut **tx)
        .await
        .map_err(db_error)?;
        ids.push(row.try_get("id").map_err(db_error)?);
    }
    Ok(ids)
}

/// Replaces an item's label set wholesale.
///
/// Delete-then-insert rather than a diff: the set is small, the whole thing
/// arrives in one request, and computing the difference would add a failure
/// mode without saving a round trip.
async fn set_task_labels(
    tx: &mut Transaction<'_, Postgres>,
    task_id: Uuid,
    label_ids: &[Uuid],
) -> Result<(), AppError> {
    sqlx::query("delete from task_labels where task_id = $1")
        .bind(task_id)
        .execute(&mut **tx)
        .await
        .map_err(db_error)?;

    for label_id in label_ids {
        sqlx::query(
            "insert into task_labels (task_id, label_id) values ($1, $2)
             on conflict do nothing",
        )
        .bind(task_id)
        .bind(label_id)
        .execute(&mut **tx)
        .await
        .map_err(db_error)?;
    }
    Ok(())
}

async fn set_note_labels(
    tx: &mut Transaction<'_, Postgres>,
    note_id: Uuid,
    label_ids: &[Uuid],
) -> Result<(), AppError> {
    sqlx::query("delete from note_labels where note_id = $1")
        .bind(note_id)
        .execute(&mut **tx)
        .await
        .map_err(db_error)?;

    for label_id in label_ids {
        sqlx::query(
            "insert into note_labels (note_id, label_id) values ($1, $2)
             on conflict do nothing",
        )
        .bind(note_id)
        .bind(label_id)
        .execute(&mut **tx)
        .await
        .map_err(db_error)?;
    }
    Ok(())
}

// ------------------- PROJECTS -------------------

#[derive(Debug, Deserialize)]
pub struct CreateProjectInput {
    pub name: String,
    pub color: Option<String>,
    pub position: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateProjectInput {
    pub name: Option<String>,
    pub color: Option<String>,
    pub position: Option<i32>,
}

pub async fn list_projects(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<Vec<ProjectItem>>, AppError> {
    let pool = db_pool(&state)?;
    ensure_user(pool, principal.user_id).await?;

    let rows = sqlx::query(
        "select id, user_id, name, color, is_inbox, position, created_at, updated_at
         from projects where user_id = $1
         order by is_inbox desc, position asc, lower(name) asc",
    )
    .bind(principal.user_id)
    .fetch_all(pool)
    .await
    .map_err(db_error)?;

    let projects = rows
        .iter()
        .map(project_from_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_error)?;
    Ok(Json(projects))
}

pub async fn create_project(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Json(input): Json<CreateProjectInput>,
) -> Result<Json<ProjectItem>, AppError> {
    let pool = db_pool(&state)?;
    ensure_user(pool, principal.user_id).await?;

    let name = input.name.trim();
    if name.is_empty() {
        return Err(AppError::BadRequest("project name is empty".into()));
    }

    let row = sqlx::query(
        "insert into projects (user_id, name, color, position)
         values ($1, $2, coalesce($3, '#808080'), coalesce($4, 0))
         returning id, user_id, name, color, is_inbox, position, created_at, updated_at",
    )
    .bind(principal.user_id)
    .bind(name)
    .bind(input.color)
    .bind(input.position)
    .fetch_one(pool)
    .await
    .map_err(db_error)?;

    Ok(Json(project_from_row(&row).map_err(db_error)?))
}

pub async fn update_project(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<Uuid>,
    Json(input): Json<UpdateProjectInput>,
) -> Result<Json<ProjectItem>, AppError> {
    let pool = db_pool(&state)?;

    if let Some(ref name) = input.name
        && name.trim().is_empty()
    {
        return Err(AppError::BadRequest("project name is empty".into()));
    }

    // `updated_at` is absent from this statement on purpose -- the
    // `projects_set_updated_at` trigger writes it. See ADR-0028.
    let row = sqlx::query(
        "update projects
            set name     = coalesce($1, name),
                color    = coalesce($2, color),
                position = coalesce($3, position)
          where id = $4 and user_id = $5
         returning id, user_id, name, color, is_inbox, position, created_at, updated_at",
    )
    .bind(input.name.as_deref().map(str::trim))
    .bind(input.color)
    .bind(input.position)
    .bind(id)
    .bind(principal.user_id)
    .fetch_optional(pool)
    .await
    .map_err(db_error)?
    .ok_or(AppError::NotFound)?;

    Ok(Json(project_from_row(&row).map_err(db_error)?))
}

/// Deleting a project moves its tasks to Inbox first.
///
/// `tasks.project_id` is `on delete restrict`, so this reassignment is the only
/// thing that makes the delete succeed -- which is the point: the tasks are
/// visibly rehomed here rather than silently destroyed by a cascade. Deleting
/// Inbox itself is refused; it is the destination.
pub async fn delete_project(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let pool = db_pool(&state)?;
    let inbox = ensure_user(pool, principal.user_id).await?;

    if id == inbox {
        return Err(AppError::BadRequest(
            "the Inbox project cannot be deleted".into(),
        ));
    }

    let mut tx = pool.begin().await.map_err(db_error)?;

    sqlx::query("update tasks set project_id = $1 where project_id = $2 and user_id = $3")
        .bind(inbox)
        .bind(id)
        .bind(principal.user_id)
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;

    let result =
        sqlx::query("delete from projects where id = $1 and user_id = $2 and not is_inbox")
            .bind(id)
            .bind(principal.user_id)
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }

    tx.commit().await.map_err(db_error)?;
    Ok(StatusCode::NO_CONTENT)
}

// ------------------- LABELS -------------------

#[derive(Debug, Deserialize)]
pub struct CreateLabelInput {
    pub name: String,
    pub color: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateLabelInput {
    pub name: Option<String>,
    pub color: Option<String>,
}

pub async fn list_labels(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<Vec<LabelItem>>, AppError> {
    let pool = db_pool(&state)?;
    ensure_user(pool, principal.user_id).await?;

    let rows = sqlx::query(
        "select id, user_id, name, color, created_at, updated_at
         from labels where user_id = $1 order by lower(name) asc",
    )
    .bind(principal.user_id)
    .fetch_all(pool)
    .await
    .map_err(db_error)?;

    let labels = rows
        .iter()
        .map(label_from_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_error)?;
    Ok(Json(labels))
}

pub async fn create_label(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Json(input): Json<CreateLabelInput>,
) -> Result<Json<LabelItem>, AppError> {
    let pool = db_pool(&state)?;
    ensure_user(pool, principal.user_id).await?;

    let name = input.name.trim();
    if name.is_empty() {
        return Err(AppError::BadRequest("label name is empty".into()));
    }

    let row = sqlx::query(
        "insert into labels (user_id, name, color)
         values ($1, $2, coalesce($3, '#808080'))
         returning id, user_id, name, color, created_at, updated_at",
    )
    .bind(principal.user_id)
    .bind(name)
    .bind(input.color)
    .fetch_one(pool)
    .await
    .map_err(db_error)?;

    Ok(Json(label_from_row(&row).map_err(db_error)?))
}

/// Renaming a label is one UPDATE, which is the whole reason it is a row: every
/// task and note carrying it follows automatically.
pub async fn update_label(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<Uuid>,
    Json(input): Json<UpdateLabelInput>,
) -> Result<Json<LabelItem>, AppError> {
    let pool = db_pool(&state)?;

    if let Some(ref name) = input.name
        && name.trim().is_empty()
    {
        return Err(AppError::BadRequest("label name is empty".into()));
    }

    let row = sqlx::query(
        "update labels
            set name  = coalesce($1, name),
                color = coalesce($2, color)
          where id = $3 and user_id = $4
         returning id, user_id, name, color, created_at, updated_at",
    )
    .bind(input.name.as_deref().map(str::trim))
    .bind(input.color)
    .bind(id)
    .bind(principal.user_id)
    .fetch_optional(pool)
    .await
    .map_err(db_error)?
    .ok_or(AppError::NotFound)?;

    Ok(Json(label_from_row(&row).map_err(db_error)?))
}

/// Both join tables are `on delete cascade`, so this detaches the label from
/// every task and note rather than failing. Unlike a project, a label carries
/// no content of its own -- there is nothing to rehome.
pub async fn delete_label(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let pool = db_pool(&state)?;
    let result = sqlx::query("delete from labels where id = $1 and user_id = $2")
        .bind(id)
        .bind(principal.user_id)
        .execute(pool)
        .await
        .map_err(db_error)?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

// ------------------- TASKS -------------------

#[derive(Debug, Deserialize)]
pub struct TaskFilter {
    pub status: Option<String>,
    pub priority: Option<String>,
    /// Project name. Kept alongside `project_id` because a saved filter or a
    /// shared link names a project the way a person does.
    pub project: Option<String>,
    pub project_id: Option<Uuid>,
    pub label: Option<String>,
    pub q: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateTaskInput {
    /// Client-generated id, so replaying a queued offline write is idempotent
    /// rather than producing a duplicate task. See ADR-0029.
    pub id: Option<Uuid>,
    pub title: String,
    pub description: Option<String>,
    pub priority: Option<String>,
    pub due_at: Option<OffsetDateTime>,
    pub project: Option<String>,
    pub project_id: Option<Uuid>,
    pub labels: Option<Vec<String>>,
    pub estimated_minutes: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateTaskInput {
    pub title: Option<String>,
    pub description: Option<String>,
    pub priority: Option<String>,
    pub status: Option<String>,
    /// Absent leaves the due date alone; an explicit `null` clears it.
    #[serde(default, deserialize_with = "double_option")]
    pub due_at: Option<Option<OffsetDateTime>>,
    pub project: Option<String>,
    pub project_id: Option<Uuid>,
    /// Absent leaves labels alone; `[]` removes them all.
    pub labels: Option<Vec<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub estimated_minutes: Option<Option<u32>>,
}

pub async fn list_tasks(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Query(filter): Query<TaskFilter>,
) -> Result<Json<Vec<TaskItem>>, AppError> {
    let pool = db_pool(&state)?;
    ensure_user(pool, principal.user_id).await?;

    // One static statement. Each optional filter is disarmed by its own bound
    // null rather than by omitting the clause, so no user value is ever
    // concatenated into SQL.
    let sql = format!(
        "{TASK_SELECT}
          where t.user_id = $1
            and ($2::text is null or t.status = $2)
            and ($3::text is null or t.priority = $3)
            and ($4::uuid is null or t.project_id = $4)
            and ($5::text is null or lower(p.name) = lower($5))
            and ($6::text is null or (t.title ilike '%' || $6 || '%' escape '\\'
                                   or t.description ilike '%' || $6 || '%' escape '\\'))
            and ($7::text is null or exists (
                    select 1 from task_labels tlf
                    join labels lf on lf.id = tlf.label_id
                    where tlf.task_id = t.id and lower(lf.name) = lower($7)))
          group by t.id, p.name
          order by t.status asc, t.due_at asc nulls last, t.created_at desc"
    );

    let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(principal.user_id)
        .bind(filter.status)
        .bind(filter.priority)
        .bind(filter.project_id)
        .bind(filter.project)
        .bind(filter.q.as_deref().map(escape_like))
        .bind(filter.label)
        .fetch_all(pool)
        .await
        .map_err(db_error)?;

    let tasks = rows
        .iter()
        .map(task_from_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_error)?;
    Ok(Json(tasks))
}

pub async fn create_task(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Json(input): Json<CreateTaskInput>,
) -> Result<Json<TaskItem>, AppError> {
    let pool = db_pool(&state)?;
    let inbox = ensure_user(pool, principal.user_id).await?;

    let mut tx = pool.begin().await.map_err(db_error)?;

    let project_id = resolve_project(
        &mut tx,
        principal.user_id,
        input.project_id,
        input.project.as_deref(),
        inbox,
    )
    .await?;

    // `on conflict (id) do update` makes a replayed offline create idempotent.
    // The `where` guards it: a client may send any UUID, and without the owner
    // check a guessed id would let one user overwrite another user's task.
    // A conflict on somebody else's row updates nothing and returns no row.
    //
    // The rejection says only that the id is unavailable. Saying it belongs to
    // another user would turn this endpoint into an existence oracle for any
    // UUID -- the same reason `AppError::NotFound` is deliberately
    // indistinguishable from "owned by somebody else" elsewhere.
    let row = sqlx::query(
        "insert into tasks (id, user_id, title, description, priority, status, due_at, project_id, estimated_minutes)
         values (coalesce($1, gen_random_uuid()), $2, $3, $4, coalesce($5, 'P4'), 'todo', $6, $7, $8)
         on conflict (id) do update
            set title             = excluded.title,
                description       = excluded.description,
                priority          = excluded.priority,
                due_at            = excluded.due_at,
                project_id        = excluded.project_id,
                estimated_minutes = excluded.estimated_minutes
          where tasks.user_id = excluded.user_id
         returning id",
    )
    .bind(input.id)
    .bind(principal.user_id)
    .bind(input.title)
    .bind(input.description.unwrap_or_default())
    .bind(input.priority)
    .bind(input.due_at)
    .bind(project_id)
    .bind(input.estimated_minutes.map(|m| m as i32))
    .fetch_optional(&mut *tx)
    .await
    .map_err(db_error)?
    .ok_or_else(|| AppError::BadRequest("task id is not available".into()))?;

    let id: Uuid = row.try_get("id").map_err(db_error)?;

    if let Some(ref names) = input.labels {
        let label_ids = resolve_labels(&mut tx, principal.user_id, names).await?;
        set_task_labels(&mut tx, id, &label_ids).await?;
    }

    let task = task_by_id(&mut tx, principal.user_id, id).await?;
    tx.commit().await.map_err(db_error)?;
    Ok(Json(task))
}

pub async fn update_task(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<Uuid>,
    Json(input): Json<UpdateTaskInput>,
) -> Result<Json<TaskItem>, AppError> {
    let pool = db_pool(&state)?;
    let inbox = ensure_user(pool, principal.user_id).await?;

    let mut tx = pool.begin().await.map_err(db_error)?;

    let current = sqlx::query("select * from tasks where id = $1 and user_id = $2 for update")
        .bind(id)
        .bind(principal.user_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db_error)?
        .ok_or(AppError::NotFound)?;

    let status: String = input.status.unwrap_or_else(|| current.get("status"));

    let project_id = if input.project_id.is_some() || input.project.is_some() {
        resolve_project(
            &mut tx,
            principal.user_id,
            input.project_id,
            input.project.as_deref(),
            inbox,
        )
        .await?
    } else {
        current.get("project_id")
    };

    let due_at: Option<OffsetDateTime> = match input.due_at {
        Some(val) => val,
        None => current.get("due_at"),
    };

    // `completed_at` records when the task was completed, so it is written on
    // the transition *into* `completed` and never recomputed afterwards.
    //
    // Deriving it from the resulting status alone had two failure modes, both
    // silent and both destroying data already written: editing the title of a
    // completed task resolved `status` from the current row, saw "completed",
    // and overwrote the original timestamp with `now()`; and archiving a
    // completed task took the `else` branch and erased it outright.
    let previously_completed: Option<OffsetDateTime> = current.get("completed_at");
    let completed_at: Option<OffsetDateTime> = match status.as_str() {
        "completed" => previously_completed.or_else(|| Some(OffsetDateTime::now_utc())),
        // Reopening a task genuinely un-completes it.
        "todo" => None,
        // Archiving keeps the record of when the work was finished.
        _ => previously_completed,
    };

    let estimated_minutes: Option<i32> = match input.estimated_minutes {
        Some(val) => val.map(|m| m as i32),
        None => current.get("estimated_minutes"),
    };

    // No `updated_at` here: `tasks_set_updated_at` writes it (ADR-0028).
    sqlx::query(
        "update tasks
            set title             = coalesce($1, title),
                description       = coalesce($2, description),
                priority          = coalesce($3, priority),
                status            = $4,
                due_at            = $5,
                project_id        = $6,
                estimated_minutes = $7,
                completed_at      = $8
          where id = $9 and user_id = $10",
    )
    .bind(input.title)
    .bind(input.description)
    .bind(input.priority)
    .bind(status)
    .bind(due_at)
    .bind(project_id)
    .bind(estimated_minutes)
    .bind(completed_at)
    .bind(id)
    .bind(principal.user_id)
    .execute(&mut *tx)
    .await
    .map_err(db_error)?;

    if let Some(ref names) = input.labels {
        let label_ids = resolve_labels(&mut tx, principal.user_id, names).await?;
        set_task_labels(&mut tx, id, &label_ids).await?;
    }

    let task = task_by_id(&mut tx, principal.user_id, id).await?;
    tx.commit().await.map_err(db_error)?;
    Ok(Json(task))
}

pub async fn delete_task(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let pool = db_pool(&state)?;
    let result = sqlx::query("delete from tasks where id = $1 and user_id = $2")
        .bind(id)
        .bind(principal.user_id)
        .execute(pool)
        .await
        .map_err(db_error)?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }

    Ok(StatusCode::NO_CONTENT)
}

// ------------------- REMINDERS -------------------

#[derive(Debug, Deserialize)]
pub struct ReminderFilter {
    pub status: Option<String>,
    pub q: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateReminderInput {
    pub id: Option<Uuid>,
    pub title: String,
    pub remind_at: OffsetDateTime,
    pub task_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateReminderInput {
    pub title: Option<String>,
    pub remind_at: Option<OffsetDateTime>,
    pub status: Option<String>,
    /// Absent leaves the link alone; an explicit `null` detaches the reminder
    /// from its task.
    #[serde(default, deserialize_with = "double_option")]
    pub task_id: Option<Option<Uuid>>,
}

pub async fn list_reminders(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Query(filter): Query<ReminderFilter>,
) -> Result<Json<Vec<ReminderItem>>, AppError> {
    let pool = db_pool(&state)?;
    ensure_user(pool, principal.user_id).await?;

    let rows = sqlx::query(
        "select id, user_id, task_id, title, remind_at, status, created_at, updated_at
           from reminders
          where user_id = $1
            and ($2::text is null or status = $2)
            and ($3::text is null or title ilike '%' || $3 || '%' escape '\\')
          order by status asc, remind_at asc",
    )
    .bind(principal.user_id)
    .bind(filter.status)
    .bind(filter.q.as_deref().map(escape_like))
    .fetch_all(pool)
    .await
    .map_err(db_error)?;

    let reminders = rows
        .iter()
        .map(reminder_from_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_error)?;
    Ok(Json(reminders))
}

pub async fn create_reminder(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Json(input): Json<CreateReminderInput>,
) -> Result<Json<ReminderItem>, AppError> {
    let pool = db_pool(&state)?;
    ensure_user(pool, principal.user_id).await?;
    verify_task_ownership(pool, principal.user_id, input.task_id).await?;

    let row = sqlx::query(
        "insert into reminders (id, user_id, title, remind_at, task_id, status)
         values (coalesce($1, gen_random_uuid()), $2, $3, $4, $5, 'pending')
         on conflict (id) do update
            set title     = excluded.title,
                remind_at = excluded.remind_at,
                task_id   = excluded.task_id
          where reminders.user_id = excluded.user_id
         returning id, user_id, task_id, title, remind_at, status, created_at, updated_at",
    )
    .bind(input.id)
    .bind(principal.user_id)
    .bind(input.title)
    .bind(input.remind_at)
    .bind(input.task_id)
    .fetch_optional(pool)
    .await
    .map_err(db_error)?
    .ok_or_else(|| AppError::BadRequest("reminder id is not available".into()))?;

    let reminder = reminder_from_row(&row).map_err(db_error)?;
    Ok(Json(reminder))
}

pub async fn update_reminder(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<Uuid>,
    Json(input): Json<UpdateReminderInput>,
) -> Result<Json<ReminderItem>, AppError> {
    let pool = db_pool(&state)?;

    let current_row = sqlx::query("select * from reminders where id = $1 and user_id = $2")
        .bind(id)
        .bind(principal.user_id)
        .fetch_optional(pool)
        .await
        .map_err(db_error)?;

    let Some(current) = current_row else {
        return Err(AppError::NotFound);
    };

    // The update rewrites `task_id` from client input, so it needs the same
    // proof the insert does.
    if let Some(task_id) = input.task_id {
        verify_task_ownership(pool, principal.user_id, task_id).await?;
    }

    let task_id: Option<Uuid> = match input.task_id {
        Some(val) => val,
        None => current.get("task_id"),
    };

    let row = sqlx::query(
        "update reminders
            set title     = coalesce($1, title),
                remind_at = coalesce($2, remind_at),
                status    = coalesce($3, status),
                task_id   = $4
          where id = $5 and user_id = $6
         returning id, user_id, task_id, title, remind_at, status, created_at, updated_at",
    )
    .bind(input.title)
    .bind(input.remind_at)
    .bind(input.status)
    .bind(task_id)
    .bind(id)
    .bind(principal.user_id)
    .fetch_one(pool)
    .await
    .map_err(db_error)?;

    let reminder = reminder_from_row(&row).map_err(db_error)?;
    Ok(Json(reminder))
}

pub async fn delete_reminder(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let pool = db_pool(&state)?;
    let result = sqlx::query("delete from reminders where id = $1 and user_id = $2")
        .bind(id)
        .bind(principal.user_id)
        .execute(pool)
        .await
        .map_err(db_error)?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }

    Ok(StatusCode::NO_CONTENT)
}

// ------------------- NOTES -------------------

#[derive(Debug, Deserialize)]
pub struct NoteFilter {
    pub is_archived: Option<bool>,
    pub tag: Option<String>,
    pub q: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateNoteInput {
    pub id: Option<Uuid>,
    pub title: String,
    pub content: Option<String>,
    /// Label names. Stored as `labels` + `note_labels` rows since ADR-0028;
    /// the field keeps its name because it is what the UI calls them.
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateNoteInput {
    pub title: Option<String>,
    pub content: Option<String>,
    pub is_archived: Option<bool>,
    pub tags: Option<Vec<String>>,
}

pub async fn list_notes(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Query(filter): Query<NoteFilter>,
) -> Result<Json<Vec<NoteItem>>, AppError> {
    let pool = db_pool(&state)?;
    ensure_user(pool, principal.user_id).await?;

    let sql = format!(
        "{NOTE_SELECT}
          where n.user_id = $1
            and ($2::boolean is null or n.is_archived = $2)
            and ($3::text is null or (n.title ilike '%' || $3 || '%' escape '\\'
                                   or n.content ilike '%' || $3 || '%' escape '\\'))
            and ($4::text is null or exists (
                    select 1 from note_labels nlf
                    join labels lf on lf.id = nlf.label_id
                    where nlf.note_id = n.id and lower(lf.name) = lower($4)))
          group by n.id
          order by n.updated_at desc"
    );

    let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(principal.user_id)
        .bind(filter.is_archived)
        .bind(filter.q.as_deref().map(escape_like))
        .bind(filter.tag)
        .fetch_all(pool)
        .await
        .map_err(db_error)?;

    let notes = rows
        .iter()
        .map(note_from_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_error)?;
    Ok(Json(notes))
}

pub async fn create_note(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Json(input): Json<CreateNoteInput>,
) -> Result<Json<NoteItem>, AppError> {
    let pool = db_pool(&state)?;
    ensure_user(pool, principal.user_id).await?;

    let mut tx = pool.begin().await.map_err(db_error)?;

    let row = sqlx::query(
        "insert into notes (id, user_id, title, content, is_archived)
         values (coalesce($1, gen_random_uuid()), $2, $3, $4, false)
         on conflict (id) do update
            set title   = excluded.title,
                content = excluded.content
          where notes.user_id = excluded.user_id
         returning id",
    )
    .bind(input.id)
    .bind(principal.user_id)
    .bind(input.title)
    .bind(input.content.unwrap_or_default())
    .fetch_optional(&mut *tx)
    .await
    .map_err(db_error)?
    .ok_or_else(|| AppError::BadRequest("note id is not available".into()))?;

    let id: Uuid = row.try_get("id").map_err(db_error)?;

    if let Some(ref names) = input.tags {
        let label_ids = resolve_labels(&mut tx, principal.user_id, names).await?;
        set_note_labels(&mut tx, id, &label_ids).await?;
    }

    let note = note_by_id(&mut tx, principal.user_id, id).await?;
    tx.commit().await.map_err(db_error)?;
    Ok(Json(note))
}

pub async fn update_note(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<Uuid>,
    Json(input): Json<UpdateNoteInput>,
) -> Result<Json<NoteItem>, AppError> {
    let pool = db_pool(&state)?;
    let mut tx = pool.begin().await.map_err(db_error)?;

    let updated = sqlx::query(
        "update notes
            set title       = coalesce($1, title),
                content     = coalesce($2, content),
                is_archived = coalesce($3, is_archived)
          where id = $4 and user_id = $5
         returning id",
    )
    .bind(input.title)
    .bind(input.content)
    .bind(input.is_archived)
    .bind(id)
    .bind(principal.user_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(db_error)?;

    if updated.is_none() {
        return Err(AppError::NotFound);
    }

    if let Some(ref names) = input.tags {
        let label_ids = resolve_labels(&mut tx, principal.user_id, names).await?;
        set_note_labels(&mut tx, id, &label_ids).await?;
    }

    let note = note_by_id(&mut tx, principal.user_id, id).await?;
    tx.commit().await.map_err(db_error)?;
    Ok(Json(note))
}

pub async fn delete_note(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let pool = db_pool(&state)?;
    let result = sqlx::query("delete from notes where id = $1 and user_id = $2")
        .bind(id)
        .bind(principal.user_id)
        .execute(pool)
        .await
        .map_err(db_error)?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }

    Ok(StatusCode::NO_CONTENT)
}

// ------------------- IDEAS -------------------

#[derive(Debug, Deserialize)]
pub struct IdeaFilter {
    pub status: Option<String>,
    pub q: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateIdeaInput {
    pub id: Option<Uuid>,
    pub title: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateIdeaInput {
    pub title: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ConvertIdeaResponse {
    pub idea: IdeaItem,
    pub task: TaskItem,
}

pub async fn list_ideas(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Query(filter): Query<IdeaFilter>,
) -> Result<Json<Vec<IdeaItem>>, AppError> {
    let pool = db_pool(&state)?;
    ensure_user(pool, principal.user_id).await?;

    let rows = sqlx::query(
        "select id, user_id, title, description, status, converted_task_id, created_at, updated_at
           from ideas
          where user_id = $1
            and ($2::text is null or status = $2)
            and ($3::text is null or (title ilike '%' || $3 || '%' escape '\\'
                                   or description ilike '%' || $3 || '%' escape '\\'))
          order by updated_at desc",
    )
    .bind(principal.user_id)
    .bind(filter.status)
    .bind(filter.q.as_deref().map(escape_like))
    .fetch_all(pool)
    .await
    .map_err(db_error)?;

    let ideas = rows
        .iter()
        .map(idea_from_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_error)?;
    Ok(Json(ideas))
}

pub async fn create_idea(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Json(input): Json<CreateIdeaInput>,
) -> Result<Json<IdeaItem>, AppError> {
    let pool = db_pool(&state)?;
    ensure_user(pool, principal.user_id).await?;

    let row = sqlx::query(
        "insert into ideas (id, user_id, title, description, status)
         values (coalesce($1, gen_random_uuid()), $2, $3, $4, 'active')
         on conflict (id) do update
            set title       = excluded.title,
                description = excluded.description
          where ideas.user_id = excluded.user_id
         returning id, user_id, title, description, status, converted_task_id, created_at, updated_at",
    )
    .bind(input.id)
    .bind(principal.user_id)
    .bind(input.title)
    .bind(input.description.unwrap_or_default())
    .fetch_optional(pool)
    .await
    .map_err(db_error)?
    .ok_or_else(|| AppError::BadRequest("idea id is not available".into()))?;

    let idea = idea_from_row(&row).map_err(db_error)?;
    Ok(Json(idea))
}

pub async fn update_idea(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<Uuid>,
    Json(input): Json<UpdateIdeaInput>,
) -> Result<Json<IdeaItem>, AppError> {
    let pool = db_pool(&state)?;

    let row = sqlx::query(
        "update ideas
            set title       = coalesce($1, title),
                description = coalesce($2, description),
                status      = coalesce($3, status)
          where id = $4 and user_id = $5
         returning id, user_id, title, description, status, converted_task_id, created_at, updated_at",
    )
    .bind(input.title)
    .bind(input.description)
    .bind(input.status)
    .bind(id)
    .bind(principal.user_id)
    .fetch_optional(pool)
    .await
    .map_err(db_error)?
    .ok_or(AppError::NotFound)?;

    let idea = idea_from_row(&row).map_err(db_error)?;
    Ok(Json(idea))
}

pub async fn convert_idea_to_task(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<Uuid>,
) -> Result<Json<ConvertIdeaResponse>, AppError> {
    let pool = db_pool(&state)?;
    let inbox = ensure_user(pool, principal.user_id).await?;
    let mut tx = pool.begin().await.map_err(db_error)?;

    let idea_row = sqlx::query("select * from ideas where id = $1 and user_id = $2 for update")
        .bind(id)
        .bind(principal.user_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db_error)?;

    let Some(idea_current) = idea_row else {
        return Err(AppError::NotFound);
    };

    let idea_obj = idea_from_row(&idea_current).map_err(db_error)?;

    // Converting the same idea twice must not produce two tasks. The row above
    // is held `for update`, so a concurrent second call blocks there and then
    // arrives here seeing the committed `converted_task_id` rather than racing
    // past it into a second insert. A double-tap on the phone is one task.
    if let Some(existing_task_id) = idea_obj.converted_task_id {
        // `converted_task_id` is `on delete set null`, so a missing task here
        // means the link was already cleared; fall through and convert again.
        match task_by_id(&mut tx, principal.user_id, existing_task_id).await {
            Ok(task) => {
                tx.commit().await.map_err(db_error)?;
                return Ok(Json(ConvertIdeaResponse {
                    idea: idea_obj,
                    task,
                }));
            }
            Err(AppError::NotFound) => {}
            Err(other) => return Err(other),
        }
    }

    let task_row = sqlx::query(
        "insert into tasks (user_id, title, description, priority, status, project_id)
         values ($1, $2, $3, 'P4', 'todo', $4)
         returning id",
    )
    .bind(principal.user_id)
    .bind(idea_obj.title)
    .bind(idea_obj.description)
    .bind(inbox)
    .fetch_one(&mut *tx)
    .await
    .map_err(db_error)?;

    let task_id: Uuid = task_row.try_get("id").map_err(db_error)?;
    let task_obj = task_by_id(&mut tx, principal.user_id, task_id).await?;

    let updated_idea_row = sqlx::query(
        "update ideas set status = 'converted', converted_task_id = $1
          where id = $2 and user_id = $3
         returning id, user_id, title, description, status, converted_task_id, created_at, updated_at",
    )
    .bind(task_obj.id)
    .bind(id)
    .bind(principal.user_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(db_error)?;

    let updated_idea = idea_from_row(&updated_idea_row).map_err(db_error)?;

    tx.commit().await.map_err(db_error)?;

    Ok(Json(ConvertIdeaResponse {
        idea: updated_idea,
        task: task_obj,
    }))
}

pub async fn delete_idea(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let pool = db_pool(&state)?;
    let result = sqlx::query("delete from ideas where id = $1 and user_id = $2")
        .bind(id)
        .bind(principal.user_id)
        .execute(pool)
        .await
        .map_err(db_error)?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }

    Ok(StatusCode::NO_CONTENT)
}
