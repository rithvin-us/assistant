-- Milestone 8 -- Document / PDF intelligence.
--
-- Two tables:
--
-- * `documents` -- one row per ingested file. Metadata, processing state and
--   the opaque storage key that tells the object store where the bytes live.
--   Bytes themselves are NOT stored here: that would duplicate the object
--   store and make backups of an ordinary row prohibitively expensive.
--
-- * `document_pages` -- one row per page. Text is stored per page so search,
--   snippets, provenance and bounded retrieval can operate on pages rather
--   than on a flattened blob.
--
-- Both tables scope on `user_id` in every read and write, the same rule as
-- `conversations`, `memories` and every other user-owned table. RLS is
-- enabled with the empty-policy pattern (ADR-0023) so the Supabase anon and
-- authenticated roles see nothing; the server connects as `postgres` and
-- enforces ownership in its SQL.

-- ---------------------------------------------------------------------------
-- documents
-- ---------------------------------------------------------------------------

create table if not exists documents (
    id                 uuid primary key default gen_random_uuid(),
    user_id            uuid not null references users (id) on delete cascade,

    filename           text not null,
    mime_type          text not null,
    size_bytes         bigint not null check (size_bytes >= 0),
    source             text not null
                       check (source in ('local_upload', 'google_drive', 'external_source')),
    source_ref         text,

    -- SHA-256 of the raw bytes, hex-encoded. 64 hex characters.
    content_hash       text not null,

    page_count         integer check (page_count is null or page_count >= 0),
    processing_state   text not null default 'uploaded'
                       check (processing_state in
                              ('uploaded', 'extracting', 'ocr', 'verifying', 'indexed', 'failed')),
    -- Free-form message for a `failed` document. Deliberately not JSON so it
    -- can be rendered directly and does not tempt code to parse it.
    processing_error   text,

    -- Path within the configured object store. Opaque to the database.
    storage_key        text not null,

    created_at         timestamptz not null default now(),
    updated_at         timestamptz not null default now(),
    processed_at       timestamptz
);

-- Same bytes for the same user is the same document; a re-upload dedups. The
-- unique index also gives create-or-return semantics for the ingest path.
create unique index if not exists documents_user_hash_uniq
    on documents (user_id, content_hash);

create index if not exists documents_user_updated_idx
    on documents (user_id, updated_at desc);

create index if not exists documents_user_state_idx
    on documents (user_id, processing_state);

create index if not exists documents_user_mime_idx
    on documents (user_id, mime_type);

-- updated_at trigger, same function as migrations 0007 / 0009.
drop trigger if exists documents_set_updated_at on documents;
create trigger documents_set_updated_at
    before update on documents
    for each row execute function public.set_updated_at();

-- ---------------------------------------------------------------------------
-- document_pages
-- ---------------------------------------------------------------------------
--
-- One row per (document, page). `user_id` is duplicated on the child row so
-- every ownership check is one predicate rather than a join -- the same
-- pattern the `messages` table uses relative to `conversations`. A `content`
-- of `''` means the page was processed but no text was recoverable; a NULL
-- would mean the page has not been processed yet, which the parent's
-- `processing_state` already tells us.

create table if not exists document_pages (
    document_id        uuid not null references documents (id) on delete cascade,
    user_id            uuid not null references users (id) on delete cascade,
    page_number        integer not null check (page_number >= 1),

    extraction_method  text not null
                       check (extraction_method in
                              ('native_text', 'ocr', 'visual_verification', 'none')),
    content            text not null default '',
    confidence         real check (confidence is null or (confidence >= 0.0 and confidence <= 1.0)),
    char_count         integer not null default 0 check (char_count >= 0),
    search_tsv         tsvector,

    primary key (document_id, page_number)
);

-- Ownership-scoped read of one document's pages.
create index if not exists document_pages_user_document_idx
    on document_pages (user_id, document_id, page_number);

-- Full-text search index; the trigger below keeps `search_tsv` in sync.
create index if not exists document_pages_search_tsv_idx
    on document_pages using gin (search_tsv);

create or replace function public.document_pages_set_search_tsv()
returns trigger
language plpgsql
set search_path = ''
as $$
begin
    new.search_tsv := pg_catalog.to_tsvector('pg_catalog.english', coalesce(new.content, ''));
    return new;
end;
$$;

drop trigger if exists document_pages_set_search_tsv on document_pages;
create trigger document_pages_set_search_tsv
    before insert or update of content on document_pages
    for each row execute function public.document_pages_set_search_tsv();

-- ---------------------------------------------------------------------------
-- Row-level security.
-- ---------------------------------------------------------------------------
alter table documents      enable row level security;
alter table document_pages enable row level security;
