-- Milestone 0 schema.
--
-- Only what the skeleton actually needs exists. Domain tables (tasks, memories,
-- documents, approvals, ...) arrive with the milestone that uses them, so this
-- file stays a record of what was really required rather than a wishlist.
--
-- Applied with: scripts/migrate.ps1   (sqlx migrate run)

create extension if not exists "pgcrypto";

-- The local user. Real accounts arrive with authentication; this row lets
-- foreign keys be written correctly from the start.
create table if not exists users (
    id          uuid primary key default gen_random_uuid(),
    email       text unique,
    created_at  timestamptz not null default now()
);

-- Conversations are a log, not memory. Nothing here is ever promoted to a
-- memory implicitly; see crates/assistant-memory.
create table if not exists conversations (
    id          uuid primary key default gen_random_uuid(),
    user_id     uuid not null references users (id) on delete cascade,
    title       text,
    created_at  timestamptz not null default now(),
    updated_at  timestamptz not null default now()
);

create index if not exists conversations_user_updated_idx
    on conversations (user_id, updated_at desc);
