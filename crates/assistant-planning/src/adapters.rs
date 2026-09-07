//! Adapters converting existing system entities into unified planning models.

use crate::model::*;
use assistant_protocol::{CalendarEvent, CourseworkItem, ReminderItem, TaskItem};

/// Adapts a [`TaskItem`] into a [`PlanningItem`].
pub fn task_to_planning_item(task: &TaskItem) -> PlanningItem {
    let deadline = task.due_at.map(|due| Deadline {
        due_at: due,
        kind: DeadlineKind::Hard,
        precision: DeadlinePrecision::ExactDateTime,
        provenance: format!("task:{}", task.id),
        confidence: Some(1.0),
    });

    let effort = match task.estimated_minutes {
        Some(mins) if mins > 0 => EffortEstimate::Known { minutes: mins },
        _ => EffortEstimate::Unknown,
    };

    PlanningItem {
        id: task.id,
        user_id: task.user_id,
        title: task.title.clone(),
        description: if task.description.is_empty() {
            None
        } else {
            Some(task.description.clone())
        },
        source: ItemSource::StandaloneTask,
        source_ref: Some(task.id.to_string()),
        priority: PlanningPriority::parse(&task.priority),
        deadline,
        effort,
        project_id: Some(task.project_id),
        project_name: if task.project.is_empty() {
            None
        } else {
            Some(task.project.clone())
        },
        is_completed: task.status == "completed" || task.status == "archived",
        dependencies: Vec::new(),
    }
}

/// Adapts a [`CalendarEvent`] into a [`Commitment`].
pub fn calendar_event_to_commitment(event: &CalendarEvent) -> Commitment {
    Commitment {
        id: event.id.clone(),
        title: event.title.clone(),
        start_time: event.start_time,
        end_time: event.end_time,
        is_all_day: event.all_day,
        location: event.location.clone(),
        source: ItemSource::GoogleCalendar,
    }
}

/// Adapts a [`CourseworkItem`] assignment into a [`PlanningItem`].
pub fn coursework_to_planning_item(user_id: UserId, coursework: &CourseworkItem) -> PlanningItem {
    let deadline = coursework.due_at.map(|due| Deadline {
        due_at: due,
        kind: DeadlineKind::Hard,
        precision: DeadlinePrecision::ExactDateTime,
        provenance: format!("coursework:{}", coursework.external_id),
        confidence: Some(1.0),
    });

    PlanningItem {
        id: uuid::Uuid::new_v5(
            &uuid::Uuid::NAMESPACE_OID,
            coursework.external_id.as_bytes(),
        ),
        user_id,
        title: coursework.title.clone(),
        description: coursework.description.clone(),
        source: ItemSource::GoogleClassroom,
        source_ref: Some(coursework.external_id.clone()),
        priority: PlanningPriority::High,
        deadline,
        effort: EffortEstimate::Unknown,
        project_id: None,
        project_name: Some("Academic".to_string()),
        is_completed: coursework.state == "TURNED_IN" || coursework.state == "RETURNED",
        dependencies: Vec::new(),
    }
}

/// Adapts a [`ReminderItem`] into a [`Commitment`].
pub fn reminder_to_commitment(reminder: &ReminderItem) -> Commitment {
    let end = reminder.remind_at + time::Duration::minutes(15);
    Commitment {
        id: reminder.id.to_string(),
        title: reminder.title.clone(),
        start_time: reminder.remind_at,
        end_time: end,
        is_all_day: false,
        location: None,
        source: ItemSource::StandaloneTask,
    }
}
