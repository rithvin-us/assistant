//! Durable conversation history.
//!
//! This module defines *what* a conversation is and what must be stored about
//! it. It does not know that Postgres exists: the SQL implementation lives in
//! `assistant-server`, which already owns `sqlx` and the pool. That is the same
//! rule that keeps model providers out of the core (ADR-0003) and durable
//! actions out of it (ADR-0014), applied a third time.
//!
//! Two properties are the store's contract rather than the caller's discipline:
//!
//! * **Ownership is part of every query.** Every method takes the authenticated
//!   principal, and an implementation must scope the SQL by it rather than
//!   filtering rows after they are read. A conversation belonging to someone
//!   else must be indistinguishable from one that does not exist.
//! * **Roles are not flattened.** A tool result is stored as a tool message
//!   with the id of the call it answers, not as prose in an assistant turn, so
//!   a turn can be reconstructed later without guessing.
//!
//! What this is *not*: memory. A conversation is the log of what was said in
//! one conversation. Nothing here is promoted to a durable fact about the user,
//! scored for importance, embedded or retrieved semantically -- see ADR-0021
//! and `crates/assistant-memory`.

use async_trait::async_trait;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::turn::{
    ContextMessage, ContextRole, ConversationId, MessageId, TurnContext, TurnId, TurnRequest,
};
use assistant_tools::ToolCall;

/// Who produced a stored message.
///
/// Mirrors [`assistant_models::Role`] minus `System`, and the omission is a
/// security property rather than an economy: the system prompt is server
/// configuration assembled per request, never conversation data. If a system
/// message could be *stored*, a client that could write one would be able to
/// change the assistant's instructions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageRole {
    User,
    Assistant,
    Tool,
}

impl MessageRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "user" => Self::User,
            "assistant" => Self::Assistant,
            "tool" => Self::Tool,
            _ => return None,
        })
    }
}

/// A conversation, owned by exactly one principal.
#[derive(Debug, Clone)]
pub struct Conversation {
    pub id: ConversationId,
    pub principal_id: Uuid,
    pub title: Option<String>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

/// A message about to be written.
#[derive(Debug, Clone)]
pub struct NewMessage {
    pub id: MessageId,
    pub conversation_id: ConversationId,
    /// The owner. Passed so the insert can be scoped rather than trusted.
    pub principal_id: Uuid,
    /// Correlates every message produced by one turn.
    pub turn_id: TurnId,
    pub role: MessageRole,
    pub content: String,
    /// Tool calls this assistant turn proposed. Empty for every other role.
    ///
    /// A narrowly scoped structured field, kept beside `content` rather than
    /// encoded into it: the plain content model stays plain, and reconstruction
    /// does not depend on parsing prose.
    pub tool_calls: Vec<ToolCall>,
    /// For [`MessageRole::Tool`], the id of the call this message answers.
    pub tool_call_id: Option<String>,
}

impl NewMessage {
    pub fn user(
        conversation_id: ConversationId,
        principal_id: Uuid,
        turn_id: TurnId,
        content: impl Into<String>,
    ) -> Self {
        Self::plain(
            conversation_id,
            principal_id,
            turn_id,
            MessageRole::User,
            content,
        )
    }

    pub fn assistant(
        conversation_id: ConversationId,
        principal_id: Uuid,
        turn_id: TurnId,
        content: impl Into<String>,
    ) -> Self {
        Self::plain(
            conversation_id,
            principal_id,
            turn_id,
            MessageRole::Assistant,
            content,
        )
    }

    pub fn with_id(mut self, id: MessageId) -> Self {
        self.id = id;
        self
    }

    pub fn with_tool_calls(mut self, tool_calls: Vec<ToolCall>) -> Self {
        self.tool_calls = tool_calls;
        self
    }

    pub fn tool(
        conversation_id: ConversationId,
        principal_id: Uuid,
        turn_id: TurnId,
        tool_call_id: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            tool_call_id: Some(tool_call_id.into()),
            ..Self::plain(
                conversation_id,
                principal_id,
                turn_id,
                MessageRole::Tool,
                content,
            )
        }
    }

