//! Deterministic test doubles.
//!
//! Available to other crates behind the `testing` feature so the server's
//! integration tests exercise the same fakes as the core's unit tests. The
//! feature is off by default, so none of this reaches a release binary.
//!
//! Everything here is deterministic. No randomness, no wall-clock dependence
//! beyond explicit sleeps, and no network.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use assistant_auth::{DevTokenVerifier, Principal};
use assistant_tools::{RiskLevel, Tool, ToolCall, ToolError, ToolSpec};
use async_trait::async_trait;

/// A principal holding no scopes, matching the development verifier's user.
pub fn dev_principal() -> Principal {
    Principal {
        user_id: DevTokenVerifier::DEV_USER_ID,
        scopes: Vec::new(),
    }
}

/// A principal holding the given scopes.
pub fn principal_with(scopes: &[&str]) -> Principal {
    Principal {
        user_id: DevTokenVerifier::DEV_USER_ID,
        scopes: scopes.iter().map(|s| s.to_string()).collect(),
    }
}

/// A tool call as a model would propose it.
pub fn call(name: &str, arguments: serde_json::Value) -> ToolCall {
    ToolCall::new(format!("call_{name}"), name, arguments)
}

/// A `ToolSpec` with the given name, risk and required scopes.
pub fn spec_with(name: &str, risk: RiskLevel, required_scopes: &[&str]) -> ToolSpec {
    ToolSpec {
        name: name.to_string(),
        description: format!("test double for {name}"),
        input_schema: serde_json::json!({"type": "object"}),
        output_schema: serde_json::json!({"type": "object"}),
        risk,
        required_scopes: required_scopes.iter().map(|s| s.to_string()).collect(),
        timeout_ms: 1_000,
    }
}

/// A tool that returns its arguments and counts how many times it ran.
///
/// The call counter is the point: tests assert a denied or approval-held tool
/// was never invoked, which a return value alone cannot prove.
pub struct EchoTool {
    spec: ToolSpec,
    calls: AtomicUsize,
}

impl EchoTool {
    pub fn new(name: &str, risk: RiskLevel, required_scopes: &[&str]) -> Self {
        Self {
            spec: spec_with(name, risk, required_scopes),
            calls: AtomicUsize::new(0),
        }
    }

    pub fn green(name: &str) -> Self {
        Self::new(name, RiskLevel::Green, &[])
    }

    pub fn red(name: &str) -> Self {
        Self::new(name, RiskLevel::Red, &[])
    }

    /// How many times `execute` was entered.
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Tool for EchoTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn execute(&self, args: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(serde_json::json!({ "echo": args }))
    }
}

/// A tool that always fails, to prove a tool failure does not fail the turn.
pub struct FailingTool {
    spec: ToolSpec,
    calls: AtomicUsize,
}

impl FailingTool {
    pub fn new(name: &str) -> Self {
        Self {
            spec: spec_with(name, RiskLevel::Green, &[]),
            calls: AtomicUsize::new(0),
        }
    }

    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Tool for FailingTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn execute(&self, _args: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(ToolError::Failed("the upstream service said no".into()))
    }
}

/// A tool that sleeps longer than its own timeout allows.
pub struct SlowTool {
    spec: ToolSpec,
    sleep_ms: u64,
}

impl SlowTool {
    /// `sleep_ms` is how long the tool takes; its declared timeout is 20ms.
    pub fn new(name: &str, sleep_ms: u64) -> Self {
        let mut spec = spec_with(name, RiskLevel::Green, &[]);
        spec.timeout_ms = 20;
        Self { spec, sleep_ms }
    }
}

#[async_trait]
impl Tool for SlowTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn execute(&self, _args: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        tokio::time::sleep(std::time::Duration::from_millis(self.sleep_ms)).await;
        Ok(serde_json::json!({"finished": true}))
    }
}

/// Records the order in which tools ran, so concurrency behaviour is observable.
#[derive(Debug, Default, Clone)]
pub struct CallLog(Arc<std::sync::Mutex<Vec<String>>>);

impl CallLog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&self, entry: impl Into<String>) {
        self.0
            .lock()
            .expect("call log not poisoned")
            .push(entry.into());
    }

    pub fn entries(&self) -> Vec<String> {
        self.0.lock().expect("call log not poisoned").clone()
    }
}

/// A tool that logs entry, sleeps, then logs exit.
///
/// Interleaved entries prove concurrent execution; strictly paired entries prove
/// sequential execution.
pub struct TracingTool {
    spec: ToolSpec,
    log: CallLog,
    sleep_ms: u64,
}

