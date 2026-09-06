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
    ToolCall {
        id: format!("call_{name}"),
        name: name.to_string(),
        arguments,
    }
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
