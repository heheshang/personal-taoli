//! Session trace: what the host did, recorded so it can be replayed (PORT-01
//! round D).
//!
//! This is the host's own record of a session, not a copy of the runtime's.
//! The split matters, and was measured rather than assumed:
//!
//! * The runtime **also** persists a session, but only for non-ephemeral
//!   threads, as `~/.codex/sessions/…/rollout-<ts>-<thread_id>.jsonl`, and can
//!   serve it back over `thread/read` / `thread/items/list`. That is the
//!   *runtime's* view.
//! * Some facts in this module exist **only** on the host side and appear in no
//!   runtime record: which approval decisions were made and *why* (a verdict
//!   versus a deadline), and which server requests were refused as
//!   unimplemented.
//!
//! So the trace is not redundant: it is the only place the host's own
//! decisions are recoverable. It also keeps working for ephemeral threads,
//! which leave nothing on disk.
//!
//! The thread id is recorded in the first event, which is what makes the
//! runtime's own rollout findable for cross-checking — the two records share
//! that key and nothing else.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::approval::ApprovalRecord;
use super::protocol::ApprovalPolicy;

/// Which sandbox the runtime is asked to apply.
///
/// Typed rather than a bare string so a typo cannot silently select a different
/// policy: these three are the values the runtime accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SandboxMode {
    ReadOnly,
    WorkspaceWrite,
    DangerFullAccess,
}

impl SandboxMode {
    pub fn as_wire(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::WorkspaceWrite => "workspace-write",
            Self::DangerFullAccess => "danger-full-access",
        }
    }
}

/// How a turn ended, as recorded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum RecordedTurnStatus {
    Completed,
    /// The turn did not complete; `detail` says why, so replay does not have to
    /// guess whether a missing finish was a crash or a failure.
    Failed {
        detail: String,
    },
}

/// Identifies the domain instructions in force for a session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstructionsFingerprint {
    /// Number of bytes injected, after trimming.
    pub bytes: usize,
    /// FNV-1a hash of the injected text.
    ///
    /// Not a security primitive: it exists to answer "was this the same
    /// revision?" when comparing two traces, and to notice an accidental edit.
    pub hash: u64,
}

impl InstructionsFingerprint {
    pub fn of(text: &str) -> Self {
        // FNV-1a, computed inline so the trace has no dependency on a hashing
        // crate for a non-security fingerprint.
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in text.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        Self {
            bytes: text.len(),
            hash,
        }
    }
}

/// One recorded step.
///
/// Variants are host-side facts, not a transcript of runtime notifications:
/// reasoning deltas and other churn are deliberately absent, because a trace
/// that grows with noise is harder to attribute from, not easier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "event")]
pub enum TraceEvent {
    SessionStarted {
        thread_id: String,
        cwd: String,
        sandbox: SandboxMode,
        approval_policy: ApprovalPolicy,
        /// Who reviews escalations for this session.
        ///
        /// `default` so a trace written before this field existed still
        /// replays: those sessions routed reviews to the client, which is
        /// exactly what the default says.
        #[serde(default)]
        approvals_reviewer: crate::agent::access::ApprovalsReviewer,
        /// Skill roots registered for this session, in order.
        ///
        /// Recorded because the reachable skill set is a property of the
        /// session: two traces with different roots were not running the same
        /// agent, and a replay should show what was reachable.
        ///
        /// `default` so that a trace written before this field existed still
        /// replays: old events describe sessions that had no extra roots, which
        /// is exactly what the default says.
        #[serde(default)]
        skill_roots: Vec<String>,
        /// Whether domain instructions were injected.
        ///
        /// Recorded as a flag plus a fingerprint rather than the full text: the
        /// text is version-controlled, so a hash identifies which revision was
        /// in force without duplicating the document into every trace.
        developer_instructions: Option<InstructionsFingerprint>,
        at_ms: u64,
    },
    TurnStarted {
        turn_id: String,
        /// What the host sent. The input half of "inputs and outputs".
        prompt: String,
        at_ms: u64,
    },
    /// A tool the host dispatched, with the arguments it received and the exact
    /// payload it returned.
    ToolCall {
        call_id: String,
        tool: String,
        arguments: serde_json::Value,
        success: bool,
        output: String,
        at_ms: u64,
    },
    /// An approval the host answered, verbatim as recorded for audit.
    Approval {
        record: ApprovalRecord,
    },
    /// A server request the host refused because it is not implemented.
    RefusedRequest {
        method: String,
        at_ms: u64,
    },
    /// A completed item worth keeping for attribution (reasoning summary or
    /// agent message). Deltas and started/updated churn are not recorded.
    Item {
        turn_id: String,
        kind: String,
        text: String,
        at_ms: u64,
    },
    TurnFinished {
        turn_id: String,
        status: RecordedTurnStatus,
        final_message: Option<String>,
        at_ms: u64,
    },
    SessionFinished {
        at_ms: u64,
    },
}

