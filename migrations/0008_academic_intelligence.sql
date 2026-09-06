-- Milestone 6 -- Academic intelligence: Classroom, Drive, and task provenance.
--
-- Three separate problems, one migration.
--
--   1. A task had no idea where it came from. Every row in `tasks` was
--      hand-created by the user, so "who owns this title?" never had to be
--      asked. Importing Classroom coursework makes it a real question: the
--      assignment's title and due date belong to Classroom, the user's notes
--      and priority belong to the user, and a sync must not trample either.
--
--   2. Classroom courses and coursework need to survive being offline and
--      need to be listable without a round trip to Google on every screen
--      open. They are cached here, keyed by the Google identifier.
--
--   3. Sync needs to know when it last ran so it can refuse to hammer the
--      Classroom quota. See ADR-0034.
--
-- Deliberately absent: any table mirroring Drive. Drive file metadata is
-- fetched on demand and cached on the device, never mirrored into Postgres.
-- A user's Drive is not this application's data to keep. See ADR-0033.
--
-- See ADR-0032 (scopes), ADR-0033 (Drive), ADR-0034 (sync and provenance).

-- ---------------------------------------------------------------------------
-- Task provenance
-- ---------------------------------------------------------------------------
--
-- `source` answers "where did this come from" for display. The
-- `external_*` triple answers "is this the same obligation I already
-- imported", which is what stops a second sync creating a second task.
--
-- `source_title` and `source_due_at` are the values Classroom last told us.
-- They exist so a sync can distinguish two cases that otherwise look
-- identical:
--
--   * the task still matches what Classroom sent  -> Classroom's change wins
--   * the user has since edited it                -> the user's edit wins
--
-- Without them the only options are "always overwrite the user" or "never
-- update the deadline", and both are wrong.
alter table tasks
    add column if not exists source text not null default 'manual'
        check (source in ('manual', 'google_classroom', 'gmail', 'calendar', 'drive')),
    add column if not exists external_provider text,
    add column if not exists external_id text,
    -- The account the item was imported from. `on delete set null` and not
    -- `cascade`: disconnecting or deleting a Google account must never take a
    -- user's task with it. See ADR-0034.
    add column if not exists external_account_id uuid references connected_accounts (id) on delete set null,
    add column if not exists source_title text,
    add column if not exists source_due_at timestamptz,
    add column if not exists source_synced_at timestamptz;

-- The identity that makes sync idempotent. Partial, because manually created
-- tasks have no external id and must not collide with each other.
create unique index if not exists tasks_external_identity_idx
    on tasks (user_id, external_provider, external_id)
    where external_id is not null;

create index if not exists tasks_user_source_idx
    on tasks (user_id, source);

-- ---------------------------------------------------------------------------
-- Classroom courses
-- ---------------------------------------------------------------------------
--
-- `external_id` is the Classroom course id, which is stable for the life of
-- the course. Scoped by (user_id, account_id) because the same person may be
-- enrolled under two Google accounts and those are different enrolments, not
-- duplicates to be merged.
create table if not exists classroom_courses (
    id                 uuid primary key default gen_random_uuid(),
    user_id            uuid not null references users (id) on delete cascade,
    account_id         uuid not null references connected_accounts (id) on delete cascade,
    external_id        text not null,
    name               text not null,
    section            text,
    description        text,
    room               text,
    teacher_name       text,
    course_state       text not null default 'ACTIVE',
    alternate_link     text,
    -- Classroom's own updateTime, not ours. Null when Google omits it.
    source_updated_at  timestamptz,
    synced_at          timestamptz not null default now(),
    created_at         timestamptz not null default now(),
    updated_at         timestamptz not null default now(),
    unique (user_id, account_id, external_id)
);

create index if not exists classroom_courses_user_account_idx
    on classroom_courses (user_id, account_id, course_state);

