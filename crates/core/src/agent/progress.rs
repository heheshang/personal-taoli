//! Normalised progress from a running turn (PORT-01-M).
//!
//! The runtime speaks in 80-odd notification methods, several of which are
//! deltas keyed by item id. This module turns the handful that matter into one
//! small vocabulary, so the host and the interface do not each re-derive it
//! from raw JSON.
//!
//! Deltas are forwarded as **chunks**, not cumulative text: the runtime sends
//! `{delta: "I"}`, then `{delta: " need"}`, and the consumer accumulates by
//! `item_id`. Forwarding chunks keeps this layer stateless — it does not have to
//! hold a transcript, and a consumer that only wants the tail can keep just the
//! tail.
//!
//! What is *not* here, deliberately: skill attribution. The protocol has no
//! "skill invoked" signal — a skill manifests as ordinary tool and shell calls
//! (reading its `SKILL.md`, running its scripts). The interface shows the
//! available skill list and the call stream side by side rather than inventing
//! an attribution the data does not support.

use std::sync::Arc;

/// What an item is, in terms a reader recognises.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    /// The model's own reasoning summary.
    Reasoning,
    /// A shell command the agent ran.
    Command,
    /// Files the agent changed.
    FileChange,
    /// An MCP or function tool call.
    ToolCall,
    /// The agent's user-visible message.
    Message,
    /// Anything else the runtime reports.
    Other,
}

impl ItemKind {
    /// Classifies an item by the runtime's own `type` tag.
    pub fn from_wire(kind: &str) -> Self {
        match kind {
            "reasoning" => Self::Reasoning,
            "commandExecution" | "localShellCall" => Self::Command,
            "fileChange" | "patchApply" => Self::FileChange,
            "mcpToolCall"
            | "dynamicToolCall"
            | "functionCall"
            | "customToolCall"
            | "toolSearchCall"
            | "webSearchCall"
            | "imageGenerationCall" => Self::ToolCall,
            "agentMessage" => Self::Message,
            _ => Self::Other,
        }
    }
}

/// How an item ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemState {
    Running,
    Completed,
    /// Ran and failed.
    Failed,
    /// Never ran: the owner refused it.
    Declined,
}

impl ItemState {
    pub fn from_wire(state: &str) -> Self {
        match state {
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "declined" => Self::Declined,
            _ => Self::Running,
        }
    }

    pub fn is_running(self) -> bool {
        self == Self::Running
    }
}

/// Coarse phase of a turn, for a status line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// The model is thinking or has not yet acted.
    Thinking,
    /// At least one item is still running.
    Working,
    /// The turn has ended; the caller should stop showing live state.
    Done,
}

/// One step in a turn, as the interface needs to show it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgressItem {
    pub item_id: String,
    pub kind: ItemKind,
    /// One-line title: the command line, or the tool name.
    pub title: String,
    /// Extra context the runtime supplied (working directory, for a command).
    pub detail: Option<String>,
    pub state: ItemState,
    /// Output accumulated so far, for a command or tool call.
    pub output: String,
    pub exit_code: Option<i64>,
    pub duration_ms: Option<u64>,
}

/// Token accounting for the turn, mirrored from the runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_output_tokens: u64,
    pub total_tokens: u64,
    /// The model's context window, when the runtime reports it.
    pub context_window: Option<u64>,
}

/// What a running turn reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnProgress {
    /// The turn began.
    Started { turn_id: String },
    /// Streaming reasoning summary text (chunk, keyed by item).
    ReasoningDelta { item_id: String, text: String },
    /// Streaming agent message text (chunk, keyed by item).
    MessageDelta { item_id: String, text: String },
    /// An item appeared.
    ItemStarted(ProgressItem),
    /// Streaming output for an item that produces output (a command).
    /// The output is a chunk; the caller appends it to the item's output.
    ItemOutput { item_id: String, text: String },
    /// An item finished, with whatever the runtime reported at the end.
    ItemFinished {
        item_id: String,
        state: ItemState,
        /// Final output, when the runtime sends it whole rather than as deltas.
        output: Option<String>,
        exit_code: Option<i64>,
        duration_ms: Option<u64>,
    },
    /// The runtime's token accounting changed.
    Tokens(TokenUsage),
    /// The owner (or the runtime's reviewer) is being asked about an action.
    ApprovalRequested { request_id: i64, summary: String },
    /// An approval was answered — by this client or by the runtime's reviewer.
    ApprovalResolved { request_id: i64 },
    /// The turn reached a new coarse stage.
    Stage(Stage),
}

/// Receives [`TurnProgress`] as a turn runs.
///
/// Called from the session task while the turn is in flight, so implementations
/// must not block: the interface's handler takes a mutex and returns.
pub type ProgressSink = Arc<dyn Fn(TurnProgress) + Send + Sync>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_kinds_cover_the_types_the_runtime_actually_sends() {
        // Measured during a real turn; these are the tags that appeared.
        assert_eq!(ItemKind::from_wire("reasoning"), ItemKind::Reasoning);
        assert_eq!(ItemKind::from_wire("commandExecution"), ItemKind::Command);
        assert_eq!(ItemKind::from_wire("agentMessage"), ItemKind::Message);
        assert_eq!(ItemKind::from_wire("userMessage"), ItemKind::Other);
        // Tools the runtime can report but this probe did not exercise.
        assert_eq!(ItemKind::from_wire("mcpToolCall"), ItemKind::ToolCall);
        assert_eq!(ItemKind::from_wire("fileChange"), ItemKind::FileChange);
        // An unknown tag must not be mistaken for something it is not.
        assert_eq!(ItemKind::from_wire("somethingNew"), ItemKind::Other);
    }

    #[test]
    fn item_states_are_read_from_the_runtimes_own_words() {
        assert_eq!(ItemState::from_wire("inProgress"), ItemState::Running);
        assert_eq!(ItemState::from_wire("completed"), ItemState::Completed);
        assert_eq!(ItemState::from_wire("failed"), ItemState::Failed);
        assert_eq!(ItemState::from_wire("declined"), ItemState::Declined);
        assert!(ItemState::Running.is_running());
        assert!(!ItemState::Completed.is_running());
    }
}
