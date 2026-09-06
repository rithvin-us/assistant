-- Durable actions: approvals, tool executions, audit.
--
-- Applied with: scripts/migrate.ps1   (sqlx migrate run). Never on boot.
--
-- What is deliberately NOT here: a generic workflow engine, a job queue, a
-- retry table. Three records with explicit states are enough for the lifecycle
-- this milestone implements, and inventing more would be schema nobody reads.
--
-- Storage boundary (ADR-0016): `tool_executions.arguments` holds the arguments
-- validated against the registry's ToolSpec, because resuming an approved action
-- means running exactly those. `audit_events` stores no arguments at all -- only
-- a summary naming the argument keys. Secrets, tokens and credentials must never
-- reach either: tools receive credentials from the server at call time and must
-- not accept them as arguments.

-- Executions come first; approvals reference them.
create table if not exists tool_executions (
    id               uuid primary key,
    turn_id          uuid        not null,
    conversation_id  uuid        not null,
    principal_id     uuid        not null,
    tool_name        text        not null,
    -- Validated arguments, replayed verbatim on approval.
    arguments        jsonb       not null,
    -- Authoritative risk from the registry at proposal time. Never client-supplied.
    risk             text        not null check (risk in ('green', 'yellow', 'orange', 'red')),
    decision         text        not null check (decision in ('allow', 'require_approval', 'deny')),
    status           text        not null check (status in (
                         'proposed', 'awaiting_approval', 'running',
                         'succeeded', 'failed', 'cancelled')),
    created_at       timestamptz not null default now(),
    started_at       timestamptz,
    completed_at     timestamptz,
    outcome          jsonb
);

create index if not exists tool_executions_principal_created_idx
    on tool_executions (principal_id, created_at desc);
create index if not exists tool_executions_turn_idx
    on tool_executions (turn_id);

create table if not exists approval_requests (
    id            uuid primary key,
    -- One approval authorises one execution. The UNIQUE constraint is the
    -- schema-level statement that an approval can never become a blanket
    -- permission covering several actions.
    execution_id  uuid        not null unique references tool_executions (id) on delete cascade,
    principal_id  uuid        not null,
    tool_name     text        not null,
    risk          text        not null check (risk in ('green', 'yellow', 'orange', 'red')),
    reason        text        not null,
    status        text        not null check (status in (
                      'requested', 'approved', 'rejected', 'expired', 'cancelled')),
    created_at    timestamptz not null default now(),
    expires_at    timestamptz not null,
    resolved_at   timestamptz,
    resolved_by   uuid,

    -- An answered approval must say when and by whom; an unanswered one must not
    -- pretend to. Without this, a partial write could leave a row that looks
    -- resolved but names nobody.
    constraint approval_resolution_is_complete check (
        (status = 'requested' and resolved_at is null and resolved_by is null)
        or (status <> 'requested' and resolved_at is not null)
    )
);

-- Serves the pending-approvals query, which is always scoped by principal.
create index if not exists approval_requests_pending_idx
    on approval_requests (principal_id, created_at desc)
    where status = 'requested';

-- Append-only. Nothing in the application updates or deletes these rows;
-- retention is a deliberate, separate operation (ADR-0017).
create table if not exists audit_events (
    id                uuid primary key,
    at                timestamptz not null default now(),
    principal_id      uuid        not null,
    turn_id           uuid        not null,
    execution_id      uuid        not null,
    tool_name         text        not null,
    risk              text        not null,
    decision          text        not null,
    approval_id       uuid,
    approval_outcome  text,
    execution_status  text        not null,
    -- Names the argument keys, never their values. See ADR-0016.
    summary           text        not null
);

create index if not exists audit_events_principal_at_idx
    on audit_events (principal_id, at desc);
create index if not exists audit_events_turn_idx
    on audit_events (turn_id, at);
create index if not exists audit_events_execution_idx
    on audit_events (execution_id);