-- ---------------------------------------------------------------------------
-- Classroom coursework
-- ---------------------------------------------------------------------------
--
-- `due_at` is null when the assignment genuinely has no deadline. Classroom
-- sends `dueDate` and `dueTime` as separate optional objects that must appear
-- together; when either is missing there is no deadline, and inventing one
-- would be a lie the scheduler would then act on. See ADR-0034.
create table if not exists classroom_coursework (
    id                  uuid primary key default gen_random_uuid(),
    user_id             uuid not null references users (id) on delete cascade,
    account_id          uuid not null references connected_accounts (id) on delete cascade,
    course_external_id  text not null,
    external_id         text not null,
    title               text not null,
    description         text,
    state               text not null default 'PUBLISHED',
    alternate_link      text,
    -- Null means "no due date", never "unknown". Stored in UTC: Classroom's
    -- dueTime is documented as UTC.
    due_at              timestamptz,
    max_points          double precision,
    work_type           text,
    -- Metadata only: title and link of attached materials. Never file bodies.
    materials           jsonb not null default '[]'::jsonb,
    source_updated_at   timestamptz,
    synced_at           timestamptz not null default now(),
    created_at          timestamptz not null default now(),
    updated_at          timestamptz not null default now(),
    unique (user_id, account_id, external_id)
);

create index if not exists classroom_coursework_user_due_idx
    on classroom_coursework (user_id, due_at);

create index if not exists classroom_coursework_course_idx
    on classroom_coursework (user_id, account_id, course_external_id);

-- ---------------------------------------------------------------------------
-- Classroom announcements
-- ---------------------------------------------------------------------------
--
-- Cached, not archived. `synced_at` exists so a prune can drop anything older
-- than the retention window without needing to ask Google what it still has.
-- Keeping a permanent copy of every announcement a student has ever received
-- is not something this application needs in order to show the recent ones.
create table if not exists classroom_announcements (
    id                  uuid primary key default gen_random_uuid(),
    user_id             uuid not null references users (id) on delete cascade,
    account_id          uuid not null references connected_accounts (id) on delete cascade,
    course_external_id  text not null,
    external_id         text not null,
    text_content        text not null default '',
    author_name         text,
    alternate_link      text,
    materials           jsonb not null default '[]'::jsonb,
    source_created_at   timestamptz,
    source_updated_at   timestamptz,
    synced_at           timestamptz not null default now(),
    created_at          timestamptz not null default now(),
    updated_at          timestamptz not null default now(),
    unique (user_id, account_id, external_id)
);

create index if not exists classroom_announcements_recent_idx
    on classroom_announcements (user_id, account_id, source_created_at desc);

-- ---------------------------------------------------------------------------
-- Sync state
-- ---------------------------------------------------------------------------
--
-- One row per (account, resource). `last_synced_at` is what a conservative
-- refresh consults before deciding it may call Google again; `last_error` is
-- what the UI shows instead of pretending the cache is live.
create table if not exists academic_sync_state (
    id              uuid primary key default gen_random_uuid(),
    user_id         uuid not null references users (id) on delete cascade,
    account_id      uuid not null references connected_accounts (id) on delete cascade,
    resource        text not null check (resource in ('courses', 'coursework', 'announcements')),
    last_synced_at  timestamptz,
    last_error      text,
    created_at      timestamptz not null default now(),
    updated_at      timestamptz not null default now(),
    unique (user_id, account_id, resource)
);

-- ---------------------------------------------------------------------------
-- updated_at triggers (the function is defined in migration 0007)
-- ---------------------------------------------------------------------------
drop trigger if exists classroom_courses_set_updated_at on classroom_courses;
create trigger classroom_courses_set_updated_at
    before update on classroom_courses
    for each row execute function public.set_updated_at();

drop trigger if exists classroom_coursework_set_updated_at on classroom_coursework;
create trigger classroom_coursework_set_updated_at
    before update on classroom_coursework
    for each row execute function public.set_updated_at();

drop trigger if exists classroom_announcements_set_updated_at on classroom_announcements;
create trigger classroom_announcements_set_updated_at
    before update on classroom_announcements
    for each row execute function public.set_updated_at();

drop trigger if exists academic_sync_state_set_updated_at on academic_sync_state;
create trigger academic_sync_state_set_updated_at
    before update on academic_sync_state
    for each row execute function public.set_updated_at();

-- ---------------------------------------------------------------------------
-- Row Level Security (ADR-0023)
-- ---------------------------------------------------------------------------
--
-- Same closed policy set as migrations 0005 and 0007: RLS on, no policies, the
-- server connects as the owning role. A new table in `public` is published by
-- PostgREST the moment it exists, so this is not optional.
alter table classroom_courses       enable row level security;
alter table classroom_coursework    enable row level security;
alter table classroom_announcements enable row level security;
alter table academic_sync_state     enable row level security;

revoke all on all tables    in schema public from anon, authenticated;
revoke all on all sequences in schema public from anon, authenticated;
revoke all on all functions in schema public from anon, authenticated;