/// Append-only JSONL writer for one session.
///
/// One line per event, flushed as it is written: a session that is killed
/// mid-turn must still leave the events up to that point, because the events
/// that matter most for attribution are the ones just before a crash.
pub struct TraceWriter {
    path: PathBuf,
}

impl TraceWriter {
    /// Opens (or creates) the trace for `thread_id` under `directory`.
    ///
    /// The path is `<directory>/<thread_id>.jsonl`, so replay is addressed by
    /// thread id — the same key the runtime uses for its own rollout.
    pub fn create(directory: impl AsRef<Path>, thread_id: &str) -> std::io::Result<Self> {
        let directory = directory.as_ref();
        std::fs::create_dir_all(directory)?;
        Ok(Self {
            path: directory.join(format!("{thread_id}.jsonl")),
        })
    }

    /// Opens a trace file directly, for replay-side helpers and tests.
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Appends one event and flushes it.
    pub fn record(&self, event: &TraceEvent) -> std::io::Result<()> {
        use std::io::Write;
        let mut line = serde_json::to_string(event)?;
        line.push('\n');
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        file.write_all(line.as_bytes())?;
        // Flush per event: durability across a crash is the whole point.
        file.flush()
    }

    /// Records and reports failure without aborting the session.
    ///
    /// A trace write failure is a real problem, but losing the trace must not
    /// also lose the turn: the error is surfaced on stderr and the session
    /// continues, having already recorded everything up to this point.
    pub fn try_record(&self, event: &TraceEvent) {
        if let Err(error) = self.record(event) {
            tracing::error!(
                target: "agent",
                path = %self.path.display(),
                %error,
                "failed to append trace event"
            );
        }
    }
}

/// One turn rebuilt from the trace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnTrace {
    pub turn_id: String,
    pub prompt: String,
    pub tool_calls: Vec<ToolCallTrace>,
    pub approvals: Vec<ApprovalRecord>,
    pub refused_requests: Vec<String>,
    pub items: Vec<(String, String)>,
    pub status: Option<RecordedTurnStatus>,
    pub final_message: Option<String>,
}

/// A tool call rebuilt from the trace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallTrace {
    pub call_id: String,
    pub tool: String,
    pub arguments: serde_json::Value,
    pub success: bool,
    pub output: String,
}

/// A whole session rebuilt from the trace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionTrace {
    pub thread_id: String,
    pub cwd: String,
    pub sandbox: SandboxMode,
    pub approval_policy: ApprovalPolicy,
    /// Who reviews escalations for this session.
    pub approvals_reviewer: crate::agent::access::ApprovalsReviewer,
    /// Skill roots registered for this session, in order.
    pub skill_roots: Vec<String>,
    /// Which revision of the domain instructions was in force, when any.
    pub instructions: Option<InstructionsFingerprint>,
    pub turns: Vec<TurnTrace>,
    pub finished: bool,
}

impl SessionTrace {
    /// Every tool call across the session, in order.
    pub fn tool_calls(&self) -> impl Iterator<Item = &ToolCallTrace> {
        self.turns.iter().flat_map(|turn| turn.tool_calls.iter())
    }

    /// Every approval across the session, in order.
    pub fn approvals(&self) -> impl Iterator<Item = &ApprovalRecord> {
        self.turns.iter().flat_map(|turn| turn.approvals.iter())
    }

