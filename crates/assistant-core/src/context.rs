//! Context assembly.
//!
//! The orchestrator asks for context; it never knows where context came from.
//! Conversation history, memory, tasks, deadlines, calendar, projects, connected
//! accounts and documents will each become an implementation of this trait, and
//! none of them requires an orchestrator change to plug in.

use std::sync::Arc;

use async_trait::async_trait;

use crate::turn::{TurnContext, TurnRequest};

/// Error type a context source may fail with. Boxed so a provider can use its
/// own error type without every provider's error leaking into the core.
pub type ContextError = Box<dyn std::error::Error + Send + Sync>;

#[async_trait]
pub trait ContextProvider: Send + Sync {
    /// Gathers whatever is relevant to this request.
    ///
    /// Implementations must be safe to call on the latency-critical path: a
    /// slow context source delays every turn, so anything expensive belongs
    /// behind a cache or a background refresh rather than here.
    async fn assemble(&self, request: &TurnRequest) -> Result<TurnContext, ContextError>;
}

/// A provider that returns nothing.
///
/// This is the honest Milestone 2 default. There is no memory engine, no task
/// store and no calendar, so there is no context to assemble; returning empty is
/// accurate, whereas inventing plausible history would make the assistant appear
/// to remember things it does not.
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptyContextProvider;

#[async_trait]
impl ContextProvider for EmptyContextProvider {
    async fn assemble(&self, _request: &TurnRequest) -> Result<TurnContext, ContextError> {
        Ok(TurnContext::default())
    }
}

/// An in-memory conversation history, keyed by conversation.
///
/// Exists so the tool loop and the transport can be tested against real
/// multi-turn behaviour without a database. It is not the durable conversation
/// store: nothing here survives a restart, and ADR-0011 records that decision.
#[derive(Debug, Default)]
pub struct InMemoryContextProvider {
    conversations: tokio::sync::RwLock<
        std::collections::HashMap<crate::turn::ConversationId, Vec<crate::turn::ContextMessage>>,
    >,
}

impl InMemoryContextProvider {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn append(
        &self,
        conversation_id: crate::turn::ConversationId,
        message: crate::turn::ContextMessage,
    ) {
        self.conversations
            .write()
            .await
            .entry(conversation_id)
            .or_default()
            .push(message);
    }
}

#[async_trait]
impl ContextProvider for InMemoryContextProvider {
    async fn assemble(&self, request: &TurnRequest) -> Result<TurnContext, ContextError> {
        let conversations = self.conversations.read().await;
        Ok(TurnContext {
            history: conversations
                .get(&request.conversation_id)
                .cloned()
                .unwrap_or_default(),
            facts: Vec::new(),
        })
    }
}

/// Bounded memory retrieval, plugged into an inner context provider.
///
/// The rule this type exists to enforce: **memory influences context, it does
/// not replace it**. An inner provider (conversation history, or the empty
/// default) still runs; this wraps it and appends up to a hard cap of ranked
/// memories to the facts. The model never sees the whole memory set.
///
/// Retrieval is deterministic: the store is filtered on the authenticated
/// principal, expired temporaries are swept, and the results are ranked by
/// [`assistant_memory::rank`] against the raw request text. Nothing here
/// consults a model to decide what to fetch.
pub struct MemoryContextProvider {
    inner: Arc<dyn ContextProvider>,
    store: Arc<dyn assistant_memory::MemoryStore>,
    /// Hard cap on injected memories per turn.
    limit: usize,
}

impl MemoryContextProvider {
    /// Wraps `inner` with a bounded memory retrieval pass.
    ///
    /// `limit` is clamped to [`assistant_memory::MAX_CONTEXT_MEMORIES`] so a
    /// caller cannot silently widen the context window.
    pub fn new(
        inner: Arc<dyn ContextProvider>,
        store: Arc<dyn assistant_memory::MemoryStore>,
        limit: usize,
    ) -> Self {
        Self {
            inner,
            store,
            limit: limit.min(assistant_memory::MAX_CONTEXT_MEMORIES),
        }
    }
}

#[async_trait]
impl ContextProvider for MemoryContextProvider {
    async fn assemble(&self, request: &TurnRequest) -> Result<TurnContext, ContextError> {
        let mut context = self.inner.assemble(request).await?;

        let user_id = request.principal.user_id;
        let now = time::OffsetDateTime::now_utc();

        // Expiring temporaries on the retrieval path keeps M7's promise
        // (`sweep_expired` on read) with no scheduler.
        let _ = self.store.sweep_expired(user_id, now).await;

        let query = assistant_memory::MemoryQuery {
            user_id,
            kinds: Vec::new(),
            lifecycles: vec![assistant_memory::Lifecycle::Active],
            min_importance: None,
            text: Some(request.input.clone()),
            // Ask the store for a somewhat wider pool so the deterministic
            // ranker below has room to sort, then cap at `self.limit`.
            limit: (self.limit as u32 * 4).max(8),
        };

        let candidates = self
            .store
            .search(query)
            .await
            .map_err(|error| Box::new(error) as ContextError)?;
        if candidates.is_empty() {
            return Ok(context);
        }

        let ranked = assistant_memory::rank(&candidates, Some(request.input.as_str()), now);
        let selected: Vec<assistant_memory::Memory> = ranked
            .into_iter()
            .take(self.limit)
            .map(|scored| scored.memory)
            .collect();
        if selected.is_empty() {
            return Ok(context);
        }

        // Register the retrieval as an access. `touch` moves `last_accessed_at`
        // and increments `access_count`; incidental reads do not do this,
        // which is what makes the counter meaningful.
        let ids: Vec<uuid::Uuid> = selected.iter().map(|memory| memory.id).collect();
        let _ = self.store.touch(user_id, &ids, now).await;

        context.facts.extend(selected.into_iter().map(|memory| {
            format!(
                "[memory:{kind}] {content}",
                kind = memory.kind.as_str(),
                content = memory.content
            )
        }));

        Ok(context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::turn::{ContextMessage, ContextRole};
    use assistant_auth::{DevTokenVerifier, Principal};
    use uuid::Uuid;

    fn principal() -> Principal {
        Principal {
            user_id: DevTokenVerifier::DEV_USER_ID,
            scopes: vec![],
        }
    }

    #[tokio::test]
    async fn empty_provider_returns_nothing_rather_than_inventing_history() {
        let request = TurnRequest::new(Uuid::new_v4(), principal(), "hello");
        let context = EmptyContextProvider.assemble(&request).await.expect("ok");
        assert!(context.history.is_empty());
        assert!(context.facts.is_empty());
    }

    #[tokio::test]
    async fn in_memory_provider_scopes_history_to_its_conversation() {
        let provider = InMemoryContextProvider::new();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();

        provider
            .append(
                a,
                ContextMessage {
                    role: ContextRole::User,
                    content: "first".into(),
                },
            )
            .await;

        let in_a = provider
            .assemble(&TurnRequest::new(a, principal(), "x"))
            .await
            .expect("ok");
        let in_b = provider
            .assemble(&TurnRequest::new(b, principal(), "x"))
            .await
            .expect("ok");

        assert_eq!(in_a.history.len(), 1);
        assert!(
            in_b.history.is_empty(),
            "history leaked across conversations"
        );
    }
}
