//! Deterministic Free-Time Calculation Engine. See ADR-0027.
//!
//! Computes available schedule slots using interval arithmetic over calendar events.
//! 100% deterministic, zero LLM dependency.

use assistant_protocol::FreeSlot;
use assistant_tools::CalendarEvent;
use time::{Duration, OffsetDateTime};

/// Interval with start and end timestamps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Interval {
    pub start: OffsetDateTime,
    pub end: OffsetDateTime,
}

impl Interval {
    pub fn new(start: OffsetDateTime, end: OffsetDateTime) -> Option<Self> {
        if start < end {
            Some(Self { start, end })
        } else {
            None
        }
    }

    pub fn duration_minutes(&self) -> u32 {
        let seconds = (self.end - self.start).whole_seconds();
        if seconds <= 0 {
            0
        } else {
            (seconds / 60) as u32
        }
    }
}

/// Merges overlapping and adjacent intervals into sorted, non-overlapping intervals.
pub fn merge_intervals(mut intervals: Vec<Interval>) -> Vec<Interval> {
    if intervals.is_empty() {
        return Vec::new();
    }

    intervals.sort_by_key(|i| i.start);

    let mut merged: Vec<Interval> = Vec::with_capacity(intervals.len());
    let mut current = intervals[0];

    for next in intervals.into_iter().skip(1) {
        if next.start <= current.end {
            // Overlapping or touching: extend current end
            if next.end > current.end {
                current.end = next.end;
            }
        } else {
            merged.push(current);
            current = next;
        }
    }
    merged.push(current);

    merged
}

/// Computes free time slots within `[window_start, window_end]` given busy calendar events.
pub fn calculate_free_slots(
    events: &[CalendarEvent],
    window_start: OffsetDateTime,
    window_end: OffsetDateTime,
    duration_minutes: u32,
    buffer_minutes: u32,
) -> Vec<FreeSlot> {
    if window_start >= window_end || duration_minutes == 0 {
        return Vec::new();
    }

    let buffer = Duration::minutes(buffer_minutes as i64);

    // 1. Convert calendar events to busy intervals within the window
    let mut busy_intervals = Vec::new();
    for event in events {
        let mut start = event.start_time - buffer;
        let mut end = event.end_time + buffer;

        if start < window_start {
            start = window_start;
        }
        if end > window_end {
            end = window_end;
        }

        if let Some(interval) = Interval::new(start, end) {
            busy_intervals.push(interval);
        }
    }

    // 2. Merge overlapping busy blocks
    let merged_busy = merge_intervals(busy_intervals);

    // 3. Compute the complement (free blocks)
    let mut free_slots = Vec::new();
    let mut cursor = window_start;

    for busy in merged_busy {
        if busy.start > cursor {
            let slot_duration = (busy.start - cursor).whole_seconds() / 60;
            if slot_duration >= duration_minutes as i64 {
                free_slots.push(FreeSlot {
                    start_time: cursor,
                    end_time: busy.start,
                    duration_minutes: slot_duration as u32,
                });
            }
        }
        if busy.end > cursor {
            cursor = busy.end;
        }
    }

    // 4. Trailing free space up to window_end
    if cursor < window_end {
        let trailing_duration = (window_end - cursor).whole_seconds() / 60;
        if trailing_duration >= duration_minutes as i64 {
            free_slots.push(FreeSlot {
                start_time: cursor,
                end_time: window_end,
                duration_minutes: trailing_duration as u32,
            });
        }
    }

    free_slots
}

