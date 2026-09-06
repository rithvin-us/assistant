//! REST endpoints for Standalone Productivity features (Tasks, Reminders, Notes, Ideas).

use assistant_auth::Principal;
use assistant_protocol::{IdeaItem, NoteItem, ReminderItem, TaskItem};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row, postgres::PgRow};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{error::AppError, state::SharedState};

/// Maps a database error to the status the caller actually deserves.
///
/// These tables carry real `check` constraints -- `priority in ('P1'..'P4')`,
/// `status in ('todo', 'completed', 'archived')` -- and a foreign key from a
/// reminder to its task. A client that sends a value outside one of those
/// enums has made a bad request; reporting it as 500 tells the client nothing
/// and buries genuine server faults in an error rate made mostly of client
/// typos. Everything else is still opaque and still logged by `AppError`.
fn db_error(error: sqlx::Error) -> AppError {
    if let sqlx::Error::Database(ref db) = error
        && (db.is_check_violation() || db.is_foreign_key_violation())
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

async fn ensure_user(pool: &PgPool, user_id: Uuid) -> Result<(), AppError> {
    sqlx::query("insert into users (id) values ($1) on conflict (id) do nothing")
        .bind(user_id)
        .execute(pool)
        .await
        .map_err(db_error)?;
    Ok(())
}

fn db_pool(state: &SharedState) -> Result<&PgPool, AppError> {
    state
        .db
        .as_ref()
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("database unavailable")))
}

