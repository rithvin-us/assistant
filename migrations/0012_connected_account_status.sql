-- The status values the application actually writes.
--
-- 0006 constrained `status` to ('active','expired','revoked'), but the server
-- writes 'disconnected' when a user disconnects an account
-- (google/client.rs::disconnect_account) and 'error' when a token refresh
-- fails. Both violated the check, so every disconnect returned 23514 and the
-- refresh-failure paths -- which discarded their result with `let _ =` --
-- failed silently. An account the user had revoked stayed 'active', its stored
-- credentials kept being used, and its scopes kept entering `Principal`.
--
-- Revocation being inoperative is the reason this is a migration and not a
-- code-only change: the constraint, not the Rust, was wrong. 'expired' and
-- 'revoked' are kept even though nothing writes them today, because rows may
-- already hold them.
alter table connected_accounts
    drop constraint if exists connected_accounts_status_check;

alter table connected_accounts
    add constraint connected_accounts_status_check
    check (status in ('active', 'expired', 'revoked', 'disconnected', 'error'));

-- Only an 'active' account is usable, and that is checked on every token load.
-- This index keeps the common "list my usable accounts" read cheap.
create index if not exists connected_accounts_user_status_idx
    on connected_accounts (user_id, status);