    fn plain(
        conversation_id: ConversationId,
        principal_id: Uuid,
        turn_id: TurnId,
        role: MessageRole,
        content: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            conversation_id,
            principal_id,
            turn_id,
            role,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }
}

/// A message as it came back out of the store.
#[derive(Debug, Clone)]
pub struct StoredMessage {
    pub id: MessageId,
    pub conversation_id: ConversationId,
    pub turn_id: TurnId,
    pub role: MessageRole,
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub tool_call_id: Option<String>,
    /// Monotonic within a conversation. Ordering is by this, not by timestamp:
    /// two messages written in the same millisecond still have an order.
    pub seq: i64,
    pub created_at: OffsetDateTime,
}

#[derive(Debug, thiserror::Error)]
pub enum ConversationError {
    /// No such conversation *for this principal*. The two cases are
    /// deliberately indistinguishable, so a caller cannot probe for the
    /// existence of somebody else's conversation.
    #[error("that conversation is not available")]
    NotFound(ConversationId),
    #[error("conversation store unavailable")]
    Unavailable(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("conversation store failed")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

#[async_trait]
pub trait ConversationStore: Send + Sync {
    /// Returns the conversation, creating it for this principal if it does not
    /// exist yet.
    ///
    /// Implementations must not hand back a conversation owned by anybody else,
    /// and must not adopt one either: an id that exists under a different owner
    /// is [`ConversationError::NotFound`], never a silent re-parenting.
    async fn ensure(
        &self,
        id: ConversationId,
        principal_id: Uuid,
    ) -> Result<Conversation, ConversationError>;

    /// Appends one message and returns it with its assigned ordering.
    async fn append(&self, message: &NewMessage) -> Result<StoredMessage, ConversationError>;

    /// The most recent `limit` messages, returned oldest first.
    ///
    /// Bounded at the query, not in the caller: an unbounded read of a long
    /// conversation is a latency and cost problem before it is a context
    /// problem.
    async fn history(
        &self,
        id: ConversationId,
        principal_id: Uuid,
        limit: usize,
    ) -> Result<Vec<StoredMessage>, ConversationError>;
}

/// How much history is replayed to the model.
///
/// Two independent bounds, because either alone is escapable: a message count
/// says nothing about a conversation of enormous messages, and a character
/// budget alone would happily replay a thousand tiny ones.
#[derive(Debug, Clone, Copy)]
pub struct ContextWindow {
    pub max_messages: usize,
    pub max_chars: usize,
}

impl Default for ContextWindow {
    fn default() -> Self {
        Self {
            // Twenty exchanges. Enough that "what did I just say" works and a
            // name given a few turns ago is still there; small enough that a
            // long conversation cannot quietly become an expensive one.
            max_messages: 40,
            max_chars: 24_000,
        }
    }
}

/// A [`ContextProvider`](crate::context::ContextProvider) backed by the durable
/// store.
///
/// The read path only. Writing is the orchestrator's job, because what to
/// persist depends on how the turn went, and a context provider is not told
/// that.
pub struct StoredContextProvider {
    store: std::sync::Arc<dyn ConversationStore>,
    window: ContextWindow,
}

impl StoredContextProvider {
    pub fn new(store: std::sync::Arc<dyn ConversationStore>, window: ContextWindow) -> Self {
        Self { store, window }
    }
}

#[async_trait]
impl crate::context::ContextProvider for StoredContextProvider {
    async fn assemble(
        &self,
        request: &TurnRequest,
    ) -> Result<TurnContext, crate::context::ContextError> {
        let history = self
            .store
            .history(
                request.conversation_id,
                request.principal.user_id,
                self.window.max_messages,
            )
            .await?;

        Ok(TurnContext {
            history: replayable(history, self.window),
            facts: Vec::new(),
        })
    }
}

/// Selects what of a stored history is replayed to the model.
///
/// Two decisions are encoded here.
///
/// *Tool messages from earlier turns are not replayed.* A tool result matters
/// while the turn that produced it is still running -- and there it is already
/// in the message list the orchestrator builds -- but replaying a stale one
/// invites the model to treat last week's inbox as current. The record of the
/// call still exists in the store; it simply is not context.
///
/// *The budget is spent from the newest end.* When history does not fit, the
/// oldest messages are dropped, because the recent ones are the ones the
/// current question is about.
fn replayable(history: Vec<StoredMessage>, window: ContextWindow) -> Vec<ContextMessage> {
    let mut selected: Vec<ContextMessage> = Vec::new();
    let mut budget = window.max_chars;

    for message in history.into_iter().rev() {
        let role = match message.role {
            MessageRole::User => ContextRole::User,
            MessageRole::Assistant => ContextRole::Assistant,
            MessageRole::Tool => continue,
        };

        if message.content.trim().is_empty() {
            continue;
        }
        if message.content.len() > budget {
            break;
        }

        budget -= message.content.len();
        selected.push(ContextMessage {
            role,
            content: message.content,
        });
    }

    selected.reverse();
    selected
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::ContextProvider;
    use crate::testing::InMemoryConversationStore;
    use assistant_auth::DevTokenVerifier;
    use std::sync::Arc;

    fn principal() -> assistant_auth::Principal {
        assistant_auth::Principal {
            user_id: DevTokenVerifier::DEV_USER_ID,
            scopes: Vec::new(),
        }
    }

    fn stored(role: MessageRole, content: &str, seq: i64) -> StoredMessage {
        StoredMessage {
            id: Uuid::new_v4(),
            conversation_id: Uuid::nil(),
            turn_id: Uuid::nil(),
            role,
            content: content.to_string(),
            tool_calls: Vec::new(),
            tool_call_id: None,
            seq,
            created_at: OffsetDateTime::now_utc(),
        }
    }

    #[test]
    fn tool_messages_are_stored_but_not_replayed_as_context() {
        let replayed = replayable(
            vec![
                stored(MessageRole::User, "read my notes", 1),
                stored(MessageRole::Assistant, "Checking.", 2),
                stored(MessageRole::Tool, "{\"notes\":[]}", 3),
                stored(MessageRole::Assistant, "Nothing there.", 4),
            ],
            ContextWindow::default(),
        );

        assert_eq!(replayed.len(), 3);
        assert!(
            replayed
                .iter()
                .all(|entry| entry.content != "{\"notes\":[]}")
        );
    }

    #[test]
    fn the_window_drops_the_oldest_messages_first() {
        let history: Vec<StoredMessage> = (0..10)
            .map(|i| stored(MessageRole::User, &format!("message {i}"), i))
            .collect();

        let replayed = replayable(
            history,
            ContextWindow {
                max_messages: 10,
                // Three messages of "message N" fit; the rest do not.
                max_chars: 27,
            },
        );

        assert_eq!(replayed.len(), 3);
        assert_eq!(replayed[0].content, "message 7");
        assert_eq!(replayed[2].content, "message 9");
    }

    #[tokio::test]
    async fn history_is_scoped_to_the_conversation_and_its_owner() {
        let store = Arc::new(InMemoryConversationStore::new());
        let mine = Uuid::new_v4();
        let theirs = Uuid::new_v4();
        let owner = principal().user_id;
        let other = Uuid::new_v4();

        store.ensure(mine, owner).await.expect("created");
        store.ensure(theirs, other).await.expect("created");

        store
            .append(&NewMessage::user(mine, owner, Uuid::new_v4(), "mine"))
            .await
            .expect("appended");
        store
            .append(&NewMessage::user(theirs, other, Uuid::new_v4(), "theirs"))
            .await
            .expect("appended");

        let provider = StoredContextProvider::new(store.clone(), ContextWindow::default());
        let context = provider
            .assemble(&TurnRequest::new(mine, principal(), "hello"))
            .await
            .expect("assembled");
        assert_eq!(context.history.len(), 1);
        assert_eq!(context.history[0].content, "mine");

        // The same id, asked for by the wrong principal, yields nothing.
        let context = provider
            .assemble(&TurnRequest::new(theirs, principal(), "hello"))
            .await
            .expect("assembled");
        assert!(
            context.history.is_empty(),
            "history leaked across principals"
        );
    }
}
