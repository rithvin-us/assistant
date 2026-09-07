-- Milestone 7 -- Long-term memory foundation.
--
-- Memory is not conversation history. A conversation is a log; a memory is a
-- durable, attributed claim that survived a promotion step. The model may
-- propose one, but the model is never the writer: rows here are always inserted
-- and updated by application code, so ranking, ownership and lifecycle are
-- decisions Rust makes, not decisions a language model can talk itself into.
--
-- Ownership is enforced the same way `conversations` enforces it (ADR-0021):
-- every read and every write scopes by `user_id` in SQL, and a row belonging
-- to somebody else must be indistinguishable from a row that does not exist.
--
-- Text search stays inside Postgres. A `tsvector` column is maintained by
-- trigger and a GIN index covers it; M7 does not introduce embeddings,
-- pgvector, Elasticsearch or any other retrieval infrastructure.

-- ---------------------------------------------------------------------------
-- memories
-- ---------------------------------------------------------------------------
--
-- `kind` is the memory type. `Preference` is how the user wants things done;
-- `Fact` is a stable claim; `Idea` is a floated thought; `Project` scopes to a
-- project and is archived with it; `Commitment` is something the user said
-- they would do; `Temporary` is scoped to an expiry and is not promoted to a
-- permanent fact without an explicit action.
--
-- `lifecycle` is one of `active`, `archived`, `superseded`. Archived rows stay
-- queryable so the UI can restore them; superseded rows point at the memory
-- that replaced them so the UI can explain what changed. Destructive deletion
-- is deliberately not the default UI action.
--
-- `importance` is a 1..=5 integer scale, mirroring the deterministic priority
-- vocabulary the productivity milestone uses (`P1`..`P4`). It is set by the
-- application, not the model.
--
-- `confidence` is a probability, kept separate from importance because they
-- mean different things: importance says how much the user cares, confidence
-- says how sure the system is the content is true.
--
-- `source_kind` records provenance. It is a bounded enum -- adding a new
-- source is a migration, not a free-form string a caller can invent -- so a
-- retrieval that filters by provenance cannot accidentally miss a source that
-- named itself differently one week.
--
-- `expires_at` is only populated for `temporary` memories; the application
-- expires them by moving them to `archived` on the retrieval path.
create table if not exists memories (
    id                 uuid primary key default gen_random_uuid(),
    user_id            uuid not null references users (id) on delete cascade,

    kind               text not null
                       check (kind in ('preference', 'fact', 'idea', 'commitment',
                                       'project', 'temporary')),
    lifecycle          text not null default 'active'
                       check (lifecycle in ('active', 'archived', 'superseded')),

    content            text not null,
    importance         smallint not null default 3
                       check (importance between 1 and 5),
    confidence         real not null default 0.7
                       check (confidence >= 0.0 and confidence <= 1.0),

    source_kind        text not null
                       check (source_kind in ('explicit_user_input', 'conversation',
                                              'task', 'note', 'idea', 'project',
                                              'document', 'external_source')),
    -- Identifier within that source (e.g. a task id, a note id, a message id).
    -- Free text on purpose: not every source is a row in this database.
    source_ref         text,

    -- When this memory took over from an earlier one. Populated together with
    -- `lifecycle = 'superseded'` on the old row.
    superseded_by      uuid references memories (id) on delete set null,

    -- Only meaningful for `kind = 'temporary'`. Enforced by a partial check.
    expires_at         timestamptz,

    created_at         timestamptz not null default now(),
    updated_at         timestamptz not null default now(),
    -- `last_accessed_at` moves only when the application declares the memory
    -- "used" -- returned from a retrieval that fed a turn. Incidental
    -- database reads do not count.
    last_accessed_at   timestamptz,
    access_count       integer not null default 0
                       check (access_count >= 0),
    archived_at        timestamptz,

    -- Maintained by trigger below. Text search stays inside Postgres.
    search_tsv         tsvector
);

-- Temporary is the only kind that may carry an expiry, and it must carry one.
alter table memories drop constraint if exists memories_temporary_expiry;
alter table memories add constraint memories_temporary_expiry
    check (
        (kind = 'temporary' and expires_at is not null)
        or (kind <> 'temporary' and expires_at is null)
    );

-- A row is either active with no `archived_at`, or archived/superseded with
-- one. This keeps the two out of sync from being representable.
alter table memories drop constraint if exists memories_archived_at_matches_lifecycle;
alter table memories add constraint memories_archived_at_matches_lifecycle
    check (
        (lifecycle = 'active' and archived_at is null)
        or (lifecycle <> 'active' and archived_at is not null)
    );

-- ---------------------------------------------------------------------------
-- Indexes
-- ---------------------------------------------------------------------------
--
-- Every predicate the API supports is either a bind parameter on an indexed
-- column or covered by the GIN index on `search_tsv`. Nothing user-supplied
-- is ever concatenated into SQL.

create index if not exists memories_user_lifecycle_updated_idx
    on memories (user_id, lifecycle, updated_at desc);

create index if not exists memories_user_kind_idx
    on memories (user_id, kind);

create index if not exists memories_user_importance_idx
    on memories (user_id, importance desc, updated_at desc);

create index if not exists memories_user_last_accessed_idx
    on memories (user_id, last_accessed_at desc nulls last);

-- Used by the expiry sweep. Partial: only temporary memories carry an expiry.
create index if not exists memories_expires_at_idx
    on memories (expires_at)
    where kind = 'temporary' and lifecycle = 'active';

-- Text search. GIN because it is what tsvector is for; the recheck is fast.
create index if not exists memories_search_tsv_idx
    on memories using gin (search_tsv);

-- ---------------------------------------------------------------------------
-- Search vector trigger
-- ---------------------------------------------------------------------------
--
-- Same shape and same hardening as `public.set_updated_at` in migration 0007
-- -- `search_path = ''` and every reference schema-qualified.
create or replace function public.memories_set_search_tsv()
returns trigger
language plpgsql
set search_path = ''
as $$
begin
    new.search_tsv := pg_catalog.to_tsvector('pg_catalog.english', coalesce(new.content, ''));
    return new;
end;
$$;

drop trigger if exists memories_set_search_tsv on memories;
create trigger memories_set_search_tsv
    before insert or update of content on memories
    for each row execute function public.memories_set_search_tsv();

-- updated_at trigger (function defined in migration 0007)
drop trigger if exists memories_set_updated_at on memories;
create trigger memories_set_updated_at
    before update on memories
    for each row execute function public.set_updated_at();

-- ---------------------------------------------------------------------------
-- Row-level security
-- ---------------------------------------------------------------------------
--
-- Same rule as every other table in `public`: RLS on, empty policy set, so
-- `anon` and `authenticated` (Supabase's PostgREST roles) see nothing. The
-- server connects as `postgres` and enforces `user_id` in its own SQL.
alter table memories enable row level security;
