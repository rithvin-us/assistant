-- Milestone 4 Standalone Productivity Schema (Tasks, Reminders, Notes, Ideas)

-- Tasks table
create table if not exists tasks (
    id            uuid primary key default gen_random_uuid(),
    user_id       uuid not null references users (id) on delete cascade,
    title         text not null,
    description   text not null default '',
    priority      text not null default 'P4' check (priority in ('P1', 'P2', 'P3', 'P4')),
    status        text not null default 'todo' check (status in ('todo', 'completed', 'archived')),
    due_at        timestamptz,
    project       text not null default 'Inbox',
    created_at    timestamptz not null default now(),
    updated_at    timestamptz not null default now(),
    completed_at  timestamptz
);

create index if not exists tasks_user_status_due_idx
    on tasks (user_id, status, due_at);

create index if not exists tasks_user_project_idx
    on tasks (user_id, project);

-- Reminders table
create table if not exists reminders (
    id          uuid primary key default gen_random_uuid(),
    user_id     uuid not null references users (id) on delete cascade,
    task_id     uuid references tasks (id) on delete set null,
    title       text not null,
    remind_at   timestamptz not null,
    status      text not null default 'pending' check (status in ('pending', 'handled', 'cancelled')),
    created_at  timestamptz not null default now(),
    updated_at  timestamptz not null default now()
);

create index if not exists reminders_user_status_remind_idx
    on reminders (user_id, status, remind_at);

-- Notes table
create table if not exists notes (
    id           uuid primary key default gen_random_uuid(),
    user_id      uuid not null references users (id) on delete cascade,
    title        text not null,
    content      text not null default '',
    is_archived  boolean not null default false,
    tags         text[] not null default '{}',
    created_at   timestamptz not null default now(),
    updated_at   timestamptz not null default now()
);

create index if not exists notes_user_archived_updated_idx
    on notes (user_id, is_archived, updated_at desc);

-- Ideas table
create table if not exists ideas (
    id                 uuid primary key default gen_random_uuid(),
    user_id            uuid not null references users (id) on delete cascade,
    title              text not null,
    description        text not null default '',
    status             text not null default 'active' check (status in ('active', 'archived', 'converted')),
    converted_task_id  uuid references tasks (id) on delete set null,
    created_at         timestamptz not null default now(),
    updated_at         timestamptz not null default now()
);

create index if not exists ideas_user_status_updated_idx
    on ideas (user_id, status, updated_at desc);
