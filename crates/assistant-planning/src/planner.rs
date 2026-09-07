//! Deterministic first-pass planner & ranking engine.

use crate::availability::{calculate_availability_windows, WorkingHours};
use crate::conflicts::detect_conflicts;
use crate::feasibility::calculate_feasibility;
use crate::model::*;
use time::{Duration, OffsetDateTime};

/// Deterministically ranks planning items.
pub fn rank_planning_items(items: &[PlanningItem], now: OffsetDateTime) -> Vec<PlanningItem> {
    let mut ranked = items.to_vec();

    ranked.sort_by(|a, b| {
        let score_a = calculate_rank_score(a, now);
        let score_b = calculate_rank_score(b, now);
        score_b.cmp(&score_a)
    });

    ranked
}

fn calculate_rank_score(item: &PlanningItem, now: OffsetDateTime) -> i64 {
    let mut score: i64 = 0;

    // 1. Priority base weight
    score += match item.priority {
        PlanningPriority::Urgent => 1000,
        PlanningPriority::High => 500,
        PlanningPriority::Medium => 200,
        PlanningPriority::Low => 50,
    };

    // 2. Deadline score
    if let Some(deadline) = &item.deadline {
        let hours_until_due = (deadline.due_at - now).whole_hours();
        if hours_until_due < 0 {
            // Overdue boost
            score += 2000;
        } else if hours_until_due <= 24 {
            score += 1500;
        } else if hours_until_due <= 48 {
            score += 800;
        } else if hours_until_due <= 168 {
            score += 300;
        }

        if deadline.kind == DeadlineKind::Hard {
            score += 500;
        }
    }

    score
}

/// Generates a deterministic [`TodayPlan`] snapshot for a user.
pub fn generate_today_plan(
    items: &[PlanningItem],
    commitments: &[Commitment],
    now: OffsetDateTime,
    working_hours: &WorkingHours,
) -> TodayPlan {
    let day_start = now.date().with_time(time::Time::MIDNIGHT).assume_utc();
    let day_end = day_start + Duration::days(1);

    let windows = calculate_availability_windows(now, day_end, commitments, working_hours);
    let ranked_items = rank_planning_items(items, now);

    let mut recommended_blocks = Vec::new();
    let mut current_window_idx = 0;
    let mut window_cursor = if !windows.is_empty() {
        windows[0].start_time
    } else {
        now
    };

    for item in &ranked_items {
        if item.is_completed {
            continue;
        }

        let duration_needed = match item.effort {
            EffortEstimate::Known { minutes } => minutes.max(15),
            EffortEstimate::Unknown => 45, // Suggested focus block duration for unestimated item
        };

        if current_window_idx >= windows.len() {
            break;
        }

        let window = &windows[current_window_idx];
        let window_remaining = (window.end_time - window_cursor).whole_minutes() as u32;

        if window_remaining >= 15 {
            let block_duration = duration_needed.min(window_remaining);
            let block_end = window_cursor + Duration::minutes(block_duration as i64);

            recommended_blocks.push(PlanBlock {
                item_id: item.id,
                title: item.title.clone(),
                start_time: window_cursor,
                end_time: block_end,
                duration_minutes: block_duration,
                rationale: format!(
                    "Prioritized due to {:?} priority and deadline proximity.",
                    item.priority
                ),
                confidence: if item.effort == EffortEstimate::Unknown {
                    0.5
                } else {
                    0.9
                },
                source: item.source,
            });

            window_cursor = block_end;
            if window_cursor >= window.end_time {
                current_window_idx += 1;
                if current_window_idx < windows.len() {
                    window_cursor = windows[current_window_idx].start_time;
                }
            }
        } else {
            current_window_idx += 1;
            if current_window_idx < windows.len() {
                window_cursor = windows[current_window_idx].start_time;
            }
        }
    }

    let conflicts = detect_conflicts(items, commitments, &windows, &recommended_blocks, now);
    let feasibility = calculate_feasibility(items, &windows, &conflicts);

    let upcoming_deadlines = items
        .iter()
        .filter_map(|i| i.deadline.clone())
        .filter(|d| d.due_at >= now && d.due_at <= day_end)
        .collect();

    let total_available = windows.iter().map(|w| w.duration_minutes).sum();

    TodayPlan {
        date: now,
        commitments: commitments.to_vec(),
        recommended_blocks,
        upcoming_deadlines,
        total_available_minutes: total_available,
        conflicts,
        feasibility,
    }
}

/// Generates an [`UpcomingPlanning`] outlook snapshot for `horizon_days`.
pub fn generate_upcoming_planning(
    items: &[PlanningItem],
    commitments: &[Commitment],
    now: OffsetDateTime,
    horizon_days: u32,
    working_hours: &WorkingHours,
) -> UpcomingPlanning {
    let end = now + Duration::days(horizon_days as i64);
    let windows = calculate_availability_windows(now, end, commitments, working_hours);

    let deadline_items: Vec<PlanningItem> = items
        .iter()
        .filter(|i| !i.is_completed)
        .filter(|i| {
            if let Some(d) = &i.deadline {
                d.due_at <= end
            } else {
                false
            }
        })
        .cloned()
        .collect();

    let conflicts = detect_conflicts(items, commitments, &windows, &[], now);
    let feasibility = calculate_feasibility(items, &windows, &conflicts);

    let mut daily_workload_minutes = Vec::new();
    let mut cursor = now.date();
    while cursor <= end.date() {
        let day_str = cursor.to_string();
        let day_start = cursor.with_time(time::Time::MIDNIGHT).assume_utc();
        let day_end = day_start + Duration::days(1);

        let day_effort: u32 = items
            .iter()
            .filter(|i| !i.is_completed)
            .filter(|i| {
                if let Some(d) = &i.deadline {
                    d.due_at >= day_start && d.due_at < day_end
                } else {
                    false
                }
            })
            .filter_map(|i| i.effort.known_minutes())
            .sum();

        daily_workload_minutes.push((day_str, day_effort));

        if let Some(next) = cursor.next_day() {
            cursor = next;
        } else {
            break;
        }
    }

    UpcomingPlanning {
        horizon_days,
        total_items: items.len(),
        deadlines: deadline_items,
        daily_workload_minutes,
        conflicts,
        feasibility,
    }
}
