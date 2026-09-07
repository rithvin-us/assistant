//! Server-side planning aggregator for M9 Unified Personal Planning Engine.

use assistant_planning::{
    Commitment, PlanningItem, WorkingHours, calculate_availability_windows, calculate_feasibility,
    detect_conflicts, generate_today_plan, generate_upcoming_planning, reminder_to_commitment,
    task_to_planning_item,
};
use assistant_protocol::{ReminderItem, TaskItem};
use assistant_tools::{PlanningProvider, ToolError};
use async_trait::async_trait;
use sqlx::{PgPool, Row};
use time::OffsetDateTime;
use uuid::Uuid;

pub struct ServerPlanningAggregator {
    pool: PgPool,
}

impl ServerPlanningAggregator {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn fetch_active_tasks(&self, user_id: Uuid) -> Result<Vec<TaskItem>, String> {
        let rows = sqlx::query(
            r#"
            SELECT t.id, t.user_id, t.title, t.description, t.priority, t.status,
                   t.due_at, t.project_id, p.name AS project_name, t.estimated_minutes,
                   t.created_at, t.updated_at, t.completed_at,
                   COALESCE(
                       array_agg(l.name ORDER BY l.name) FILTER (WHERE l.name IS NOT NULL),
                       ARRAY[]::text[]
                   ) AS labels
            FROM tasks t
            JOIN projects p ON p.id = t.project_id
            LEFT JOIN task_labels tl ON tl.task_id = t.id
            LEFT JOIN labels l ON l.id = tl.label_id
            WHERE t.user_id = $1 AND t.status = 'todo'
            GROUP BY t.id, p.name
            ORDER BY t.created_at DESC
            "#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| format!("database error fetching tasks: {e}"))?;

        let mut tasks = Vec::new();
        for row in rows {
            let priority: String = row.try_get("priority").unwrap_or_else(|_| "P4".to_string());
            let status: String = row.try_get("status").unwrap_or_else(|_| "todo".to_string());
            let est_mins: Option<i32> = row.try_get("estimated_minutes").ok().flatten();
            tasks.push(TaskItem {
                id: row.try_get("id").map_err(|e| e.to_string())?,
                user_id: row.try_get("user_id").map_err(|e| e.to_string())?,
                title: row.try_get("title").map_err(|e| e.to_string())?,
                description: row.try_get("description").unwrap_or_default(),
                priority,
                status,
                due_at: row.try_get("due_at").ok().flatten(),
                project_id: row.try_get("project_id").map_err(|e| e.to_string())?,
                project: row.try_get("project_name").unwrap_or_default(),
                labels: row.try_get("labels").unwrap_or_default(),
                estimated_minutes: est_mins.map(|m| m.max(0) as u32),
                created_at: row.try_get("created_at").map_err(|e| e.to_string())?,
                updated_at: row.try_get("updated_at").map_err(|e| e.to_string())?,
                completed_at: row.try_get("completed_at").ok().flatten(),
            });
        }
        Ok(tasks)
    }

    pub async fn fetch_user_commitments(&self, user_id: Uuid) -> Result<Vec<Commitment>, String> {
        let rows = sqlx::query(
            r#"
            SELECT id, title, remind_at, status
            FROM reminders
            WHERE user_id = $1 AND status = 'pending'
            ORDER BY remind_at ASC
            "#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| format!("database error fetching reminders: {e}"))?;

        let mut commitments = Vec::new();
        for row in rows {
            let r = ReminderItem {
                id: row.try_get("id").map_err(|e| e.to_string())?,
                user_id,
                task_id: None,
                title: row.try_get("title").map_err(|e| e.to_string())?,
                remind_at: row.try_get("remind_at").map_err(|e| e.to_string())?,
                status: row.try_get("status").map_err(|e| e.to_string())?,
                created_at: OffsetDateTime::now_utc(),
                updated_at: OffsetDateTime::now_utc(),
            };
            commitments.push(reminder_to_commitment(&r));
        }

        Ok(commitments)
    }

    pub async fn build_planning_context(
        &self,
        user_id: Uuid,
    ) -> Result<(Vec<PlanningItem>, Vec<Commitment>), String> {
        let tasks = self.fetch_active_tasks(user_id).await?;
        let commitments = self.fetch_user_commitments(user_id).await?;

        let planning_items: Vec<PlanningItem> = tasks.iter().map(task_to_planning_item).collect();
        Ok((planning_items, commitments))
    }
}

#[async_trait]
impl PlanningProvider for ServerPlanningAggregator {
    async fn get_today_plan(&self, user_id: Uuid) -> Result<serde_json::Value, ToolError> {
        let (items, commitments) = self
            .build_planning_context(user_id)
            .await
            .map_err(ToolError::Failed)?;

        let now = OffsetDateTime::now_utc();
        let working_hours = WorkingHours::default();
        let plan = generate_today_plan(&items, &commitments, now, &working_hours);

        serde_json::to_value(&plan).map_err(|e| ToolError::Failed(e.to_string()))
    }

    async fn get_upcoming_planning(
        &self,
        user_id: Uuid,
        horizon_days: u32,
    ) -> Result<serde_json::Value, ToolError> {
        let (items, commitments) = self
            .build_planning_context(user_id)
            .await
            .map_err(ToolError::Failed)?;

        let now = OffsetDateTime::now_utc();
        let working_hours = WorkingHours::default();
        let upcoming =
            generate_upcoming_planning(&items, &commitments, now, horizon_days, &working_hours);

        serde_json::to_value(&upcoming).map_err(|e| ToolError::Failed(e.to_string()))
    }

    async fn analyze_schedule(
        &self,
        user_id: Uuid,
        horizon_days: u32,
    ) -> Result<serde_json::Value, ToolError> {
        self.get_upcoming_planning(user_id, horizon_days).await
    }

    async fn check_feasibility(&self, user_id: Uuid) -> Result<serde_json::Value, ToolError> {
        let (items, commitments) = self
            .build_planning_context(user_id)
            .await
            .map_err(ToolError::Failed)?;

        let now = OffsetDateTime::now_utc();
        let working_hours = WorkingHours::default();
        let windows = calculate_availability_windows(
            now,
            now + time::Duration::days(7),
            &commitments,
            &working_hours,
        );
        let conflicts = detect_conflicts(&items, &commitments, &windows, &[], now);
        let feasibility = calculate_feasibility(&items, &windows, &conflicts);

        serde_json::to_value(&feasibility).map_err(|e| ToolError::Failed(e.to_string()))
    }

    async fn detect_conflicts(&self, user_id: Uuid) -> Result<serde_json::Value, ToolError> {
        let (items, commitments) = self
            .build_planning_context(user_id)
            .await
            .map_err(ToolError::Failed)?;

        let now = OffsetDateTime::now_utc();
        let working_hours = WorkingHours::default();
        let windows = calculate_availability_windows(
            now,
            now + time::Duration::days(7),
            &commitments,
            &working_hours,
        );
        let conflicts = detect_conflicts(&items, &commitments, &windows, &[], now);

        serde_json::to_value(&conflicts).map_err(|e| ToolError::Failed(e.to_string()))
    }
}
