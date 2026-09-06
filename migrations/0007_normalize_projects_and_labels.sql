-- Milestone 6 -- Normalizing the productivity layer.
--
-- Two columns carried repeated, unmanaged strings:
--
--   * `tasks.project text not null default 'Inbox'` -- the project name was
--     stored once per task. Renaming a project meant an UPDATE across every
--     task that mentioned it, a project could not carry a colour or an
--     ordering, and 'Work', 'work' and 'Work ' were three different projects.
--
--   * `notes.tags text[]` -- the same problem plus an array. A tag could not be
--     renamed, could not be shared with a task, and `tags @> '{x}'` cannot use
--     the same index as an equality lookup on a join table.
--
-- Both become first-class rows owned by a user, referenced by id. Names are
-- unique per user, case-insensitively, which is the rule a person actually
-- expects from a label picker.
--
-- Labels are shared between tasks and notes on purpose: one @home means one
-- @home everywhere, which is the point of naming it.
--
-- See ADR-0028.

-- ---------------------------------------------------------------------------
-- updated_at, once, in the database
-- ---------------------------------------------------------------------------
--
-- Every UPDATE in `routes/productivity.rs` hand-writes `updated_at = now()`.
-- That is one place per statement to forget, and a row written by anything
-- other than that file -- a migration, psql, a future job -- silently keeps a
-- stale timestamp. The database is the only writer that cannot be bypassed.
--
-- `search_path = ''` is set explicitly and every reference is schema-qualified:
-- a `security definer`-adjacent function with a mutable search_path is a
-- privilege-escalation shape Supabase's own advisor flags.
create or replace function public.set_updated_at()
returns trigger
language plpgsql
set search_path = ''
as $$
begin
    new.updated_at = pg_catalog.now();
    return new;
end;
$$;

-- ---------------------------------------------------------------------------
-- projects
-- ---------------------------------------------------------------------------
create table if not exists projects (
    id          uuid primary key default gen_random_uuid(),
    user_id     uuid not null references users (id) on delete cascade,
    name        text not null check (length(btrim(name)) > 0),
    color       text not null default '#808080',
    -- Exactly one project per user is the fallback every task lands in when it
    -- names no project, and it is the one project that must not be deleted.
    -- Marking it in the row beats matching on the literal name 'Inbox', which a
    -- user is free to rename.
    is_inbox    boolean not null default false,
    position    integer not null default 0,
    created_at  timestamptz not null default now(),
    updated_at  timestamptz not null default now()
);

-- Case-insensitive uniqueness per user. `lower(name)` rather than `citext`
-- because the extension buys nothing else here.
create unique index if not exists projects_user_name_uniq
    on projects (user_id, lower(name));

create unique index if not exists projects_user_inbox_uniq
    on projects (user_id) where is_inbox;

create index if not exists projects_user_position_idx
    on projects (user_id, position, created_at);

-- ---------------------------------------------------------------------------
-- labels (shared by tasks and notes)
-- ---------------------------------------------------------------------------
create table if not exists labels (
    id          uuid primary key default gen_random_uuid(),
    user_id     uuid not null references users (id) on delete cascade,
    name        text not null check (length(btrim(name)) > 0),
    color       text not null default '#808080',
    created_at  timestamptz not null default now(),
    updated_at  timestamptz not null default now()
);

create unique index if not exists labels_user_name_uniq
    on labels (user_id, lower(name));

-- ---------------------------------------------------------------------------
-- join tables
-- ---------------------------------------------------------------------------
--
-- The composite primary key is the deduplication: attaching the same label
-- twice is a no-op rather than a second row. The reverse index exists because
-- "every task with label X" is the query a label page runs.
create table if not exists task_labels (
    task_id   uuid not null references tasks (id)  on delete cascade,
    label_id  uuid not null references labels (id) on delete cascade,
    primary key (task_id, label_id)
);

create index if not exists task_labels_label_idx on task_labels (label_id);

create table if not exists note_labels (
    note_id   uuid not null references notes (id)  on delete cascade,
    label_id  uuid not null references labels (id) on delete cascade,
    primary key (note_id, label_id)
);

create index if not exists note_labels_label_idx on note_labels (label_id);

-- ---------------------------------------------------------------------------
-- Backfill: tasks.project (text) -> projects row
-- ---------------------------------------------------------------------------

