//! Tool registry.
//!
//! The registry is the single source of truth for what a tool *is*. A model may
//! propose a call by name and supply arguments; everything else -- the risk
//! level, the required scopes, the timeout, the schemas -- is read from here.
//!
//! This is the mechanism behind ADR-0005. A `ToolCall` that arrives from a model
//! carries no authority, and there is deliberately no code path that lets one
//! contribute a `RiskLevel`.

use std::{collections::HashMap, sync::Arc};

use assistant_tools::{Tool, ToolSpec};

#[derive(Default)]
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a tool under the name in its own spec.
    ///
    /// Returns the previously registered tool of the same name, if any. Callers
    /// that must not shadow an existing tool should check for `Some`.
    pub fn register(&mut self, tool: Arc<dyn Tool>) -> Option<Arc<dyn Tool>> {
        let name = tool.spec().name.clone();
        self.tools.insert(name, tool)
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    /// The authoritative spec for a tool. This -- not the model's message -- is
    /// what the permission engine evaluates.
    pub fn spec(&self, name: &str) -> Option<ToolSpec> {
        self.tools.get(name).map(|tool| tool.spec().clone())
    }

    pub fn contains(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// Declarations offered to the model, in a stable order so that prompts are
    /// reproducible and caching is not defeated by HashMap iteration order.
    pub fn declarations(&self) -> Vec<ToolSpec> {
        let mut specs: Vec<ToolSpec> = self
            .tools
            .values()
            .map(|tool| tool.spec().clone())
            .collect();
        specs.sort_by(|a, b| a.name.cmp(&b.name));
        specs
    }

    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.tools.keys().cloned().collect();
        names.sort();
        names
    }
}

impl std::fmt::Debug for ToolRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolRegistry")
            .field("tools", &self.names())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::EchoTool;
    use assistant_tools::RiskLevel;

    #[test]
    fn registered_tool_resolves_by_name() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool::green("notes.read")));

        assert!(registry.contains("notes.read"));
        assert!(registry.get("notes.read").is_some());
        assert_eq!(
            registry.spec("notes.read").expect("spec").risk,
            RiskLevel::Green
        );
    }

    #[test]
    fn unknown_tool_resolves_to_nothing() {
        let registry = ToolRegistry::new();
        assert!(!registry.contains("gmail.send"));
        assert!(registry.get("gmail.send").is_none());
        assert!(registry.spec("gmail.send").is_none());
    }

    #[test]
    fn declarations_are_ordered_so_prompts_are_reproducible() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool::green("z.tool")));
        registry.register(Arc::new(EchoTool::green("a.tool")));
        registry.register(Arc::new(EchoTool::green("m.tool")));

        let names: Vec<String> = registry
            .declarations()
            .into_iter()
            .map(|spec| spec.name)
            .collect();
        assert_eq!(names, ["a.tool", "m.tool", "z.tool"]);
    }

    #[test]
    fn the_registry_spec_is_authoritative_not_the_proposed_name() {
        // A tool registered as Red stays Red no matter what any caller believes.
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool::red("gmail.send")));

        assert_eq!(
            registry.spec("gmail.send").expect("spec").risk,
            RiskLevel::Red
        );
    }
}
