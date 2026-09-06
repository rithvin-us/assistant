-- Conversation history: the messages of a conversation.
--
-- Applied with: scripts/migrate.ps1   (sqlx migrate run). Never on boot.
--
-- `conversations` already exists from 0001 and is unchanged here: the id the
-- client already uses on the WebSocket is the conversation id, so there is no
-- second identifier to reconcile.
--
-- What this is NOT: memory. These rows are the log of what was said in one
-- conversation. Nothing is promoted from here into a durable fact about the
-- user, scored for importance, embedded, or retrieved semantically. That is a
-- later milestone with its own schema and its own retention decision. See
-- ADR-0021.
--
-- Storage boundary. A message row holds exactly what was said, because
-- replaying the conversation is the whole point of the table. That makes it the
-- most sensitive table in the database, and the reason the audit trail
-- (0002_durable_actions.sql) deliberately holds none of it: audit rows are
-- written by generic code that cannot tell an email body from a calendar title,
-- whereas these rows are only ever read back to the one principal who owns them.

create table if not exists messages (
    id               uuid        primary key,
    conversation_id  uuid        not null references conversations (id) on delete cascade,
    -- Correlates every message produced by one turn: the user's question, the
    -- tool calls it caused, and the answer.
    turn_id          uuid        not null,
    -- Ordering. Not `created_at`: two messages written in the same millisecond
    -- still have an order, and a conversation replayed in the wrong order is a
    -- different conversation.
    seq              bigint      not null generated always as identity,
    role             text        not null check (role in ('user', 'assistant', 'tool')),
    content          text        not null,
    -- Structured tool calls proposed by an assistant turn. A narrowly scoped
    -- field beside `content` rather than prose encoded into it, so
    -- reconstruction never depends on parsing English.
    tool_calls       jsonb       not null default '[]'::jsonb,
    -- For a tool message, the id of the call it answers.
    tool_call_id     text,
    created_at       timestamptz not null default now(),

    -- A tool result must say which call it answers, and nothing else may claim
    -- to. Without this a result could be orphaned, or attached to a call it did
    -- not come from.
    constraint messages_tool_result_names_its_call check (
        (role = 'tool' and tool_call_id is not null)
        or (role <> 'tool' and tool_call_id is null)
    ),

    -- Only an assistant turn can propose tool calls. The schema says so because
    -- a user-supplied tool call is exactly the shape an injection attempt would
    -- take.
    constraint messages_only_assistants_propose_tools check (
        role = 'assistant' or tool_calls = '[]'::jsonb
    )
);

-- Serves the only read this milestone performs: the most recent N messages of
-- one conversation, newest first.
create index if not exists messages_conversation_seq_idx
    on messages (conversation_id, seq desc);

-- Serves reconstruction of a single turn, and the audit trail's turn_id.
create index if not exists messages_turn_idx
    on messages (turn_id, seq);