    /// Tool call counts by tool name, for a quick attribution view.
    pub fn tool_call_counts(&self) -> BTreeMap<String, usize> {
        let mut counts = BTreeMap::new();
        for call in self.tool_calls() {
            *counts.entry(call.tool.clone()).or_insert(0) += 1;
        }
        counts
    }
}

/// What a recorded session started with, accumulated during replay.
///
/// Aliased rather than repeated inline: it appears in both the replayer and the
/// helper that guards ordering, and a tuple this wide is easy to transpose.
type SessionStart = (
    String,
    String,
    SandboxMode,
    ApprovalPolicy,
    crate::agent::access::ApprovalsReviewer,
    Vec<String>,
    Option<InstructionsFingerprint>,
);

/// Why a trace could not be replayed.
///
/// Replay is used as evidence, so every one of these is a hard failure: a trace
/// that cannot be understood is not a trace that can be trusted.
#[derive(Debug)]
pub enum TraceError {
    /// The file could not be read.
    Io { source: std::io::Error },
    /// A line was not a valid event.
    Malformed {
        line: usize,
        source: serde_json::Error,
    },
    /// The events were structurally impossible (see [`TraceError::Unordered`]).
    Unordered { line: usize, detail: String },
    /// The trace contained no `SessionStarted`.
    Empty,
}

impl std::fmt::Display for TraceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { source } => write!(formatter, "cannot read trace: {source}"),
            Self::Malformed { line, source } => {
                write!(formatter, "trace line {line} is malformed: {source}")
            }
            Self::Unordered { line, detail } => {
                write!(formatter, "trace line {line} is out of order: {detail}")
            }
            Self::Empty => write!(formatter, "trace contains no session_started event"),
        }
    }
}

