//! Deterministic conflict detection engine.

use crate::model::*;
use time::OffsetDateTime;

/// Detects all planning conflicts across items, commitments, availability, and plan blocks.
pub fn detect_conflicts(
    items: &[PlanningItem],
    commitments: &[Commitment],
    available_windows: &[AvailabilityWindow],
    plan_blocks: &[PlanBlock],
    now: OffsetDateTime,
) -> Vec<Conflict> {
    let mut conflicts = Vec::new();

    // 1. Detect overdue items with hard deadlines
    for item in items {
        if let Some(deadline) = &item.deadline {
            if !item.is_completed && deadline.due_at < now {
                conflicts.push(Conflict {
                    conflict_type: ConflictType::ScheduledPastDeadline,
                    severity: ConflictSeverity::Critical,
                    affected_item_ids: vec![item.id],
                    reason: format!(
                        "Item '{}' was due at {} but remains incomplete.",
                        item.title, deadline.due_at
                    ),
                });
            }
        }
    }

    // 2. Detect overlapping commitments
    let mut sorted_commitments = commitments.to_vec();
    sorted_commitments.sort_by_key(|c| c.start_time);
    for window in sorted_commitments.windows(2) {
        let c1 = &window[0];
        let c2 = &window[1];
        if !c1.is_all_day && !c2.is_all_day && c1.end_time > c2.start_time {
            conflicts.push(Conflict {
                conflict_type: ConflictType::OverlappingCommitments,
                severity: ConflictSeverity::Critical,
                affected_item_ids: Vec::new(),
                reason: format!(
                    "Commitment '{}' ({}-{}) overlaps with '{}' ({}-{}).",
                    c1.title, c1.start_time, c1.end_time, c2.title, c2.start_time, c2.end_time
                ),
            });
        }
    }

    // 3. Detect insufficient time before hard deadlines
    let total_available: u32 = available_windows.iter().map(|w| w.duration_minutes).sum();
    let mut required_effort: u32 = 0;
    let mut unknown_count: usize = 0;

    for item in items.iter().filter(|i| !i.is_completed) {
        match item.effort {
            EffortEstimate::Known { minutes } => required_effort += minutes,
            EffortEstimate::Unknown => unknown_count += 1,
        }
    }

    if required_effort > total_available {
        conflicts.push(Conflict {
            conflict_type: ConflictType::InsufficientTimeBeforeDeadline,
            severity: ConflictSeverity::Critical,
            affected_item_ids: items.iter().map(|i| i.id).collect(),
            reason: format!(
                "Required effort ({} mins) exceeds total available usable time ({} mins).",
                required_effort, total_available
            ),
        });
    } else if unknown_count > 0 && required_effort + (unknown_count as u32 * 30) > total_available {
        conflicts.push(Conflict {
            conflict_type: ConflictType::InsufficientTimeBeforeDeadline,
            severity: ConflictSeverity::Warning,
            affected_item_ids: items.iter().map(|i| i.id).collect(),
            reason: format!(
                "Available time ({} mins) is tight relative to known effort ({} mins) with {} unestimated items.",
                total_available, required_effort, unknown_count
            ),
        });
    }

    // 4. Detect overlapping plan blocks
    let mut sorted_blocks = plan_blocks.to_vec();
    sorted_blocks.sort_by_key(|b| b.start_time);
    for window in sorted_blocks.windows(2) {
        let b1 = &window[0];
        let b2 = &window[1];
        if b1.end_time > b2.start_time {
            conflicts.push(Conflict {
                conflict_type: ConflictType::OverlappingCommitments,
                severity: ConflictSeverity::Critical,
                affected_item_ids: vec![b1.item_id, b2.item_id],
                reason: format!(
                    "Scheduled block '{}' overlaps with '{}'.",
                    b1.title, b2.title
                ),
            });
        }
    }

    conflicts
}
