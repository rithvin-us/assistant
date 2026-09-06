-- Row Level Security on every table in `public`.
--
-- Why this exists. Supabase publishes every table in the `public` schema
-- through PostgREST and GraphQL, reachable with the project's `anon` key --
-- a key that is designed to be handed to clients. Supabase's default
-- privileges additionally grant `anon` and `authenticated` full DML on every
-- table created in `public` (`arwdDxtm`: insert, select, update, delete,
-- truncate, references, trigger, maintain), and neither role holds BYPASSRLS.
--
-- With RLS disabled, the only thing standing between the public internet and
-- the conversation log, the audit trail and the approval records is the anon
-- key staying secret. A key meant to be published is not an access control.
--
-- This is deliberately NOT a set of policies. The project does not use
-- Supabase Auth or PostgREST: `assistant-server` connects as `postgres`, which
-- owns these tables and holds BYPASSRLS. Enabling RLS with an empty policy set
-- therefore denies `anon` and `authenticated` everything while leaving the
-- server's own access completely untouched. Ownership is expressed in SQL by
-- `user_id` predicates the server writes itself -- see ADR-0015 and ADR-0021 --
-- and duplicating that as a policy would put the same rule in two places.
--
-- `force row level security` is deliberately NOT used. It subjects the table
-- owner to the policy set, and the policy set here is empty on purpose. See
-- ADR-0023.

alter table users             enable row level security;
alter table conversations     enable row level security;
alter table messages          enable row level security;
alter table tool_executions   enable row level security;
alter table approval_requests enable row level security;
alter table audit_events      enable row level security;
alter table tasks             enable row level security;
alter table reminders         enable row level security;
alter table notes             enable row level security;
alter table ideas             enable row level security;

-- sqlx's own bookkeeping table. It is created by the migrator rather than by a
-- migration, so it is easy to forget; it still sits in `public` and is still
-- published like everything else.
alter table _sqlx_migrations  enable row level security;

-- Defence in depth. RLS alone would be enough, but a future table that someone
-- forgets to enable it on should not be reachable either, so the grant that
-- makes these roles interesting in the first place is withdrawn.
--
-- `service_role` keeps its grants: it holds BYPASSRLS regardless, and it is the
-- documented server-side role. Revoking from it would buy nothing.
revoke all on all tables    in schema public from anon, authenticated;
revoke all on all sequences in schema public from anon, authenticated;
revoke all on all functions in schema public from anon, authenticated;

-- The same grant is re-applied to every *future* table unless the default is
-- changed, which is how these tables came to be exposed in the first place.
-- This applies to objects created by `postgres`, which is the role migrations
-- run as.
alter default privileges in schema public revoke all on tables    from anon, authenticated;
alter default privileges in schema public revoke all on sequences from anon, authenticated;
alter default privileges in schema public revoke all on functions from anon, authenticated;