-- Every user who owns a task gets an Inbox first, so the is_inbox flag lands on
-- that row rather than on a same-named project created by the next statement.
insert into projects (user_id, name, is_inbox, position)
select distinct t.user_id, 'Inbox', true, 0
from tasks t
on conflict (user_id, lower(name)) do nothing;

-- `distinct on` collapses 'Work' and 'work' before the insert rather than
-- relying on `on conflict` to absorb duplicates raised inside one statement.
insert into projects (user_id, name, is_inbox)
select distinct on (t.user_id, lower(btrim(t.project)))
       t.user_id, btrim(t.project), false
from tasks t
where btrim(t.project) <> ''
order by t.user_id, lower(btrim(t.project)), t.created_at
on conflict (user_id, lower(name)) do nothing;

-- `on delete restrict`: deleting a project that still holds tasks is a
-- decision, not a side effect. The route reassigns to Inbox and then deletes.
alter table tasks add column if not exists project_id uuid references projects (id) on delete restrict;

update tasks t
set project_id = p.id
from projects p
where p.user_id = t.user_id
  and lower(p.name) = lower(btrim(t.project))
  and t.project_id is null;

-- A task whose project was blank, or whose name lost a race above, lands in Inbox.
update tasks t
set project_id = p.id
from projects p
where t.project_id is null
  and p.user_id = t.user_id
  and p.is_inbox;

alter table tasks alter column project_id set not null;

drop index if exists tasks_user_project_idx;
alter table tasks drop column if exists project;

create index if not exists tasks_user_project_id_idx
    on tasks (user_id, project_id, status);

-- ---------------------------------------------------------------------------
-- Backfill: notes.tags (text[]) -> labels + note_labels
-- ---------------------------------------------------------------------------
insert into labels (user_id, name)
select distinct on (n.user_id, lower(btrim(tag)))
       n.user_id, btrim(tag)
from notes n
cross join lateral unnest(n.tags) as tag
where btrim(tag) <> ''
order by n.user_id, lower(btrim(tag)), n.created_at
on conflict (user_id, lower(name)) do nothing;

insert into note_labels (note_id, label_id)
select distinct n.id, l.id
from notes n
cross join lateral unnest(n.tags) as tag
join labels l
  on l.user_id = n.user_id
 and lower(l.name) = lower(btrim(tag))
where btrim(tag) <> ''
on conflict do nothing;

alter table notes drop column if exists tags;

-- ---------------------------------------------------------------------------
-- updated_at triggers
-- ---------------------------------------------------------------------------
drop trigger if exists projects_set_updated_at on projects;
create trigger projects_set_updated_at
    before update on projects
    for each row execute function public.set_updated_at();

drop trigger if exists labels_set_updated_at on labels;
create trigger labels_set_updated_at
    before update on labels
    for each row execute function public.set_updated_at();

drop trigger if exists tasks_set_updated_at on tasks;
create trigger tasks_set_updated_at
    before update on tasks
    for each row execute function public.set_updated_at();

drop trigger if exists reminders_set_updated_at on reminders;
create trigger reminders_set_updated_at
    before update on reminders
    for each row execute function public.set_updated_at();

drop trigger if exists notes_set_updated_at on notes;
create trigger notes_set_updated_at
    before update on notes
    for each row execute function public.set_updated_at();

drop trigger if exists ideas_set_updated_at on ideas;
create trigger ideas_set_updated_at
    before update on ideas
    for each row execute function public.set_updated_at();

drop trigger if exists connected_accounts_set_updated_at on connected_accounts;
create trigger connected_accounts_set_updated_at
    before update on connected_accounts
    for each row execute function public.set_updated_at();

-- ---------------------------------------------------------------------------
-- Row Level Security (ADR-0023)
-- ---------------------------------------------------------------------------
--
-- Same closed policy set as migration 0005: RLS on, no policies, server
-- connects as the owning role. A new table in `public` is published by
-- PostgREST the moment it exists, so this is not optional.
alter table projects    enable row level security;
alter table labels      enable row level security;
alter table task_labels enable row level security;
alter table note_labels enable row level security;

revoke all on all tables    in schema public from anon, authenticated;
revoke all on all sequences in schema public from anon, authenticated;
revoke all on all functions in schema public from anon, authenticated;
