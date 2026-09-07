//! Feasibility calculation engine.

use crate::model::*;

/// Evaluates feasibility based on total effort, available windows, unknown estimates, and conflicts.
pub fn calculate_feasibility(
    items: &[PlanningItem],
    available_windows: &[AvailabilityWindow],
    conflicts: &[Conflict],
) -> FeasibilityResult {
    let mut total_known_effort_minutes: u32 = 0;
    let mut unknown_effort_count: usize = 0;

    for item in items.iter().filter(|i| !i.is_completed) {
        match item.effort {
            EffortEstimate::Known { minutes } => total_known_effort_minutes += minutes,
            EffortEstimate::Unknown => unknown_effort_count += 1,
        }
    }

    let total_available_minutes: u32 = available_windows.iter().map(|w| w.duration_minutes).sum();

    let has_critical_conflicts = conflicts
        .iter()
        .any(|c| c.severity == ConflictSeverity::Critical);

    let state = if has_critical_conflicts || total_known_effort_minutes > total_available_minutes {
        FeasibilityState::Infeasible
    } else if unknown_effort_count > 0 {
        if total_known_effort_minutes + (unknown_effort_count as u32 * 60) > total_available_minutes
        {
            FeasibilityState::Uncertain
        } else {
            FeasibilityState::LikelyFeasible
        }
    } else {
        FeasibilityState::Feasible
    };

    let explanation = match state {
        FeasibilityState::Feasible => format!(
            "Feasible: {} mins of known work fits within {} mins of available time.",
            total_known_effort_minutes, total_available_minutes
        ),
        FeasibilityState::LikelyFeasible => format!(
            "Likely Feasible: {} mins of known work fits within {} mins available, but {} items have unknown effort.",
            total_known_effort_minutes, total_available_minutes, unknown_effort_count
        ),
        FeasibilityState::Uncertain => format!(
            "Uncertain: {} mins known work with {} unestimated items in {} mins available time.",
            total_known_effort_minutes, unknown_effort_count, total_available_minutes
        ),
        FeasibilityState::Infeasible => format!(
            "Infeasible: {} mins known work exceeds {} mins available time or critical conflicts exist.",
            total_known_effort_minutes, total_available_minutes
        ),
    };

    FeasibilityResult {
        state,
        total_known_effort_minutes,
        total_available_minutes,
        unknown_effort_count,
        conflicts: conflicts.to_vec(),
        explanation,
    }
}