fn task_from_row(row: &PgRow) -> Result<TaskItem, sqlx::Error> {
    Ok(TaskItem {
        id: row.try_get("id")?,
        user_id: row.try_get("user_id")?,
        title: row.try_get("title")?,
        description: row.try_get("description")?,
        priority: row.try_get("priority")?,
        status: row.try_get("status")?,
        due_at: row.try_get("due_at")?,
        project: row.try_get("project")?,
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

// ------------------- TASKS -------------------

#[derive(Debug, Deserialize)]
pub struct TaskFilter {
    pub status: Option<String>,
    pub priority: Option<String>,
    pub project: Option<String>,
    pub q: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateTaskInput {
    pub title: String,
    pub description: Option<String>,
    pub priority: Option<String>,
    pub due_at: Option<OffsetDateTime>,
    pub project: Option<String>,
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
}

pub async fn list_tasks(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Query(filter): Query<TaskFilter>,
) -> Result<Json<Vec<TaskItem>>, AppError> {
    let pool = db_pool(&state)?;
    ensure_user(pool, principal.user_id).await?;

    let mut query = String::from(
        "select id, user_id, title, description, priority, status, due_at, project, created_at, updated_at, completed_at
         from tasks where user_id = $1",
    );

    if let Some(ref st) = filter.status {
        query.push_str(&format!(" and status = '{}'", st.replace('\'', "''")));
    }
    if let Some(ref pr) = filter.priority {
        query.push_str(&format!(" and priority = '{}'", pr.replace('\'', "''")));
    }
    if let Some(ref proj) = filter.project {
        query.push_str(&format!(" and project = '{}'", proj.replace('\'', "''")));
    }
    if let Some(ref search) = filter.q {
        let escaped = search.replace('\'', "''");
        query.push_str(&format!(
            " and (title ilike '%{escaped}%' or description ilike '%{escaped}%')"
        ));
    }
    query.push_str(" order by status asc, due_at asc nulls last, created_at desc");

    let rows = sqlx::query(sqlx::AssertSqlSafe(query.as_str()))
        .bind(principal.user_id)
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
    ensure_user(pool, principal.user_id).await?;

    let priority = input.priority.unwrap_or_else(|| "P4".into());
    let description = input.description.unwrap_or_default();
    let project = input.project.unwrap_or_else(|| "Inbox".into());

    let row = sqlx::query(
        "insert into tasks (user_id, title, description, priority, status, due_at, project)
         values ($1, $2, $3, $4, 'todo', $5, $6)
         returning id, user_id, title, description, priority, status, due_at, project, created_at, updated_at, completed_at",
    )
    .bind(principal.user_id)
    .bind(input.title)
    .bind(description)
    .bind(priority)
    .bind(input.due_at)
    .bind(project)
    .fetch_one(pool)
    .await
    .map_err(db_error)?;

    let task = task_from_row(&row).map_err(db_error)?;
    Ok(Json(task))
}

pub async fn update_task(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<Uuid>,
    Json(input): Json<UpdateTaskInput>,
) -> Result<Json<TaskItem>, AppError> {
    let pool = db_pool(&state)?;

    let current_row = sqlx::query("select * from tasks where id = $1 and user_id = $2")
        .bind(id)
        .bind(principal.user_id)
        .fetch_optional(pool)
        .await
        .map_err(db_error)?;

    let Some(current) = current_row else {
        return Err(AppError::NotFound);
    };

    let title: String = input.title.unwrap_or_else(|| current.get("title"));
    let description: String = input
        .description
        .unwrap_or_else(|| current.get("description"));
    let priority: String = input.priority.unwrap_or_else(|| current.get("priority"));
    let status: String = input.status.unwrap_or_else(|| current.get("status"));
    let project: String = input.project.unwrap_or_else(|| current.get("project"));
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

    let row = sqlx::query(
        "update tasks set title = $1, description = $2, priority = $3, status = $4, due_at = $5, project = $6, updated_at = now(), completed_at = $7
         where id = $8 and user_id = $9
         returning id, user_id, title, description, priority, status, due_at, project, created_at, updated_at, completed_at",
    )
    .bind(title)
    .bind(description)
    .bind(priority)
    .bind(status)
    .bind(due_at)
    .bind(project)
    .bind(completed_at)
    .bind(id)
    .bind(principal.user_id)
    .fetch_one(pool)
    .await
    .map_err(db_error)?;

    let task = task_from_row(&row).map_err(db_error)?;
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

    let mut query = String::from(
        "select id, user_id, task_id, title, remind_at, status, created_at, updated_at
         from reminders where user_id = $1",
    );

    if let Some(ref st) = filter.status {
        query.push_str(&format!(" and status = '{}'", st.replace('\'', "''")));
    }
    if let Some(ref search) = filter.q {
        let escaped = search.replace('\'', "''");
        query.push_str(&format!(" and title ilike '%{escaped}%'"));
    }
    query.push_str(" order by status asc, remind_at asc");

    let rows = sqlx::query(sqlx::AssertSqlSafe(query.as_str()))
        .bind(principal.user_id)
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

    let row = sqlx::query(
        "insert into reminders (user_id, title, remind_at, task_id, status)
         values ($1, $2, $3, $4, 'pending')
         returning id, user_id, task_id, title, remind_at, status, created_at, updated_at",
    )
    .bind(principal.user_id)
    .bind(input.title)
    .bind(input.remind_at)
    .bind(input.task_id)
    .fetch_one(pool)
    .await
    .map_err(db_error)?;

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

    let title: String = input.title.unwrap_or_else(|| current.get("title"));
    let remind_at: OffsetDateTime = input.remind_at.unwrap_or_else(|| current.get("remind_at"));
    let status: String = input.status.unwrap_or_else(|| current.get("status"));
    let task_id: Option<Uuid> = match input.task_id {
        Some(val) => val,
        None => current.get("task_id"),
    };

    let row = sqlx::query(
        "update reminders set title = $1, remind_at = $2, status = $3, task_id = $4, updated_at = now()
         where id = $5 and user_id = $6
         returning id, user_id, task_id, title, remind_at, status, created_at, updated_at",
    )
    .bind(title)
    .bind(remind_at)
    .bind(status)
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
    pub q: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateNoteInput {
    pub title: String,
    pub content: Option<String>,
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

    let mut query = String::from(
        "select id, user_id, title, content, is_archived, tags, created_at, updated_at
         from notes where user_id = $1",
    );

    if let Some(archived) = filter.is_archived {
        query.push_str(&format!(" and is_archived = {}", archived));
    }
    if let Some(ref search) = filter.q {
        let escaped = search.replace('\'', "''");
        query.push_str(&format!(
            " and (title ilike '%{escaped}%' or content ilike '%{escaped}%')"
        ));
    }
    query.push_str(" order by updated_at desc");

    let rows = sqlx::query(sqlx::AssertSqlSafe(query.as_str()))
        .bind(principal.user_id)
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

    let content = input.content.unwrap_or_default();
    let tags = input.tags.unwrap_or_default();

    let row = sqlx::query(
        "insert into notes (user_id, title, content, is_archived, tags)
         values ($1, $2, $3, false, $4)
         returning id, user_id, title, content, is_archived, tags, created_at, updated_at",
    )
    .bind(principal.user_id)
    .bind(input.title)
    .bind(content)
    .bind(tags)
    .fetch_one(pool)
    .await
    .map_err(db_error)?;

    let note = note_from_row(&row).map_err(db_error)?;
    Ok(Json(note))
}

pub async fn update_note(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<Uuid>,
    Json(input): Json<UpdateNoteInput>,
) -> Result<Json<NoteItem>, AppError> {
    let pool = db_pool(&state)?;

    let current_row = sqlx::query("select * from notes where id = $1 and user_id = $2")
        .bind(id)
        .bind(principal.user_id)
        .fetch_optional(pool)
        .await
        .map_err(db_error)?;

    let Some(current) = current_row else {
        return Err(AppError::NotFound);
    };

    let title: String = input.title.unwrap_or_else(|| current.get("title"));
    let content: String = input.content.unwrap_or_else(|| current.get("content"));
    let is_archived: bool = input
        .is_archived
        .unwrap_or_else(|| current.get("is_archived"));
    let tags: Vec<String> = input.tags.unwrap_or_else(|| current.get("tags"));

    let row = sqlx::query(
        "update notes set title = $1, content = $2, is_archived = $3, tags = $4, updated_at = now()
         where id = $5 and user_id = $6
         returning id, user_id, title, content, is_archived, tags, created_at, updated_at",
    )
    .bind(title)
    .bind(content)
    .bind(is_archived)
    .bind(tags)
    .bind(id)
    .bind(principal.user_id)
    .fetch_one(pool)
    .await
    .map_err(db_error)?;

    let note = note_from_row(&row).map_err(db_error)?;
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

    let mut query = String::from(
        "select id, user_id, title, description, status, converted_task_id, created_at, updated_at
         from ideas where user_id = $1",
    );

    if let Some(ref st) = filter.status {
        query.push_str(&format!(" and status = '{}'", st.replace('\'', "''")));
    }
    if let Some(ref search) = filter.q {
        let escaped = search.replace('\'', "''");
        query.push_str(&format!(
            " and (title ilike '%{escaped}%' or description ilike '%{escaped}%')"
        ));
    }
    query.push_str(" order by updated_at desc");

    let rows = sqlx::query(sqlx::AssertSqlSafe(query.as_str()))
        .bind(principal.user_id)
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

    let description = input.description.unwrap_or_default();

    let row = sqlx::query(
        "insert into ideas (user_id, title, description, status)
         values ($1, $2, $3, 'active')
         returning id, user_id, title, description, status, converted_task_id, created_at, updated_at",
    )
    .bind(principal.user_id)
    .bind(input.title)
    .bind(description)
    .fetch_one(pool)
    .await
    .map_err(db_error)?;

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

    let current_row = sqlx::query("select * from ideas where id = $1 and user_id = $2")
        .bind(id)
        .bind(principal.user_id)
        .fetch_optional(pool)
        .await
        .map_err(db_error)?;

    let Some(current) = current_row else {
        return Err(AppError::NotFound);
    };

    let title: String = input.title.unwrap_or_else(|| current.get("title"));
    let description: String = input
        .description
        .unwrap_or_else(|| current.get("description"));
    let status: String = input.status.unwrap_or_else(|| current.get("status"));

    let row = sqlx::query(
        "update ideas set title = $1, description = $2, status = $3, updated_at = now()
         where id = $4 and user_id = $5
         returning id, user_id, title, description, status, converted_task_id, created_at, updated_at",
    )
    .bind(title)
    .bind(description)
    .bind(status)
    .bind(id)
    .bind(principal.user_id)
    .fetch_one(pool)
    .await
    .map_err(db_error)?;

    let idea = idea_from_row(&row).map_err(db_error)?;
    Ok(Json(idea))
}

pub async fn convert_idea_to_task(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<Uuid>,
) -> Result<Json<ConvertIdeaResponse>, AppError> {
    let pool = db_pool(&state)?;
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
        let existing = sqlx::query(
            "select id, user_id, title, description, priority, status, due_at, project, created_at, updated_at, completed_at
             from tasks where id = $1 and user_id = $2",
        )
        .bind(existing_task_id)
        .bind(principal.user_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db_error)?;

        // `converted_task_id` is `on delete set null`, so a missing task here
        // means the link was already cleared; fall through and convert again.
        if let Some(existing) = existing {
            let task = task_from_row(&existing).map_err(db_error)?;
            tx.commit().await.map_err(db_error)?;
            return Ok(Json(ConvertIdeaResponse {
                idea: idea_obj,
                task,
            }));
        }
    }

    let task_row = sqlx::query(
        "insert into tasks (user_id, title, description, priority, status, project)
         values ($1, $2, $3, 'P4', 'todo', 'Inbox')
         returning id, user_id, title, description, priority, status, due_at, project, created_at, updated_at, completed_at",
    )
    .bind(principal.user_id)
    .bind(idea_obj.title)
    .bind(idea_obj.description)
    .fetch_one(&mut *tx)
    .await
    .map_err(db_error)?;

    let task_obj = task_from_row(&task_row).map_err(db_error)?;

    let updated_idea_row = sqlx::query(
        "update ideas set status = 'converted', converted_task_id = $1, updated_at = now()
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