impl std::error::Error for TraceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source } => Some(source),
            Self::Malformed { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Replays a trace file into the chain it recorded.
///
/// Ordering is validated while rebuilding, so a structurally impossible file
/// (an event before its turn, a second `SessionStarted`, anything after
/// `SessionFinished`) is an error rather than a silently reordered story.
pub fn replay(path: impl AsRef<Path>) -> Result<SessionTrace, TraceError> {
    let contents = std::fs::read_to_string(path).map_err(|source| TraceError::Io { source })?;
    replay_str(&contents)
}

/// Replays trace text. Split out so tests can build traces inline.
pub fn replay_str(contents: &str) -> Result<SessionTrace, TraceError> {
    let mut started: Option<SessionStart> = None;
    let mut finished = false;
    let mut last_at_ms = 0;

    // The turn being rebuilt. Held here rather than appended immediately so
    // that "an event after its own turn finished" is detectable: with the turn
    // already in the list there is nothing to distinguish a late event from one
    // belonging to the current turn.
    let mut current: Option<TurnTrace> = None;
    let mut turns: Vec<TurnTrace> = Vec::new();

    for (index, line) in contents.lines().enumerate() {
        let line_number = index + 1;
        if line.trim().is_empty() {
            continue;
        }
        let event: TraceEvent =
            serde_json::from_str(line).map_err(|source| TraceError::Malformed {
                line: line_number,
                source,
            })?;

        if finished {
            return Err(TraceError::Unordered {
                line: line_number,
                detail: "an event follows session_finished".to_string(),
            });
        }

        match event {
            TraceEvent::SessionStarted {
                thread_id,
                cwd,
                sandbox,
                approval_policy,
                approvals_reviewer,
                skill_roots,
                developer_instructions,
                at_ms,
            } => {
                if started.is_some() {
                    return Err(TraceError::Unordered {
                        line: line_number,
                        detail: "a second session_started".to_string(),
                    });
                }
                last_at_ms = at_ms;
                started = Some((
                    thread_id,
                    cwd,
                    sandbox,
                    approval_policy,
                    approvals_reviewer,
                    skill_roots,
                    developer_instructions,
                ));
            }
            TraceEvent::TurnStarted {
                turn_id,
                prompt,
                at_ms,
            } => {
                require_started(&started, line_number)?;
                require_monotonic(&mut last_at_ms, at_ms, line_number)?;
                if current.is_some() {
                    return Err(TraceError::Unordered {
                        line: line_number,
                        detail: "a turn started before the previous one finished".to_string(),
                    });
                }
                current = Some(TurnTrace {
                    turn_id,
                    prompt,
                    tool_calls: Vec::new(),
                    approvals: Vec::new(),
                    refused_requests: Vec::new(),
                    items: Vec::new(),
                    status: None,
                    final_message: None,
                });
            }
            TraceEvent::ToolCall {
                call_id,
                tool,
                arguments,
                success,
                output,
                at_ms,
            } => {
                require_monotonic(&mut last_at_ms, at_ms, line_number)?;
                open_turn(&mut current, line_number)?
                    .tool_calls
                    .push(ToolCallTrace {
                        call_id,
                        tool,
                        arguments,
                        success,
                        output,
                    });
            }
            TraceEvent::Approval { record } => {
                require_monotonic(&mut last_at_ms, record.decided_at_ms, line_number)?;
                open_turn(&mut current, line_number)?.approvals.push(record);
            }
            TraceEvent::RefusedRequest { method, at_ms } => {
                require_monotonic(&mut last_at_ms, at_ms, line_number)?;
                open_turn(&mut current, line_number)?
                    .refused_requests
                    .push(method);
            }
            TraceEvent::Item {
                turn_id,
                kind,
                text,
                at_ms,
            } => {
                require_monotonic(&mut last_at_ms, at_ms, line_number)?;
                let turn = open_turn(&mut current, line_number)?;
                if turn.turn_id != turn_id {
                    return Err(TraceError::Unordered {
                        line: line_number,
                        detail: format!("item for turn {turn_id} inside turn {}", turn.turn_id),
                    });
                }
                turn.items.push((kind, text));
            }
            TraceEvent::TurnFinished {
                turn_id,
                status,
                final_message,
                at_ms,
            } => {
                require_monotonic(&mut last_at_ms, at_ms, line_number)?;
                let Some(mut turn) = current.take() else {
                    return Err(TraceError::Unordered {
                        line: line_number,
                        detail: "turn_finished without a turn_started".to_string(),
                    });
                };
                if turn.turn_id != turn_id {
                    return Err(TraceError::Unordered {
                        line: line_number,
                        detail: format!("turn_finished for {turn_id} inside turn {}", turn.turn_id),
                    });
                }
                turn.status = Some(status);
                turn.final_message = final_message;
                turns.push(turn);
            }
            TraceEvent::SessionFinished { at_ms } => {
                require_monotonic(&mut last_at_ms, at_ms, line_number)?;
                finished = true;
            }
        }
    }

    // A turn with no turn_finished is a turn that did not end: keep it, so the
    // replay shows what was in flight instead of hiding it.
    if let Some(turn) = current.take() {
        turns.push(turn);
    }

    let Some((
        thread_id,
        cwd,
        sandbox,
        approval_policy,
        approvals_reviewer,
        skill_roots,
        instructions,
    )) = started
    else {
        return Err(TraceError::Empty);
    };

    Ok(SessionTrace {
        thread_id,
        cwd,
        sandbox,
        approval_policy,
        approvals_reviewer,
        skill_roots,
        instructions,
        turns,
        finished,
    })
}

fn require_started(started: &Option<SessionStart>, line: usize) -> Result<(), TraceError> {
    if started.is_none() {
        return Err(TraceError::Unordered {
            line,
            detail: "a turn appears before session_started".to_string(),
        });
    }
    Ok(())
}

/// Timestamps must not go backwards: doing so means the file is not a single
/// append-only session, so it must not be presented as one.
fn require_monotonic(last: &mut u64, at_ms: u64, line: usize) -> Result<(), TraceError> {
    if at_ms < *last {
        return Err(TraceError::Unordered {
            line,
            detail: format!("timestamp {at_ms} precedes {last}"),
        });
    }
    *last = at_ms;
    Ok(())
}

fn open_turn(current: &mut Option<TurnTrace>, line: usize) -> Result<&mut TurnTrace, TraceError> {
    match current {
        Some(turn) => Ok(turn),
        None => Err(TraceError::Unordered {
            line,
            detail: "an event appears before any turn_started, or after its turn_finished"
                .to_string(),
        }),
    }
}

/// Parameters for starting a thread.
///
/// Bundled because round D adds a sixth concern; positional parameters at that
/// width invite transposition, and these two booleans make it easy.
#[derive(Debug, Clone)]
pub struct ThreadOptions {
    pub cwd: String,
    /// `true` leaves nothing on disk on the runtime's side. The host trace is
    /// written either way.
    pub ephemeral: bool,
    pub sandbox: SandboxMode,
    /// Must be [`ApprovalPolicy::UnlessTrusted`] for every side-effecting action
    /// to reach the owner; see `protocol::ApprovalPolicy`.
    pub approval_policy: ApprovalPolicy,
    /// Who reviews what the policy escalates.
    ///
    /// `User` routes requests here; `AutoReview` has the runtime review them
    /// (the app's “帮我批准”). Part of the session's contract, so it is recorded.
    pub approvals_reviewer: crate::agent::access::ApprovalsReviewer,
    /// Domain constraints injected as the runtime's `developerInstructions`.
    ///
    /// Carried verbatim from the repository document; see
    /// [`super::instructions`]. `None` is for sessions that intentionally run
    /// without them (tests), not a default a production caller should pick.
    pub developer_instructions: Option<String>,
    /// Extra skill directories, registered with the runtime as
    /// `skills/extraRoots/set`.
    ///
    /// Must be absolute: the runtime rejects relative roots for the same reason
    /// the sandbox does. Empty keeps the previous behaviour — the runtime's own
    /// skills only — which means a skill repository checked out elsewhere is
    /// invisible to the model unless it is named here.
    pub skill_roots: Vec<PathBuf>,
    /// Directory for the host trace. `None` disables tracing.
    pub trace_dir: Option<PathBuf>,
}

impl Default for ThreadOptions {
    fn default() -> Self {
        Self {
            cwd: ".".to_string(),
            ephemeral: true,
            sandbox: SandboxMode::ReadOnly,
            approval_policy: ApprovalPolicy::UnlessTrusted,
            approvals_reviewer: crate::agent::access::ApprovalsReviewer::User,
            developer_instructions: None,
            skill_roots: Vec::new(),
            trace_dir: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::approval::{ApprovalKind, Decision, DecisionSource};
    use serde_json::json;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "taoli-trace-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch");
        dir
    }

    fn approval_record(request_id: i64, at_ms: u64) -> ApprovalRecord {
        ApprovalRecord {
            request_id,
            method: "item/commandExecution/requestApproval".to_string(),
            kind: ApprovalKind::CommandExecution,
            summary: "echo hi".to_string(),
            decision: Decision::Deny,
            source: DecisionSource::Decider,
            decided_at_ms: at_ms,
            waited_ms: 3,
            advertised: vec!["accept".to_string()],
        }
    }

    /// A complete, well-formed trace used by several tests.
    fn sample_events() -> Vec<TraceEvent> {
        vec![
            TraceEvent::SessionStarted {
                thread_id: "t-1".to_string(),
                cwd: "/tmp".to_string(),
                sandbox: SandboxMode::ReadOnly,
                approval_policy: ApprovalPolicy::UnlessTrusted,
                approvals_reviewer: crate::agent::access::ApprovalsReviewer::User,
                skill_roots: Vec::new(),
                developer_instructions: None,
                at_ms: 100,
            },
            TraceEvent::TurnStarted {
                turn_id: "turn-1".to_string(),
                prompt: "do the thing".to_string(),
                at_ms: 110,
            },
            TraceEvent::Item {
                turn_id: "turn-1".to_string(),
                kind: "reasoning".to_string(),
                text: "thinking".to_string(),
                at_ms: 120,
            },
            TraceEvent::ToolCall {
                call_id: "call-1".to_string(),
                tool: "taoli_test_echo".to_string(),
                arguments: json!({}),
                success: true,
                output: "{\"records\":284}".to_string(),
                at_ms: 130,
            },
            TraceEvent::Approval {
                record: approval_record(7, 140),
            },
            TraceEvent::RefusedRequest {
                method: "mcpServer/elicitation/request".to_string(),
                at_ms: 150,
            },
            TraceEvent::TurnFinished {
                turn_id: "turn-1".to_string(),
                status: RecordedTurnStatus::Completed,
                final_message: Some("done".to_string()),
                at_ms: 160,
            },
            TraceEvent::SessionFinished { at_ms: 170 },
        ]
    }

    fn write_events(path: &Path, events: &[TraceEvent]) {
        let writer = TraceWriter::at(path);
        for event in events {
            writer.record(event).expect("record");
        }
    }

    #[test]
    fn a_recorded_session_replays_into_the_chain_it_recorded() {
        let dir = scratch("round-trip");
        let path = dir.join("t-1.jsonl");
        write_events(&path, &sample_events());

        let trace = replay(&path).expect("replays");
        assert_eq!(trace.thread_id, "t-1");
        assert_eq!(trace.cwd, "/tmp");
        assert_eq!(trace.sandbox, SandboxMode::ReadOnly);
        assert_eq!(trace.approval_policy, ApprovalPolicy::UnlessTrusted);
        assert!(trace.finished);
        assert_eq!(trace.turns.len(), 1, "one turn, not one per event");

        let turn = &trace.turns[0];
        assert_eq!(turn.turn_id, "turn-1");
        assert_eq!(turn.prompt, "do the thing");
        assert_eq!(turn.tool_calls.len(), 1);
        assert_eq!(turn.tool_calls[0].tool, "taoli_test_echo");
        assert_eq!(turn.tool_calls[0].output, "{\"records\":284}");
        assert_eq!(turn.approvals.len(), 1);
        assert_eq!(turn.approvals[0].request_id, 7);
        assert_eq!(turn.refused_requests, vec!["mcpServer/elicitation/request"]);
        assert_eq!(
            turn.items,
            vec![("reasoning".to_string(), "thinking".to_string())]
        );
        assert_eq!(turn.status, Some(RecordedTurnStatus::Completed));
        assert_eq!(turn.final_message.as_deref(), Some("done"));

        assert_eq!(trace.tool_call_counts().get("taoli_test_echo"), Some(&1));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The determinism half of the acceptance criterion: replaying the same
    /// bytes twice must give the same chain.
    #[test]
    fn replaying_the_same_trace_twice_is_identical() {
        let dir = scratch("determinism");
        let path = dir.join("t-1.jsonl");
        write_events(&path, &sample_events());

        let first = replay(&path).expect("first replay");
        let second = replay(&path).expect("second replay");
        assert_eq!(first, second);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn multiple_turns_stay_separate_and_in_order() {
        let dir = scratch("multi-turn");
        let path = dir.join("t.jsonl");
        let mut events = vec![TraceEvent::SessionStarted {
            thread_id: "t".to_string(),
            cwd: ".".to_string(),
            sandbox: SandboxMode::WorkspaceWrite,
            approval_policy: ApprovalPolicy::OnRequest,
            approvals_reviewer: crate::agent::access::ApprovalsReviewer::User,
            skill_roots: Vec::new(),
            developer_instructions: None,
            at_ms: 1,
        }];
        for (index, turn_id) in ["turn-a", "turn-b"].iter().enumerate() {
            let base = (index as u64 + 1) * 100;
            events.push(TraceEvent::TurnStarted {
                turn_id: (*turn_id).to_string(),
                prompt: format!("p-{turn_id}"),
                at_ms: base,
            });
            events.push(TraceEvent::ToolCall {
                call_id: format!("c-{turn_id}"),
                tool: "taoli_test_echo".to_string(),
                arguments: json!({}),
                success: true,
                output: "ok".to_string(),
                at_ms: base + 1,
            });
            events.push(TraceEvent::TurnFinished {
                turn_id: (*turn_id).to_string(),
                status: RecordedTurnStatus::Completed,
                final_message: Some("done".to_string()),
                at_ms: base + 2,
            });
        }
        write_events(&path, &events);

        let trace = replay(&path).expect("replays");
        assert_eq!(trace.turns.len(), 2);
        assert_eq!(trace.turns[0].turn_id, "turn-a");
        assert_eq!(trace.turns[1].turn_id, "turn-b");
        // A tool call must not leak into the wrong turn.
        assert_eq!(trace.turns[0].tool_calls[0].call_id, "c-turn-a");
        assert_eq!(trace.turns[1].tool_calls[0].call_id, "c-turn-b");
        assert_eq!(trace.tool_call_counts().get("taoli_test_echo"), Some(&2));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unfinished_session_replays_as_unfinished_rather_than_as_success() {
        let dir = scratch("unfinished");
        let path = dir.join("t.jsonl");
        let mut events = sample_events();
        // Drop session_finished and the turn's own finish: a crash mid-turn.
        events.truncate(5);
        write_events(&path, &events);

        let trace = replay(&path).expect("replays");
        assert!(!trace.finished);
        assert_eq!(trace.turns.len(), 1);
        assert_eq!(trace.turns[0].status, None, "no finish was recorded");
        assert_eq!(trace.turns[0].final_message, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_turn_records_why() {
        let dir = scratch("failed");
        let path = dir.join("t.jsonl");
        write_events(
            &path,
            &[
                TraceEvent::SessionStarted {
                    thread_id: "t".to_string(),
                    cwd: ".".to_string(),
                    sandbox: SandboxMode::ReadOnly,
                    approval_policy: ApprovalPolicy::UnlessTrusted,
                    approvals_reviewer: crate::agent::access::ApprovalsReviewer::User,
                    skill_roots: Vec::new(),
                    developer_instructions: None,
                    at_ms: 1,
                },
                TraceEvent::TurnStarted {
                    turn_id: "turn-x".to_string(),
                    prompt: "p".to_string(),
                    at_ms: 2,
                },
                TraceEvent::TurnFinished {
                    turn_id: "turn-x".to_string(),
                    status: RecordedTurnStatus::Failed {
                        detail: "turn timed out".to_string(),
                    },
                    final_message: None,
                    at_ms: 3,
                },
            ],
        );
        let trace = replay(&path).expect("replays");
        assert_eq!(
            trace.turns[0].status,
            Some(RecordedTurnStatus::Failed {
                detail: "turn timed out".to_string()
            })
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_empty_trace_is_an_error_not_an_empty_session() {
        // An empty chain and "nothing was recorded" must not look the same.
        assert!(matches!(replay_str("").unwrap_err(), TraceError::Empty));
        assert!(matches!(replay_str("\n\n").unwrap_err(), TraceError::Empty));
    }

    #[test]
    fn a_malformed_line_names_its_line_number() {
        let text = concat!(
            r#"{"event":"session_started","thread_id":"t","cwd":".","sandbox":"read-only","approval_policy":"never","at_ms":1}"#,
            "\n",
            "{\"event\":\"not_a_real_event\"}\n"
        );
        match replay_str(text).unwrap_err() {
            TraceError::Malformed { line, .. } => assert_eq!(line, 2),
            other => panic!("expected Malformed, got {other:?}"),
        }
    }

    #[test]
    fn events_before_the_session_starts_are_rejected() {
        let text = concat!(
            r#"{"event":"turn_started","turn_id":"t1","prompt":"p","at_ms":5}"#,
            "\n"
        );
        match replay_str(text).unwrap_err() {
            TraceError::Unordered { line, detail } => {
                assert_eq!(line, 1);
                assert!(detail.contains("before session_started"), "{detail}");
            }
            other => panic!("expected Unordered, got {other:?}"),
        }
    }

    #[test]
    fn a_second_session_started_is_rejected() {
        let text = concat!(
            r#"{"event":"session_started","thread_id":"t","cwd":".","sandbox":"read-only","approval_policy":"never","at_ms":1}"#,
            "\n",
            r#"{"event":"session_started","thread_id":"u","cwd":".","sandbox":"read-only","approval_policy":"never","at_ms":2}"#,
            "\n"
        );
        match replay_str(text).unwrap_err() {
            TraceError::Unordered { line, detail } => {
                assert_eq!(line, 2);
                assert!(detail.contains("second session_started"), "{detail}");
            }
            other => panic!("expected Unordered, got {other:?}"),
        }
    }

    #[test]
    fn events_after_session_finished_are_rejected() {
        let mut events = sample_events();
        events.push(TraceEvent::TurnStarted {
            turn_id: "late".to_string(),
            prompt: "p".to_string(),
            at_ms: 200,
        });
        let dir = scratch("after-finish");
        let path = dir.join("t.jsonl");
        write_events(&path, &events);

        match replay(&path).unwrap_err() {
            TraceError::Unordered { detail, .. } => {
                assert!(detail.contains("follows session_finished"), "{detail}");
            }
            other => panic!("expected Unordered, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_tool_call_after_its_turn_finished_is_rejected() {
        // Otherwise it would be silently attributed to the *next* turn.
        let dir = scratch("orphan-call");
        let path = dir.join("t.jsonl");
        write_events(
            &path,
            &[
                TraceEvent::SessionStarted {
                    thread_id: "t".to_string(),
                    cwd: ".".to_string(),
                    sandbox: SandboxMode::ReadOnly,
                    approval_policy: ApprovalPolicy::UnlessTrusted,
                    approvals_reviewer: crate::agent::access::ApprovalsReviewer::User,
                    skill_roots: Vec::new(),
                    developer_instructions: None,
                    at_ms: 1,
                },
                TraceEvent::TurnStarted {
                    turn_id: "turn-a".to_string(),
                    prompt: "p".to_string(),
                    at_ms: 2,
                },
                TraceEvent::TurnFinished {
                    turn_id: "turn-a".to_string(),
                    status: RecordedTurnStatus::Completed,
                    final_message: None,
                    at_ms: 3,
                },
                TraceEvent::ToolCall {
                    call_id: "c".to_string(),
                    tool: "taoli_test_echo".to_string(),
                    arguments: json!({}),
                    success: true,
                    output: "ok".to_string(),
                    at_ms: 4,
                },
            ],
        );
        match replay(&path).unwrap_err() {
            TraceError::Unordered { line, detail } => {
                assert_eq!(line, 4);
                // The message must name both possibilities, since one state
                // (no turn open) covers a missing turn_started and a completed
                // turn alike.
                assert!(detail.contains("after its turn_finished"), "{detail}");
                assert!(detail.contains("before any turn_started"), "{detail}");
            }
            other => panic!("expected Unordered, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_non_monotonic_timestamp_is_rejected() {
        let dir = scratch("time-travel");
        let path = dir.join("t.jsonl");
        write_events(
            &path,
            &[
                TraceEvent::SessionStarted {
                    thread_id: "t".to_string(),
                    cwd: ".".to_string(),
                    sandbox: SandboxMode::ReadOnly,
                    approval_policy: ApprovalPolicy::UnlessTrusted,
                    approvals_reviewer: crate::agent::access::ApprovalsReviewer::User,
                    skill_roots: Vec::new(),
                    developer_instructions: None,
                    at_ms: 100,
                },
                TraceEvent::TurnStarted {
                    turn_id: "turn-a".to_string(),
                    prompt: "p".to_string(),
                    at_ms: 50,
                },
            ],
        );
        match replay(&path).unwrap_err() {
            TraceError::Unordered { line, detail } => {
                assert_eq!(line, 2);
                assert!(detail.contains("precedes"), "{detail}");
            }
            other => panic!("expected Unordered, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_trace_path_is_addressed_by_thread_id() {
        let dir = scratch("path");
        let writer = TraceWriter::create(&dir, "01a08bb4-3abe").expect("create");
        assert_eq!(writer.path(), dir.join("01a08bb4-3abe.jsonl"));
        // The directory is created on demand.
        assert!(dir.is_dir());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_trace_file_is_an_error_not_an_empty_session() {
        let error = replay("/nonexistent/does-not-exist.jsonl").unwrap_err();
        assert!(matches!(error, TraceError::Io { .. }), "{error:?}");
    }

    #[test]
    fn sandbox_modes_map_to_the_values_the_runtime_accepts() {
        assert_eq!(SandboxMode::ReadOnly.as_wire(), "read-only");
        assert_eq!(SandboxMode::WorkspaceWrite.as_wire(), "workspace-write");
        assert_eq!(
            SandboxMode::DangerFullAccess.as_wire(),
            "danger-full-access"
        );
    }

    #[test]
    fn thread_options_default_to_the_narrow_choices() {
        let options = ThreadOptions::default();
        assert!(options.ephemeral);
        assert_eq!(options.sandbox, SandboxMode::ReadOnly);
        // The safe default is the one that actually raises approvals.
        assert_eq!(options.approval_policy, ApprovalPolicy::UnlessTrusted);
        assert!(options.trace_dir.is_none(), "tracing is opt-in");
    }
}
