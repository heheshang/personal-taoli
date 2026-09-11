//! Host-side tool mechanism for the codex agent runtime.
//!
//! This module is the *mechanism*: it turns host capabilities into the
//! `DynamicToolSpec` wire shape, and routes the runtime's `item/tool/call`
//! requests to the implementation that declared the name. Concrete tools (and
//! their policies) live in sibling modules.
//!
//! Boundary rule enforced here: a call naming a tool that was never registered
//! is **rejected**, never reinterpreted. The registry deliberately has no
//! fallback — an unknown name is an error the runtime passes back to the model,
//! not an invitation to guess or to let codex's built-in tools stand in.

use std::collections::BTreeMap;

use serde_json::{Value, json};

/// One entry of `thread/start.dynamicTools`.
///
/// Wire shape measured against `--experimental` schema output:
/// `{type: "function", name, description, inputSchema}`.
#[derive(Debug, Clone)]
pub struct DynamicToolSpec {
    pub name: String,
    pub description: String,
    /// JSON Schema for the call arguments.
    pub input_schema: Value,
}

impl DynamicToolSpec {
    pub fn to_wire(&self) -> Value {
        json!({
            "type": "function",
            "name": self.name,
            "description": self.description,
            "inputSchema": self.input_schema,
        })
    }
}

/// A piece of tool output returned to the model.
///
/// Only text is modelled: images and audio are part of the wire schema but no
/// round needs them, and an unimplemented variant is better absent than faked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentItem {
    Text(String),
}

impl ContentItem {
    fn to_wire(&self) -> Value {
        match self {
            Self::Text(text) => json!({ "type": "inputText", "text": text }),
        }
    }
}

/// Outcome of one tool call, as the runtime expects it
/// (`DynamicToolCallResponse`: `{contentItems, success}`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutcome {
    pub success: bool,
    pub content_items: Vec<ContentItem>,
}

impl ToolOutcome {
    /// Successful result carrying `text`.
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            success: true,
            content_items: vec![ContentItem::Text(text.into())],
        }
    }

    /// Failed result carrying an explanation the model can act on.
    ///
    /// `success: false` is how the runtime is told the call did not work;
    /// returning empty text instead would look like a successful empty answer.
    pub fn failed(reason: impl Into<String>) -> Self {
        Self {
            success: false,
            content_items: vec![ContentItem::Text(reason.into())],
        }
    }

    pub fn to_wire(&self) -> Value {
        json!({
            "success": self.success,
            "contentItems": self
                .content_items
                .iter()
                .map(ContentItem::to_wire)
                .collect::<Vec<_>>(),
        })
    }

    /// First text item, if any. Used when recording what the model was told.
    pub fn first_text(&self) -> Option<&str> {
        self.content_items
            .iter()
            .map(|item| match item {
                ContentItem::Text(text) => text.as_str(),
            })
            .next()
    }
}

/// A capability the host exposes to the agent runtime.
///
/// Implementations must be **read-only** with respect to the trading path: the
/// agent may observe state, never mutate it. `call` is synchronous because the
/// host's data paths are synchronous file/database reads; the caller runs it on
/// a blocking thread so the async runtime is never parked on I/O.
pub trait AgentTool: Send + Sync {
    /// Name as it appears to the model and on the wire.
    fn name(&self) -> &'static str;

    /// Human-readable description; this is prompt surface for the model.
    fn description(&self) -> &'static str;

    /// JSON Schema for the arguments.
    fn input_schema(&self) -> Value;

    /// Executes the call. `arguments` is whatever the model produced and is
    /// therefore untrusted input.
    fn call(&self, arguments: &Value) -> ToolOutcome;
}

/// Why a dispatch failed before any tool ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchError {
    /// The model asked for a tool that was not registered on this session.
    UnknownTool {
        requested: String,
        registered: Vec<String>,
    },
}

impl std::fmt::Display for DispatchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownTool {
                requested,
                registered,
            } => write!(
                formatter,
                "unknown tool `{requested}`; this session exposes only: {}",
                if registered.is_empty() {
                    "(none)".to_string()
                } else {
                    registered.join(", ")
                }
            ),
        }
    }
}

impl std::error::Error for DispatchError {}

