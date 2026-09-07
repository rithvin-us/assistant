-- Milestone 9 -- Unified Personal Planning preferences and snapshots.
--
-- Provides durable preference storage for user daily working hours and saved
-- planning snapshots. RLS is enabled and ownership is enforced on user_id.

CREATE TABLE IF NOT EXISTS planning_preferences (
    user_id UUID PRIMARY KEY,
    work_start_time TIME NOT NULL DEFAULT '08:00:00',
    work_end_time TIME NOT NULL DEFAULT '22:00:00',
    max_daily_minutes INT NOT NULL DEFAULT 480,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

ALTER TABLE planning_preferences ENABLE ROW LEVEL SECURITY;

CREATE POLICY planning_preferences_user_isolation ON planning_preferences
    FOR ALL USING (auth.uid() = user_id);