impl TracingTool {
    pub fn new(name: &str, risk: RiskLevel, log: CallLog, sleep_ms: u64) -> Self {
        let mut spec = spec_with(name, risk, &[]);
        spec.timeout_ms = 5_000;
        Self {
            spec,
            log,
            sleep_ms,
        }
    }
}

#[async_trait]
impl Tool for TracingTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn execute(&self, _args: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        self.log.push(format!("enter:{}", self.spec.name));
        tokio::time::sleep(std::time::Duration::from_millis(self.sleep_ms)).await;
        self.log.push(format!("exit:{}", self.spec.name));
        Ok(serde_json::json!({"ok": true}))
    }
}

/// An in-memory [`ConversationStore`](crate::conversation::ConversationStore).
///
/// Exists so the orchestrator's persistence behaviour -- what is written, in
/// what order, and what is *not* written when a turn fails -- can be asserted
/// without a database, and so the server's WebSocket tests can prove the
/// end-to-end path without one either.
///
/// It is not a substitute for the Postgres tests. Ownership scoping, ordering
/// under concurrent writes and survival across a restart are properties of the
/// database, and are tested against a real one.
#[derive(Debug, Default)]
pub struct InMemoryConversationStore {
    state: std::sync::Mutex<InMemoryState>,
}

#[derive(Debug, Default)]
struct InMemoryState {
    conversations: std::collections::HashMap<uuid::Uuid, crate::conversation::Conversation>,
    messages: Vec<crate::conversation::StoredMessage>,
    next_seq: i64,
}

impl InMemoryConversationStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Every message written, in insertion order.
    pub fn all(&self) -> Vec<crate::conversation::StoredMessage> {
        self.state.lock().expect("not poisoned").messages.clone()
    }
}

#[async_trait]
impl crate::conversation::ConversationStore for InMemoryConversationStore {
    async fn ensure(
        &self,
        id: uuid::Uuid,
        principal_id: uuid::Uuid,
    ) -> Result<crate::conversation::Conversation, crate::conversation::ConversationError> {
        let mut state = self.state.lock().expect("not poisoned");
        if let Some(existing) = state.conversations.get(&id) {
            // An id that exists under another owner is reported as missing,
            // exactly as the Postgres implementation must.
            if existing.principal_id != principal_id {
                return Err(crate::conversation::ConversationError::NotFound(id));
            }
            return Ok(existing.clone());
        }

        let now = time::OffsetDateTime::now_utc();
        let conversation = crate::conversation::Conversation {
            id,
            principal_id,
            title: None,
            created_at: now,
            updated_at: now,
        };
        state.conversations.insert(id, conversation.clone());
        Ok(conversation)
    }

    async fn append(
        &self,
        message: &crate::conversation::NewMessage,
    ) -> Result<crate::conversation::StoredMessage, crate::conversation::ConversationError> {
        let mut state = self.state.lock().expect("not poisoned");
        match state.conversations.get(&message.conversation_id) {
            Some(conversation) if conversation.principal_id == message.principal_id => {}
            _ => {
                return Err(crate::conversation::ConversationError::NotFound(
                    message.conversation_id,
                ));
            }
        }

        state.next_seq += 1;
        let stored = crate::conversation::StoredMessage {
            id: message.id,
            conversation_id: message.conversation_id,
            turn_id: message.turn_id,
            role: message.role,
            content: message.content.clone(),
            tool_calls: message.tool_calls.clone(),
            tool_call_id: message.tool_call_id.clone(),
            seq: state.next_seq,
            created_at: time::OffsetDateTime::now_utc(),
        };
        state.messages.push(stored.clone());
        Ok(stored)
    }

    async fn history(
        &self,
        id: uuid::Uuid,
        principal_id: uuid::Uuid,
        limit: usize,
    ) -> Result<Vec<crate::conversation::StoredMessage>, crate::conversation::ConversationError>
    {
        let state = self.state.lock().expect("not poisoned");
        let owned = state
            .conversations
            .get(&id)
            .is_some_and(|conversation| conversation.principal_id == principal_id);
        if !owned {
            return Ok(Vec::new());
        }

        let mut messages: Vec<_> = state
            .messages
            .iter()
            .filter(|message| message.conversation_id == id)
            .cloned()
            .collect();
        messages.sort_by_key(|message| message.seq);
        let start = messages.len().saturating_sub(limit);
        Ok(messages.split_off(start))
    }
}