/// The set of tools exposed to one thread.
///
/// Ordering is by name so that the spec list sent to the runtime — and any
/// error message naming the registered tools — is deterministic.
#[derive(Default)]
pub struct ToolRegistry {
    tools: BTreeMap<String, Box<dyn AgentTool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `tool`, returning the previous implementation if the name was
    /// already taken.
    ///
    /// Overwriting silently would make "which tool answered?" depend on
    /// registration order; the caller must decide.
    pub fn register(&mut self, tool: Box<dyn AgentTool>) -> Option<Box<dyn AgentTool>> {
        self.tools.insert(tool.name().to_string(), tool)
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    pub fn names(&self) -> Vec<String> {
        self.tools.keys().cloned().collect()
    }

    /// The `dynamicTools` payload for `thread/start`.
    pub fn specs(&self) -> Vec<Value> {
        self.tools
            .values()
            .map(|tool| {
                DynamicToolSpec {
                    name: tool.name().to_string(),
                    description: tool.description().to_string(),
                    input_schema: tool.input_schema(),
                }
                .to_wire()
            })
            .collect()
    }

    /// Runs the tool named `name`.
    ///
    /// Unknown names are rejected here, at the single routing point, so no call
    /// path can bypass the check.
    pub fn dispatch(&self, name: &str, arguments: &Value) -> Result<ToolOutcome, DispatchError> {
        let Some(tool) = self.tools.get(name) else {
            return Err(DispatchError::UnknownTool {
                requested: name.to_string(),
                registered: self.names(),
            });
        };
        Ok(tool.call(arguments))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EchoTool;

    impl AgentTool for EchoTool {
        fn name(&self) -> &'static str {
            "probe_echo"
        }
        fn description(&self) -> &'static str {
            "Echo the `text` argument."
        }
        fn input_schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": { "text": { "type": "string" } },
                "required": ["text"],
                "additionalProperties": false,
            })
        }
        fn call(&self, arguments: &Value) -> ToolOutcome {
            match arguments.get("text").and_then(Value::as_str) {
                Some(text) => ToolOutcome::text(text),
                // Argument validation is the tool's job; the registry only
                // routes.
                None => ToolOutcome::failed("missing required argument `text`"),
            }
        }
    }

    fn registry() -> ToolRegistry {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(EchoTool));
        registry
    }

    #[test]
    fn specs_use_the_function_wire_shape() {
        let specs = registry().specs();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0]["type"], json!("function"));
        assert_eq!(specs[0]["name"], json!("probe_echo"));
        assert_eq!(specs[0]["inputSchema"]["required"][0], json!("text"));
    }

    #[test]
    fn dispatch_routes_to_the_registered_tool() {
        let outcome = registry()
            .dispatch("probe_echo", &json!({ "text": "hello" }))
            .expect("registered tool");
        assert_eq!(outcome, ToolOutcome::text("hello"));
    }

    /// The core boundary property: an unregistered name is an error, and the
    /// error names what *is* available so the model can correct itself.
    #[test]
    fn dispatch_rejects_unknown_tools_without_falling_back() {
        let error = registry()
            .dispatch("shell", &json!({}))
            .expect_err("unknown tool must be rejected");
        match &error {
            DispatchError::UnknownTool {
                requested,
                registered,
            } => {
                assert_eq!(requested, "shell");
                assert_eq!(registered, &vec!["probe_echo".to_string()]);
            }
        }
        let rendered = error.to_string();
        assert!(rendered.contains("unknown tool `shell`"), "{rendered}");
        assert!(rendered.contains("probe_echo"), "{rendered}");
    }

    #[test]
    fn an_empty_registry_lists_nothing_rather_than_claiming_success() {
        let registry = ToolRegistry::new();
        assert!(registry.is_empty());
        let error = registry
            .dispatch("anything", &json!({}))
            .expect_err("nothing is registered");
        assert!(error.to_string().contains("(none)"), "{error}");
    }

    #[test]
    fn tool_reports_its_own_argument_failure_as_unsuccessful() {
        let outcome = registry()
            .dispatch("probe_echo", &json!({}))
            .expect("routing succeeded");
        assert!(!outcome.success, "a bad argument must not look successful");
        assert_eq!(outcome.to_wire()["success"], json!(false));
    }

    #[test]
    fn register_reports_a_name_collision_instead_of_overwriting() {
        let mut registry = registry();
        let displaced = registry.register(Box::new(EchoTool));
        assert!(displaced.is_some(), "collision must be reported");
        assert_eq!(registry.names(), vec!["probe_echo".to_string()]);
    }
}
