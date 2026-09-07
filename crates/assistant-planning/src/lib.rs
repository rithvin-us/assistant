//! Deterministic, provider-independent unified planning domain and calculation engine.
//!
//! What lives here:
//! - Strongly typed planning models ([`PlanningItem`], [`Deadline`], [`AvailabilityWindow`],
//!   [`Commitment`], [`EffortEstimate`], [`PlanBlock`], [`Conflict`], [`FeasibilityResult`]).
//! - Entity adapters mapping existing domain entities (tasks, calendar events, coursework,
//!   reminders) into unified planning structures.
//! - Deterministic availability calculator ([`availability`]).
//! - Deterministic conflict detection engine ([`conflicts`]).
//! - Deterministic schedule feasibility evaluator ([`feasibility`]).
//! - Deterministic first-pass planner & ranking engine ([`planner`]).
//!
//! What deliberately does NOT live here:
//! - LLM prompts, model clients, network calls, or direct database queries.
//! - The planning engine is strictly deterministic and provider-independent.

pub mod adapters;
pub mod availability;
pub mod conflicts;
pub mod feasibility;
pub mod model;
pub mod planner;

pub use adapters::*;
pub use availability::*;
pub use conflicts::*;
pub use feasibility::*;
pub use model::*;
pub use planner::*;

#[cfg(test)]
mod tests {
    use super::*;
    use time::{Duration, OffsetDateTime, Time};

    #[test]
    fn test_availability_calculation_subtracts_commitments() {
        let now = OffsetDateTime::now_utc();
        let start = now
            .date()
            .with_time(Time::from_hms(8, 0, 0).unwrap())
            .assume_utc();
        let end = now
            .date()
            .with_time(Time::from_hms(18, 0, 0).unwrap())
            .assume_utc();

        let meeting_start = now
            .date()
            .with_time(Time::from_hms(10, 0, 0).unwrap())
            .assume_utc();
        let meeting_end = now
            .date()
            .with_time(Time::from_hms(11, 30, 0).unwrap())
            .assume_utc();

        let commitments = vec![Commitment {
            id: "meeting-1".to_string(),
            title: "Team Sync".to_string(),
            start_time: meeting_start,
            end_time: meeting_end,
            is_all_day: false,
            location: None,
            source: ItemSource::GoogleCalendar,
        }];

        let working_hours = WorkingHours::default();
        let windows = calculate_availability_windows(start, end, &commitments, &working_hours);

        let total_mins = total_available_minutes(&windows);
        // Total working window 08:00 to 18:00 is 10 hours = 600 mins.
        // Subtract 1.5 hours (90 mins) meeting -> 510 mins usable.
        assert_eq!(total_mins, 510);
        assert_eq!(windows.len(), 2);
    }

    #[test]
    fn test_feasibility_detects_overbooked_schedule() {
        let now = OffsetDateTime::now_utc();
        let due = now + Duration::hours(5);

        let items = vec![
            PlanningItem {
                id: uuid::Uuid::new_v4(),
                user_id: uuid::Uuid::new_v4(),
                title: "Big Assignment".to_string(),
                description: None,
                source: ItemSource::StandaloneTask,
                source_ref: None,
                priority: PlanningPriority::Urgent,
                deadline: Some(Deadline {
                    due_at: due,
                    kind: DeadlineKind::Hard,
                    precision: DeadlinePrecision::ExactDateTime,
                    provenance: "test".to_string(),
                    confidence: Some(1.0),
                }),
                effort: EffortEstimate::Known { minutes: 300 }, // 5 hours
                project_id: None,
                project_name: None,
                is_completed: false,
                dependencies: Vec::new(),
            },
            PlanningItem {
                id: uuid::Uuid::new_v4(),
                user_id: uuid::Uuid::new_v4(),
                title: "Second Task".to_string(),
                description: None,
                source: ItemSource::StandaloneTask,
                source_ref: None,
                priority: PlanningPriority::High,
                deadline: None,
                effort: EffortEstimate::Known { minutes: 120 }, // 2 hours
                project_id: None,
                project_name: None,
                is_completed: false,
                dependencies: Vec::new(),
            },
        ];

        let windows = vec![AvailabilityWindow {
            start_time: now,
            end_time: now + Duration::hours(4),
            duration_minutes: 240, // Only 4 hours available
            is_usable: true,
            source: "test".to_string(),
        }];

        let conflicts = detect_conflicts(&items, &[], &windows, &[], now);
        let feasibility = calculate_feasibility(&items, &windows, &conflicts);

        assert_eq!(feasibility.state, FeasibilityState::Infeasible);
        assert!(!conflicts.is_empty());
    }

    #[test]
    fn test_unknown_effort_yields_uncertain_feasibility() {
        let now = OffsetDateTime::now_utc();
        let items = vec![PlanningItem {
            id: uuid::Uuid::new_v4(),
            user_id: uuid::Uuid::new_v4(),
            title: "Unestimated Task".to_string(),
            description: None,
            source: ItemSource::StandaloneTask,
            source_ref: None,
            priority: PlanningPriority::Medium,
            deadline: None,
            effort: EffortEstimate::Unknown,
            project_id: None,
            project_name: None,
            is_completed: false,
            dependencies: Vec::new(),
        }];

        let windows = vec![AvailabilityWindow {
            start_time: now,
            end_time: now + Duration::hours(1),
            duration_minutes: 60,
            is_usable: true,
            source: "test".to_string(),
        }];

        let conflicts = detect_conflicts(&items, &[], &windows, &[], now);
        let feasibility = calculate_feasibility(&items, &windows, &conflicts);

        assert_eq!(feasibility.unknown_effort_count, 1);
        assert_ne!(feasibility.state, FeasibilityState::Feasible);
    }
}
