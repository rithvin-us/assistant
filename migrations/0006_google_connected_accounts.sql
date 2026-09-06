-- Milestone 5 Google Connected Accounts & Task Scheduling Bridge

create table if not exists connected_accounts (
    id                    uuid primary key default gen_random_uuid(),
    user_id               uuid not null references users (id) on delete cascade,
    provider              text not null default 'google',
    provider_account_id   text not null,
    email                 text not null,
    display_name          text,
    scopes                text[] not null default '{}',
    encrypted_credentials bytea not null,
    status                text not null default 'active' check (status in ('active', 'expired', 'revoked')),
    created_at            timestamptz not null default now(),
    updated_at            timestamptz not null default now(),
    constraint connected_accounts_user_provider_account_uniq unique (user_id, provider, provider_account_id)
);

create index if not exists idx_connected_accounts_user
    on connected_accounts (user_id, provider, status);

create index if not exists idx_connected_accounts_email
    on connected_accounts (user_id, email);

-- Task to calendar bridge: duration estimate for scheduling & free-time planning
alter table tasks add column if not exists estimated_minutes integer check (estimated_minutes > 0);

-- Enforce Row Level Security with closed policy set (ADR-0023)
alter table connected_accounts enable row level security;
