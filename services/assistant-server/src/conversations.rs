//! Postgres implementation of [`ConversationStore`].
//!
//! Lives here, not in `assistant-core`, because `sqlx` is infrastructure and
//! the core depends on interfaces. Same rule as `PostgresActionStore`.
//!
//! Ownership is the property this file exists to guarantee, and it is enforced
//! in SQL rather than in Rust. Every statement either joins `conversations` on
//! `user_id` or selects the conversation by `(id, user_id)`; there is no code
//! path that reads a row first and checks who owns it afterwards, because that
//! is the shape of check people forget to write. A conversation belonging to
//! somebody else produces exactly what a nonexistent one produces.

use assistant_core::conversation::{
    Conversation, ConversationError, ConversationStore, MessageRole, NewMessage, StoredMessage,
};
use assistant_tools::ToolCall;
use async_trait::async_trait;
use sqlx::{PgPool, Row, postgres::PgRow};
use uuid::Uuid;

pub struct PostgresConversationStore {
    pool: PgPool,
}

impl PostgresConversationStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn backend(error: sqlx::Error) -> ConversationError {
    ConversationError::Backend(Box::new(error))
}

fn conversation_from_row(row: &PgRow) -> Result<Conversation, ConversationError> {
    Ok(Conversation {
        id: row.try_get("id").map_err(backend)?,
        principal_id: row.try_get("user_id").map_err(backend)?,
        title: row.try_get("title").map_err(backend)?,
        created_at: row.try_get("created_at").map_err(backend)?,
        updated_at: row.try_get("updated_at").map_err(backend)?,
    })
}

fn message_from_row(row: &PgRow) -> Result<StoredMessage, ConversationError> {
    let role: String = row.try_get("role").map_err(backend)?;
    let role = MessageRole::parse(&role).ok_or_else(|| {
        ConversationError::Backend(format!("unknown message role in database: {role}").into())
    })?;

    let tool_calls: serde_json::Value = row.try_get("tool_calls").map_err(backend)?;
    let tool_calls: Vec<ToolCall> = serde_json::from_value(tool_calls).map_err(|error| {
        ConversationError::Backend(format!("stored tool calls were unreadable: {error}").into())
    })?;

    Ok(StoredMessage {
        id: row.try_get("id").map_err(backend)?,
        conversation_id: row.try_get("conversation_id").map_err(backend)?,
        turn_id: row.try_get("turn_id").map_err(backend)?,
        role,
        content: row.try_get("content").map_err(backend)?,
        tool_calls,
        tool_call_id: row.try_get("tool_call_id").map_err(backend)?,
        seq: row.try_get("seq").map_err(backend)?,
        created_at: row.try_get("created_at").map_err(backend)?,
    })
}

#[async_trait]
impl ConversationStore for PostgresConversationStore {
    /// Creates the conversation for this principal, or returns the existing one.
    ///
    /// One transaction, and the final `SELECT` is scoped by `(id, user_id)`.
    /// That is what makes an id owned by somebody else come back as
    /// [`ConversationError::NotFound`]: the insert does nothing because the row
    /// exists, and the select finds nothing because the owner differs. The
    /// conversation is never silently re-parented.
    async fn ensure(
        &self,
        id: Uuid,
        principal_id: Uuid,
    ) -> Result<Conversation, ConversationError> {
        let mut tx = self.pool.begin().await.map_err(backend)?;

        // `conversations.user_id` references `users`, which is the local user
        // registry migration 0001 created for exactly this. The authenticated
        // principal is the identity; this row is its foreign-key anchor, not a
        // second source of truth about who the user is.
        sqlx::query("insert into users (id) values ($1) on conflict (id) do nothing")
            .bind(principal_id)
            .execute(&mut *tx)
            .await
            .map_err(backend)?;

        sqlx::query(
            "insert into conversations (id, user_id) values ($1, $2) on conflict (id) do nothing",
        )
        .bind(id)
        .bind(principal_id)
        .execute(&mut *tx)
        .await
        .map_err(backend)?;

        let row = sqlx::query(
            "select id, user_id, title, created_at, updated_at
             from conversations
             where id = $1 and user_id = $2",
        )
        .bind(id)
        .bind(principal_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(backend)?;

        let conversation = match row {
            Some(row) => conversation_from_row(&row)?,
            None => return Err(ConversationError::NotFound(id)),
        };

        tx.commit().await.map_err(backend)?;
        Ok(conversation)
    }

    /// Appends one message.
    ///
    /// The insert selects its `conversation_id` *from* `conversations` filtered
    /// by owner, so an unowned conversation inserts zero rows rather than
    /// inserting a row nobody checked. `updated_at` moves in the same
    /// transaction, so a conversation cannot gain a message without its
    /// ordering metadata following.
    async fn append(&self, message: &NewMessage) -> Result<StoredMessage, ConversationError> {
        let mut tx = self.pool.begin().await.map_err(backend)?;

        let row = sqlx::query(
            "insert into messages
                 (id, conversation_id, turn_id, role, content, tool_calls, tool_call_id)
             select $1, c.id, $3, $4, $5, $6, $7
             from conversations c
             where c.id = $2 and c.user_id = $8
             returning id, conversation_id, turn_id, seq, role, content, tool_calls,
                       tool_call_id, created_at",
        )
        .bind(message.id)
        .bind(message.conversation_id)
        .bind(message.turn_id)
        .bind(message.role.as_str())
        .bind(&message.content)
        .bind(serde_json::to_value(&message.tool_calls).map_err(|error| {
            ConversationError::Backend(format!("tool calls were not serialisable: {error}").into())
        })?)
        .bind(message.tool_call_id.as_deref())
        .bind(message.principal_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(backend)?;

        let Some(row) = row else {
            return Err(ConversationError::NotFound(message.conversation_id));
        };
        let stored = message_from_row(&row)?;

        sqlx::query("update conversations set updated_at = now() where id = $1 and user_id = $2")
            .bind(message.conversation_id)
            .bind(message.principal_id)
            .execute(&mut *tx)
            .await
            .map_err(backend)?;

        tx.commit().await.map_err(backend)?;
        Ok(stored)
    }

    /// The most recent `limit` messages, oldest first.
    ///
    /// The bound is in the query. Reading a whole conversation and slicing it
    /// in Rust would make a long conversation slow and expensive before it made
    /// it wrong.
    async fn history(
        &self,
        id: Uuid,
        principal_id: Uuid,
        limit: usize,
    ) -> Result<Vec<StoredMessage>, ConversationError> {
        let rows = sqlx::query(
            "select m.id, m.conversation_id, m.turn_id, m.seq, m.role, m.content,
                    m.tool_calls, m.tool_call_id, m.created_at
             from messages m
             join conversations c on c.id = m.conversation_id
             where m.conversation_id = $1 and c.user_id = $2
             order by m.seq desc
             limit $3",
        )
        .bind(id)
        .bind(principal_id)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(backend)?;

        let mut messages = rows
            .iter()
            .map(message_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        messages.reverse();
        Ok(messages)
    }
}
