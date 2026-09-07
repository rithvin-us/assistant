//! Deterministic availability window calculator.

use crate::model::*;
use time::{OffsetDateTime, Time};

/// Planning preferences for daily availability boundaries.
#[derive(Debug, Clone)]
pub struct WorkingHours {
    /// Start of daily active window (default 08:00).
    pub start_time: Time,
    /// End of daily active window (default 22:00).
    pub end_time: Time,
    /// Maximum daily focus work minutes (default 480 mins = 8 hours).
    pub max_daily_minutes: u32,
}

impl Default for WorkingHours {
    fn default() -> Self {
        Self {
            start_time: Time::from_hms(8, 0, 0).unwrap_or(Time::MIDNIGHT),
            end_time: Time::from_hms(22, 0, 0).unwrap_or(Time::MIDNIGHT),
            max_daily_minutes: 480,
        }
    }
}

/// Computes usable availability windows within `[start, end]`, subtracting fixed commitments.
pub fn calculate_availability_windows(
    start: OffsetDateTime,
    end: OffsetDateTime,
    commitments: &[Commitment],
    working_hours: &WorkingHours,
) -> Vec<AvailabilityWindow> {
    let mut windows = Vec::new();
    let mut current_day = start.date();
    let end_day = end.date();

    while current_day <= end_day {
        let day_start_dt = current_day.with_time(working_hours.start_time).assume_utc();
        let day_end_dt = current_day.with_time(working_hours.end_time).assume_utc();

        let window_start = day_start_dt.max(start);
        let window_end = day_end_dt.min(end);

        if window_start < window_end {
            // Find commitments overlapping with this day's usable window
            let mut day_commitments: Vec<(OffsetDateTime, OffsetDateTime)> = commitments
                .iter()
                .filter(|c| !c.is_all_day && c.end_time > window_start && c.start_time < window_end)
                .map(|c| (c.start_time.max(window_start), c.end_time.min(window_end)))
                .collect();

            day_commitments.sort_by_key(|(s, _)| *s);

            let mut cursor = window_start;
            for (c_start, c_end) in day_commitments {
                if c_start > cursor {
                    let duration = (c_start - cursor).whole_minutes() as u32;
                    if duration >= 15 {
                        windows.push(AvailabilityWindow {
                            start_time: cursor,
                            end_time: c_start,
                            duration_minutes: duration,
                            is_usable: true,
                            source: "working_hours".to_string(),
                        });
                    }
                }
                cursor = cursor.max(c_end);
            }

            if window_end > cursor {
                let duration = (window_end - cursor).whole_minutes() as u32;
                if duration >= 15 {
                    windows.push(AvailabilityWindow {
                        start_time: cursor,
                        end_time: window_end,
                        duration_minutes: duration,
                        is_usable: true,
                        source: "working_hours".to_string(),
                    });
                }
            }
        }

        current_day = match current_day.next_day() {
            Some(d) => d,
            None => break,
        };
    }

    windows
}

/// Sums total available minutes across a slice of windows.
pub fn total_available_minutes(windows: &[AvailabilityWindow]) -> u32 {
    windows.iter().map(|w| w.duration_minutes).sum()
}