/// Alias with optional buffer.
pub fn find_free_slots(
    events: &[CalendarEvent],
    window_start: OffsetDateTime,
    window_end: OffsetDateTime,
    duration_minutes: u32,
    buffer_minutes: Option<u32>,
) -> Vec<FreeSlot> {
    calculate_free_slots(
        events,
        window_start,
        window_end,
        duration_minutes,
        buffer_minutes.unwrap_or(0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;
    use uuid::Uuid;

    fn make_event(start: OffsetDateTime, end: OffsetDateTime) -> CalendarEvent {
        CalendarEvent {
            id: "test-event".into(),
            account_id: Uuid::nil(),
            title: "Meeting".into(),
            start_time: start,
            end_time: end,
            description: None,
            location: None,
            all_day: false,
        }
    }

    #[test]
    fn single_event_produces_two_free_slots() {
        let start = datetime!(2026-09-07 09:00:00 UTC);
        let end = datetime!(2026-09-07 17:00:00 UTC);

        let event = make_event(
            datetime!(2026-09-07 12:00:00 UTC),
            datetime!(2026-09-07 13:00:00 UTC),
        );

        let slots = calculate_free_slots(&[event], start, end, 60, 0);
        assert_eq!(slots.len(), 2);
        assert_eq!(slots[0].start_time, start);
        assert_eq!(slots[0].end_time, datetime!(2026-09-07 12:00:00 UTC));
        assert_eq!(slots[0].duration_minutes, 180);

        assert_eq!(slots[1].start_time, datetime!(2026-09-07 13:00:00 UTC));
        assert_eq!(slots[1].end_time, end);
        assert_eq!(slots[1].duration_minutes, 240);
    }

    #[test]
    fn overlapping_events_are_merged() {
        let start = datetime!(2026-09-07 09:00:00 UTC);
        let end = datetime!(2026-09-07 17:00:00 UTC);

        let e1 = make_event(
            datetime!(2026-09-07 10:00:00 UTC),
            datetime!(2026-09-07 12:00:00 UTC),
        );
        let e2 = make_event(
            datetime!(2026-09-07 11:00:00 UTC),
            datetime!(2026-09-07 13:00:00 UTC),
        );

        let slots = calculate_free_slots(&[e1, e2], start, end, 60, 0);
        assert_eq!(slots.len(), 2);
        assert_eq!(slots[0].start_time, start);
        assert_eq!(slots[0].end_time, datetime!(2026-09-07 10:00:00 UTC));
        assert_eq!(slots[0].duration_minutes, 60);

        assert_eq!(slots[1].start_time, datetime!(2026-09-07 13:00:00 UTC));
        assert_eq!(slots[1].end_time, end);
        assert_eq!(slots[1].duration_minutes, 240);
    }

    #[test]
    fn slot_shorter_than_requested_duration_is_excluded() {
        let start = datetime!(2026-09-07 09:00:00 UTC);
        let end = datetime!(2026-09-07 17:00:00 UTC);

        let e1 = make_event(
            datetime!(2026-09-07 09:30:00 UTC),
            datetime!(2026-09-07 17:00:00 UTC),
        );

        // Gap is only 30 minutes, requested 60 minutes
        let slots = calculate_free_slots(&[e1], start, end, 60, 0);
        assert_eq!(slots.len(), 0);
    }

    #[test]
    fn step_17_controlled_specification_test() {
        let start = datetime!(2026-09-07 09:00:00 UTC);
        let end = datetime!(2026-09-07 18:00:00 UTC);

        let e1 = make_event(
            datetime!(2026-09-07 10:00:00 UTC),
            datetime!(2026-09-07 11:00:00 UTC),
        );
        let e2 = make_event(
            datetime!(2026-09-07 13:00:00 UTC),
            datetime!(2026-09-07 15:00:00 UTC),
        );
        let e3 = make_event(
            datetime!(2026-09-07 14:30:00 UTC),
            datetime!(2026-09-07 16:00:00 UTC),
        );

        // Requested duration: 90 minutes
        let slots = calculate_free_slots(&[e1, e2, e3], start, end, 90, 0);

        // Expected:
        // 09:00-10:00 = 60 mins (<90 -> excluded)
        // 11:00-13:00 = 120 mins (>=90 -> included)
        // 16:00-18:00 = 120 mins (>=90 -> included)
        assert_eq!(slots.len(), 2);
        assert_eq!(slots[0].start_time, datetime!(2026-09-07 11:00:00 UTC));
        assert_eq!(slots[0].end_time, datetime!(2026-09-07 13:00:00 UTC));
        assert_eq!(slots[0].duration_minutes, 120);

        assert_eq!(slots[1].start_time, datetime!(2026-09-07 16:00:00 UTC));
        assert_eq!(slots[1].end_time, datetime!(2026-09-07 18:00:00 UTC));
        assert_eq!(slots[1].duration_minutes, 120);
    }

    #[test]
    fn adjacent_events_merge_correctly() {
        let start = datetime!(2026-09-07 09:00:00 UTC);
        let end = datetime!(2026-09-07 18:00:00 UTC);

        let e1 = make_event(
            datetime!(2026-09-07 10:00:00 UTC),
            datetime!(2026-09-07 11:00:00 UTC),
        );
        let e2 = make_event(
            datetime!(2026-09-07 11:00:00 UTC),
            datetime!(2026-09-07 12:00:00 UTC),
        );

        let slots = calculate_free_slots(&[e1, e2], start, end, 60, 0);
        assert_eq!(slots.len(), 2);
        assert_eq!(slots[0].end_time, datetime!(2026-09-07 10:00:00 UTC));
        assert_eq!(slots[1].start_time, datetime!(2026-09-07 12:00:00 UTC));
    }

    #[test]
    fn events_outside_range_and_spanning_boundaries() {
        let start = datetime!(2026-09-07 09:00:00 UTC);
        let end = datetime!(2026-09-07 18:00:00 UTC);

        let e_outside_before = make_event(
            datetime!(2026-09-07 07:00:00 UTC),
            datetime!(2026-09-07 08:00:00 UTC),
        );
        let e_span_start = make_event(
            datetime!(2026-09-07 08:30:00 UTC),
            datetime!(2026-09-07 09:30:00 UTC),
        );
        let e_span_end = make_event(
            datetime!(2026-09-07 17:30:00 UTC),
            datetime!(2026-09-07 18:30:00 UTC),
        );
        let e_outside_after = make_event(
            datetime!(2026-09-07 19:00:00 UTC),
            datetime!(2026-09-07 20:00:00 UTC),
        );

        let slots = calculate_free_slots(
            &[e_outside_before, e_span_start, e_span_end, e_outside_after],
            start,
            end,
            60,
            0,
        );

        // Gap from 09:30 to 17:30 = 8 hours (480 mins)
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].start_time, datetime!(2026-09-07 09:30:00 UTC));
        assert_eq!(slots[0].end_time, datetime!(2026-09-07 17:30:00 UTC));
        assert_eq!(slots[0].duration_minutes, 480);
    }

    #[test]
    fn no_busy_events_and_exact_duration_slot() {
        let start = datetime!(2026-09-07 09:00:00 UTC);
        let end = datetime!(2026-09-07 10:30:00 UTC);

        // No busy events, search 90 mins, duration 90 mins -> 1 exact slot
        let slots = calculate_free_slots(&[][..], start, end, 90, 0);
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].start_time, start);
        assert_eq!(slots[0].end_time, end);
        assert_eq!(slots[0].duration_minutes, 90);
    }
}
