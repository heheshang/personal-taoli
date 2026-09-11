//! codex `app-server` integration (PORT-01).
//!
//! Scope is deliberately narrow: it lets the host drive a codex agent runtime
//! **for read-only analysis**, over the `app-server` JSON-RPC protocol. It
//! contains no codex source, does not touch the trading path, and is not wired
//! into the observer loop.
//!
//! Implemented rounds:
//!
//! * **A** — connect, `initialize`, shut down cleanly with no orphaned
//!   processes ([`AgentSession::connect`] / [`AgentSession::shutdown`]).
//! * **B** — start a thread exposing host tools, run a turn, and answer the
//!   runtime's tool calls ([`AgentSession::start_thread`] /
//!   [`AgentSession::run_turn`]).
//!
//! Not implemented yet, and deliberately absent rather than stubbed: approval
//! flows (round C) and trace persistence (round D). A server request outside
//! the implemented surface is **refused with a JSON-RPC error**, which the
//! runtime turns into a failed operation — the fail-closed outcome.
//!
//! Boundary invariants (see `docs/PORT-01-codex接入-需求.md` R03): the child
//! starts with a cleared environment plus an allowlist, and nothing in this
//! module reaches `order` / `execution` / `account` writes.

pub mod access;
pub mod app_server;
pub mod approval;
pub mod instructions;
pub mod progress;
pub mod protocol;
pub mod report;
pub mod sandbox;
pub mod tools;
pub mod trace;

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use app_server::{AppServerConfig, CodexAppServer, ServerMessage, SkillInfo};
use approval::{
    ApprovalDecider, ApprovalKind, ApprovalLog, ApprovalRecord, ApprovalRequest, Decision,
    DecisionSource, advertised_decisions, response_payload, summarise,
};
use progress::{ItemKind, ItemState, ProgressItem, ProgressSink, Stage, TokenUsage, TurnProgress};
use tools::{ToolOutcome, ToolRegistry};
use trace::{RecordedTurnStatus, ThreadOptions, TraceEvent, TraceWriter};

/// Failures from driving the agent runtime.
///
/// Every variant is a real failure condition observed at a specific boundary;
/// none of them is recoverable by retrying blindly, so the caller decides.
#[derive(Debug)]
pub enum AgentError {
    /// The child process could not be started.
    Spawn {
        program: std::path::PathBuf,
        source: std::io::Error,
    },
    /// The child did not expose a pipe this client requires.
    MissingPipe(&'static str),
    /// A message could not be encoded into the wire format.
    Encode { source: serde_json::Error },
    /// Writing to the child failed.
    Transport { source: std::io::Error },
    /// Waiting for the child failed.
    Wait { source: std::io::Error },
    /// The child answered with a JSON-RPC error.
    Remote { id: i64, error: Value },
    /// The child answered a request with a payload we could not interpret.
    UnexpectedResponse { method: String, detail: String },
    /// No response arrived within the configured deadline.
    Timeout { method: String, timeout: Duration },
    /// The child exited (or its pipes closed) before the operation finished.
    Closed { detail: String },
    /// The turn stopped making progress, or ran past its ceiling.
    ///
    /// The two cases are reported apart because they mean different things: idle
    /// means the runtime went quiet, which points at a stuck process or a hung
    /// call; `MaxDuration` means it was still working but the host's budget ran
    /// out, which points at the budget.
    TurnTimeout {
        turn_id: String,
        reason: TurnTimeoutReason,
        timeout: Duration,
    },
    /// The session trace could not be opened.
    TraceFile {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    /// The agent runtime could not be confined, so it was not started.
    ///
    /// Refusing here is the point: starting it unconfined would silently drop
    /// the confinement the caller asked for.
    Confinement { source: sandbox::ConfineError },
    /// The model asked for more tool calls than the host permits in one turn.
    TooManyToolCalls { limit: usize },
    /// The runtime reported the turn as failed.
    TurnFailed { turn_id: String, detail: String },
}

impl std::fmt::Display for AgentError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn { program, source } => write!(
                formatter,
                "failed to start `{}`: {source}. Install codex and ensure it is on PATH.",
                program.display()
            ),
            Self::MissingPipe(pipe) => write!(formatter, "agent child exposed no {pipe} pipe"),
            Self::Encode { source } => write!(formatter, "failed to encode request: {source}"),
            Self::Transport { source } => write!(formatter, "agent transport failed: {source}"),
            Self::Wait { source } => write!(formatter, "failed to wait for agent child: {source}"),
            Self::Remote { id, error } => {
                let code = error
                    .get("code")
                    .and_then(Value::as_i64)
                    .unwrap_or_default();
                let message = error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("<no message>");
                write!(formatter, "agent rejected request {id}: [{code}] {message}")
            }
            Self::UnexpectedResponse { method, detail } => {
                write!(formatter, "unexpected `{method}` response: {detail}")
            }
            Self::Timeout { method, timeout } => write!(
                formatter,
                "agent did not answer `{method}` within {}s",
                timeout.as_secs()
            ),
            Self::Closed { detail } => write!(formatter, "agent connection closed: {detail}"),
            Self::TurnTimeout {
                turn_id,
                reason,
                timeout,
            } => match reason {
                TurnTimeoutReason::Idle => write!(
                    formatter,
                    "turn {turn_id} produced no progress for {}s and was interrupted",
                    timeout.as_secs()
                ),
                TurnTimeoutReason::MaxDuration => write!(
                    formatter,
                    "turn {turn_id} exceeded the {}s ceiling and was interrupted",
                    timeout.as_secs()
                ),
            },
            Self::TraceFile { path, source } => write!(
                formatter,
                "cannot open session trace {}: {source}",
                path.display()
            ),
            Self::Confinement { source } => {
                write!(formatter, "cannot confine agent runtime: {source}")
            }
            Self::TooManyToolCalls { limit } => {
                write!(
                    formatter,
                    "turn exceeded the host limit of {limit} tool calls"
                )
            }
            Self::TurnFailed { turn_id, detail } => {
                write!(formatter, "turn {turn_id} failed: {detail}")
            }
        }
    }
}

impl std::error::Error for AgentError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Spawn { source, .. } | Self::Transport { source } | Self::Wait { source } => {
                Some(source)
            }
            Self::Encode { source } => Some(source),
            _ => None,
        }
    }
}

/// Which limit ended a turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnTimeoutReason {
    /// No event arrived for `idle_timeout`.
    Idle,
    /// The turn ran for `max_duration`.
    MaxDuration,
}

/// Limits the host places on one turn.
///
/// All three are host policy: a runaway model must not be able to spend
/// unbounded time or money, and none of the bounds can be widened by anything
/// the model says.
#[derive(Debug, Clone)]
pub struct TurnLimits {
    /// How long the turn may go **without any event** before it is treated as
    /// stuck.
    ///
    /// This is the limit that matters for real work. A wall-clock budget is the
    /// wrong instrument for a deep-analysis skill: measured on a real run, the
    /// UZI skill was still streaming progress past five minutes (network
    /// pre-flight, three fetch waves, rule scoring), and a total cap killed it
    /// mid-flight while it was working perfectly. Activity is the signal —
    /// a turn that keeps reporting is not stuck, however long it takes.
    pub idle_timeout: Duration,
    /// Hard ceiling on one turn, for a loop that never stops reporting.
    ///
    /// A backstop rather than the primary control: generous, because the cost of
    /// cutting off real work exceeds the cost of waiting.
    pub max_duration: Duration,
    /// Maximum tool calls the host will service in one turn.
    pub max_tool_calls: usize,
    /// How long an approval request may wait for a verdict.
    ///
    /// Exceeding it is a **refusal**, never a silent approval.
    pub approval_timeout: Duration,
}

impl Default for TurnLimits {
    fn default() -> Self {
        Self {
            // Ten minutes of silence. Long, deliberately: a false positive
            // discards work that may have cost many minutes and many tokens,
            // while a true positive only delays a diagnosis the owner can also
            // make by pressing Stop.
            idle_timeout: Duration::from_secs(600),
            // Two hours: long enough for any analysis that is still making
            // progress, short enough to stop a runaway loop eventually.
            max_duration: Duration::from_secs(7_200),
            max_tool_calls: 32,
            // Generous relative to the turn budget: the owner is a person.
            approval_timeout: Duration::from_secs(300),
        }
    }
}

/// Everything one turn needs beyond the prompt.
///
/// Bundled rather than passed positionally so that adding a concern (round D's
/// trace sink) does not re-shape every call site.
pub struct TurnContext {
    /// Tools advertised to the runtime and dispatched against.
    ///
    /// The same registry serves both roles so the advertised set and the
    /// dispatchable set cannot drift apart unnoticed.
    pub tools: Arc<ToolRegistry>,
    /// Who answers approval requests. Defaults are the caller's choice; an
    /// unwired session must pass [`approval::DenyAll`].
    pub approvals: Arc<dyn ApprovalDecider>,
    /// Optional audit sink for approval decisions.
    pub audit: Option<Arc<ApprovalLog>>,
    /// Whether a permanent approval can take effect in this session.
    ///
    /// Callers pass [`approval::Persistence::Blocked`] whenever the runtime is
    /// confined away from its rule store, which is the default here: a permanent
    /// verdict then silently does nothing, so the option must not be offered.
    pub persistence: approval::Persistence,
    /// JSON Schema constraining this turn's **final** assistant message.
    ///
    /// `None` asks for free-form prose, which is right for a conversational turn
    /// ("what tools do you have?") and wrong for an analysis the interface renders
    /// as a report. Loaded from the repository document rather than inlined here —
    /// see [`report`].
    pub output_schema: Option<Value>,
    /// Where live progress goes, when someone is watching.
    ///
    /// `None` runs the turn silently, which is what tests want. The interface
    /// passes a sink that folds events into the shared status, so a turn is
    /// visible while it runs rather than only after it ends.
    pub progress: Option<ProgressSink>,
    pub limits: TurnLimits,
}

/// Host-side record of one tool call, for the caller's trace and assertions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallRecord {
    pub call_id: String,
    pub tool: String,
    pub arguments: Value,
    /// What the host told the model. `false` covers both a refused unknown tool
    /// and a tool that reported its own failure.
    pub success: bool,
    /// Text handed back to the model, verbatim.
    pub output: String,
}

/// How a turn ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnStatus {
    Completed,
    Failed { detail: String },
}

/// Result of one turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnOutcome {
    pub turn_id: String,
    pub status: TurnStatus,
    /// Every tool call the host serviced, in arrival order.
    pub tool_calls: Vec<ToolCallRecord>,
    /// Every approval request the host answered, in arrival order.
    pub approvals: Vec<ApprovalRecord>,
    /// Every completed `agentMessage` item, in order.
    ///
    /// All of them, because an analysis is not one message: a skill reports
    /// progress as it goes ("stage 1 pre-flight done, two of three fetch waves
    /// complete, …") and then states its conclusion. Keeping only the last threw
    /// away the intermediate results, which is the bulk of what a reader wants to
    /// see in the conversation.
    pub messages: Vec<String>,
    /// The last of [`Self::messages`]; `None` when the turn produced none.
    pub final_message: Option<String>,
    /// The final message parsed as JSON, when the turn was asked for structure and
    /// the result satisfied it.
    ///
    /// Kept alongside the raw text, not instead of it: the interface renders this
    /// as a report but falls back to the prose when parsing fails, so a model that
    /// ignored the schema degrades to Markdown rather than to an empty panel.
    pub report: Option<Value>,
    /// Why a requested report was rejected, when one was asked for and did not
    /// satisfy the schema.
    ///
    /// Surfaced rather than only logged: the interface shows prose where the reader
    /// expected a report, and "the model ignored the structure" is the difference
    /// between a bug and a model that needs a firmer instruction.
    pub report_error: Option<String>,
    /// Server requests refused because this client does not implement them.
    ///
    /// Recorded rather than dropped: a non-empty list means the model tried
    /// something outside the implemented surface, which the caller should see.
    /// Approval requests are **not** listed here — they are answered, and their
    /// verdicts live in [`Self::approvals`].
    pub refused_requests: Vec<String>,
}

impl TurnOutcome {
    pub fn succeeded(&self) -> bool {
        matches!(self.status, TurnStatus::Completed)
    }
}

/// Locates the `codex` executable on `PATH`.
///
/// Returns the first match so that the value used to launch the runtime is the
/// same one a readiness check reports. `None` means it is not installed, which
/// callers must treat as "not ready" rather than as a reason to try something
/// else — there is no fallback runtime.
pub fn discover_program() -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join("codex"))
        .find(|candidate| candidate.is_file())
}

/// A connected, initialized agent runtime.
///
/// Owning an `AgentSession` means "a codex child process is running and has
/// completed the handshake". Dropping it tears the child down.
pub struct AgentSession {
    server: CodexAppServer,
    /// The `initialize` result, kept so callers can inspect negotiated
    /// capabilities without repeating the handshake.
    handshake: Value,
    /// Thread started by [`Self::start_thread`], if any.
    thread_id: Option<String>,
    /// Host-side session trace, when tracing is enabled.
    ///
    /// Owned by the session rather than passed per turn: it is one file per
    /// thread, and every turn of that thread appends to the same one.
    trace: Option<TraceWriter>,
    /// Skills the runtime reported for this thread's working directory.
    skills: Vec<SkillInfo>,
}

impl AgentSession {
    /// Starts the agent runtime and completes the `initialize` handshake.
    pub async fn connect(config: AppServerConfig) -> Result<Self, AgentError> {
        let mut server = CodexAppServer::spawn(config).await?;
        let handshake = server.initialize().await?;
        Ok(Self {
            server,
            handshake,
            thread_id: None,
            trace: None,
            skills: Vec::new(),
        })
    }

    /// The `initialize` result as reported by the runtime.
    pub fn handshake(&self) -> &Value {
        &self.handshake
    }

    /// Retained stderr from the child, newest last.
    pub fn stderr_tail(&self) -> Vec<String> {
        self.server.stderr_tail()
    }

    /// Process-group id of the agent runtime, when it was assigned one.
    pub fn child_id(&self) -> Option<u32> {
        self.server.child_id()
    }

    /// Thread id started by [`Self::start_thread`].
    pub fn thread_id(&self) -> Option<&str> {
        self.thread_id.as_deref()
    }

    /// Sends one JSON-RPC request to the runtime and awaits its response.
    pub async fn request(&mut self, method: &str, params: Value) -> Result<Value, AgentError> {
        self.server.request(method, params).await
    }

    /// Awaits the next server-initiated message; `None` once the child is gone.
    pub async fn next_message(&mut self) -> Option<ServerMessage> {
        self.server.next_message().await
    }

    /// Starts a thread whose only tools are the ones in `tools`.
    ///
    /// `tools` is consumed by value and stored, so the registry a turn
    /// dispatches against is exactly the registry whose specs were sent.
    pub async fn start_thread(
        &mut self,
        options: &ThreadOptions,
        tools: Arc<ToolRegistry>,
    ) -> Result<String, AgentError> {
        // Roots must be registered before the thread starts: the skill list is
        // resolved when the thread is created, so a later registration would
        // not reach this thread's prompt.
        if !options.skill_roots.is_empty() {
            self.server.set_skill_roots(&options.skill_roots).await?;
        }

        let thread_id = self.server.start_thread(options, tools.specs()).await?;
        self.thread_id = Some(thread_id.clone());

        // What the model will actually be offered, asked rather than assumed:
        // the runtime decides which roots are usable, and it namespaces skills
        // from extra roots. Listing is best-effort — a session without its skill
        // list is still usable, so a failure here must not abort the start.
        match self
            .server
            .list_skills(std::slice::from_ref(&options.cwd), true)
            .await
        {
            Ok(skills) => self.skills = skills,
            Err(error) => {
                tracing::warn!(target: "agent", %error, "could not list skills");
                self.skills = Vec::new();
            }
        }

        // Open the trace before the session_started event so a failure to open
        // is reported here, not silently mid-turn.
        self.trace = match &options.trace_dir {
            Some(directory) => {
                let writer = TraceWriter::create(directory, &thread_id).map_err(|source| {
                    AgentError::TraceFile {
                        path: directory.join(format!("{thread_id}.jsonl")),
                        source,
                    }
                })?;
                writer.try_record(&TraceEvent::SessionStarted {
                    thread_id: thread_id.clone(),
                    cwd: options.cwd.clone(),
                    sandbox: options.sandbox,
                    approval_policy: options.approval_policy,
                    approvals_reviewer: options.approvals_reviewer,
                    skill_roots: options
                        .skill_roots
                        .iter()
                        .map(|root| root.to_string_lossy().into_owned())
                        .collect(),
                    developer_instructions: options
                        .developer_instructions
                        .as_deref()
                        .map(trace::InstructionsFingerprint::of),
                    at_ms: now_ms(),
                });
                Some(writer)
            }
            None => None,
        };

        Ok(thread_id)
    }

    /// Skills the runtime will offer the model in this thread.
    ///
    /// Empty when the runtime reported none, or when the listing failed — a
    /// missing skill list must not fail a session that is otherwise usable.
    pub fn skills(&self) -> &[SkillInfo] {
        &self.skills
    }

    /// Path of the session trace, when tracing is enabled.
    pub fn trace_path(&self) -> Option<&std::path::Path> {
        self.trace.as_ref().map(TraceWriter::path)
    }

    /// Runs one turn, servicing tool calls and routing approval requests to the
    /// owner.
    ///
    /// Returns when the runtime reports the turn finished, or when a host limit
    /// is exceeded. Approval requests are answered with a verdict (or a refusal
    /// if the verdict does not arrive in time); server requests outside the
    /// implemented surface are refused with a JSON-RPC error and recorded in
    /// [`TurnOutcome::refused_requests`].
    pub async fn run_turn(
        &mut self,
        prompt: &str,
        context: TurnContext,
    ) -> Result<TurnOutcome, AgentError> {
        let TurnContext {
            tools,
            approvals,
            audit,
            persistence,
            output_schema,
            progress,
            limits,
        } = context;

        let thread_id = self
            .thread_id
            .clone()
            .ok_or_else(|| AgentError::UnexpectedResponse {
                method: "turn/start".to_string(),
                detail: "no thread has been started on this session".to_string(),
            })?;
        // The requirement is also stated in words, derived from the schema so the
        // two cannot disagree. Measured: the schema field alone was not enough — the
        // model refused, reporting that no structure had been provided.
        let fragment = output_schema
            .as_ref()
            .map(crate::agent::report::context_fragment);
        let turn_id = self
            .server
            .start_turn(
                &thread_id,
                prompt,
                output_schema.as_ref(),
                fragment
                    .as_ref()
                    .map(|(source, value)| (source.as_str(), value.as_str())),
            )
            .await?;

        let mut outcome = TurnOutcome {
            turn_id: turn_id.clone(),
            status: TurnStatus::Completed,
            tool_calls: Vec::new(),
            approvals: Vec::new(),
            messages: Vec::new(),
            final_message: None,
            report: None,
            report_error: None,
            refused_requests: Vec::new(),
        };

        if let Some(trace) = &self.trace {
            trace.try_record(&TraceEvent::TurnStarted {
                turn_id: turn_id.clone(),
                prompt: prompt.to_string(),
                at_ms: now_ms(),
            });
        }

        let result = self
            .drive_turn(
                &mut outcome,
                &tools,
                &*approvals,
                &audit,
                persistence,
                output_schema.is_some(),
                progress
                    .as_ref()
                    .map(|sink| sink.as_ref() as &(dyn Fn(TurnProgress) + Send + Sync)),
                &limits,
            )
            .await;

        // Exactly one turn_finished per turn, on every exit path: a trace with a
        // turn that never ends would be indistinguishable from a crash, and the
        // difference matters when attributing one.
        if let Some(trace) = &self.trace {
            let status = match &result {
                Ok(()) => RecordedTurnStatus::Completed,
                Err(error) => RecordedTurnStatus::Failed {
                    detail: error.to_string(),
                },
            };
            trace.try_record(&TraceEvent::TurnFinished {
                turn_id: turn_id.clone(),
                status,
                final_message: outcome.final_message.clone(),
                at_ms: now_ms(),
            });
        }

        result.map(|()| outcome)
    }

    /// Drives one turn's message loop, filling `outcome`.
    ///
    /// Split from [`Self::run_turn`] so that the turn-finished trace event is
    /// written once, by the caller, regardless of which error path is taken.
    #[allow(clippy::too_many_arguments)]
    async fn drive_turn(
        &mut self,
        outcome: &mut TurnOutcome,
        tools: &Arc<ToolRegistry>,
        approvals: &dyn ApprovalDecider,
        audit: &Option<Arc<ApprovalLog>>,
        persistence: approval::Persistence,
        wants_report: bool,
        progress: Option<&(dyn Fn(TurnProgress) + Send + Sync)>,
        limits: &TurnLimits,
    ) -> Result<(), AgentError> {
        // Announced here rather than by the caller so a sink cannot observe
        // item events for a turn whose start it never saw.
        report(
            progress,
            TurnProgress::Started {
                turn_id: outcome.turn_id.clone(),
            },
        );
        let turn_id = outcome.turn_id.clone();
        let started = tokio::time::Instant::now();
        // Reset whenever the turn shows signs of life. A turn that keeps
        // reporting is not stuck, however long it runs; see `TurnLimits`.
        let mut last_activity = started;

        loop {
            let idle_deadline = last_activity + limits.idle_timeout;
            let ceiling = started + limits.max_duration;
            let message = match tokio::time::timeout_at(
                idle_deadline.min(ceiling),
                self.server.next_message(),
            )
            .await
            {
                Err(_elapsed) => {
                    let (reason, timeout) = if idle_deadline <= ceiling {
                        (TurnTimeoutReason::Idle, limits.idle_timeout)
                    } else {
                        (TurnTimeoutReason::MaxDuration, limits.max_duration)
                    };
                    // Stop the runtime before returning. Measured: a thread left
                    // mid-turn silently ignores the next `turn/start` and
                    // re-reports the running turn, so the following turn would
                    // watch the wrong turn's events and end on its completion.
                    self.interrupt_turn(&turn_id).await;
                    return Err(AgentError::TurnTimeout {
                        turn_id,
                        reason,
                        timeout,
                    });
                }
                Ok(None) => {
                    return Err(AgentError::Closed {
                        detail: "agent stdout reached EOF during turn".to_string(),
                    });
                }
                Ok(Some(message)) => message,
            };

            match message {
                ServerMessage::Request { id, method, params } => {
                    if method == "item/tool/call" {
                        if outcome.tool_calls.len() >= limits.max_tool_calls {
                            return Err(AgentError::TooManyToolCalls {
                                limit: limits.max_tool_calls,
                            });
                        }
                        let (record, payload) = dispatch_tool_call(tools, &params).await;
                        self.server.respond(id, payload.to_wire()).await?;
                        if let Some(trace) = &self.trace {
                            trace.try_record(&TraceEvent::ToolCall {
                                call_id: record.call_id.clone(),
                                tool: record.tool.clone(),
                                arguments: record.arguments.clone(),
                                success: record.success,
                                output: record.output.clone(),
                                at_ms: now_ms(),
                            });
                        }
                        outcome.tool_calls.push(record);
                    } else if let Some(kind) = ApprovalKind::from_method(&method) {
                        // A side-effecting action: the owner decides, and a
                        // missing verdict is a refusal.
                        // Announced before the decider runs, so a watcher sees
                        // the wait while it lasts rather than only afterwards.
                        report(
                            progress,
                            TurnProgress::ApprovalRequested {
                                request_id: id,
                                summary: summarise(kind, &params),
                            },
                        );
                        let (record, payload) = decide_approval(
                            approvals,
                            id,
                            kind,
                            &method,
                            &params,
                            persistence,
                            limits,
                            started + limits.max_duration,
                        )
                        .await;
                        if let Some(trace) = &self.trace {
                            trace.try_record(&TraceEvent::Approval {
                                record: record.clone(),
                            });
                        }
                        if let Some(audit) = &audit {
                            // The audit trail is evidence; failing to write it
                            // must not silently lose a decision, so it is
                            // reported on stderr and the turn continues with a
                            // refusal already recorded in the outcome.
                            if let Err(error) = audit.append(&record) {
                                tracing::error!(
                                    target: "agent",
                                    path = %audit.path().display(),
                                    %error,
                                    "failed to append approval audit record"
                                );
                            }
                        }
                        self.server.respond(id, payload).await?;
                        report(
                            progress,
                            TurnProgress::ApprovalResolved {
                                request_id: record.request_id,
                            },
                        );
                        outcome.approvals.push(record);
                    } else {
                        // Anything else (elicitation, auth refresh, attestation)
                        // is outside this round. Refusing fails the operation
                        // closed instead of inventing a decision.
                        self.server.respond_not_implemented(id, &method).await?;
                        if let Some(trace) = &self.trace {
                            trace.try_record(&TraceEvent::RefusedRequest {
                                method: method.clone(),
                                at_ms: now_ms(),
                            });
                        }
                        outcome.refused_requests.push(method);
                    }
                }
                ServerMessage::Notification { method, params } => {
                    // Everything observable is mirrored to the sink first, then
                    // acted on: the stream is a view of the turn, so it must not
                    // diverge based on which branch returns.
                    forward_progress(progress, &method, params.as_ref());
                    match method.as_str() {
                        "turn/completed" => return Ok(()),
                        "turn/failed" => {
                            let detail = params
                                .as_ref()
                                .map(|params| params.to_string())
                                .unwrap_or_default();
                            return Err(AgentError::TurnFailed { turn_id, detail });
                        }
                        "item/completed" => {
                            if let Some(text) = completed_agent_message(params.as_ref()) {
                                if let Some(trace) = &self.trace {
                                    trace.try_record(&TraceEvent::Item {
                                        turn_id: turn_id.clone(),
                                        kind: "agent_message".to_string(),
                                        text: text.clone(),
                                        at_ms: now_ms(),
                                    });
                                }
                                if wants_report {
                                    // Two ways a requested report fails to arrive, and
                                    // both are worth telling the reader about: the model
                                    // produced no JSON at all, or it produced JSON that
                                    // does not satisfy the schema. The second was
                                    // measured — it invented its own field names.
                                    match serde_json::from_str::<Value>(&text) {
                                        Ok(report) => {
                                            match crate::agent::report::validate(&report) {
                                                Ok(()) => outcome.report = Some(report),
                                                Err(reason) => {
                                                    tracing::warn!(
                                                        target: "agent",
                                                        %reason,
                                                        "final message did not satisfy the report schema; \
                                                         keeping the prose instead"
                                                    );
                                                    outcome.report_error = Some(reason);
                                                }
                                            }
                                        }
                                        Err(error) => {
                                            let reason = format!("最终回复不是 JSON（{error}）");
                                            tracing::warn!(
                                                target: "agent",
                                                %reason,
                                                "final message was not JSON; keeping the prose"
                                            );
                                            outcome.report_error = Some(reason);
                                        }
                                    }
                                }
                                outcome.final_message = Some(text.clone());
                                outcome.messages.push(text);
                            }
                        }
                        _ => {}
                    }
                }
            }

            // Time we spent handling — dispatching a tool, waiting for a verdict
            // — is not idleness. Without this, a slow tool would push the idle
            // deadline into the past and the very next pass would call the turn
            // stuck, immediately after it had just finished working.
            last_activity = tokio::time::Instant::now();
        }
    }

    /// Asks the runtime to stop the running turn.
    ///
    /// Needed before abandoning a turn: measured, a thread left mid-turn
    /// silently ignores the next `turn/start` and re-reports the running turn,
    /// so a following turn would watch the wrong turn's events.
    ///
    /// Best-effort by design — the caller is already on an error path, and a
    /// failure here must not replace the reason the turn was abandoned. The
    /// thread id is read from this session because the caller may not have it.
    async fn interrupt_turn(&mut self, turn_id: &str) {
        let Some(thread_id) = self.thread_id.clone() else {
            return;
        };
        match self
            .server
            .request(
                "turn/interrupt",
                serde_json::json!({ "threadId": thread_id, "turnId": turn_id }),
            )
            .await
        {
            Ok(_) => tracing::debug!(target: "agent", turn_id, "interrupted the running turn"),
            Err(error) => tracing::warn!(
                target: "agent",
                turn_id,
                %error,
                "could not interrupt the running turn; the runtime may still be working"
            ),
        }
    }

    /// Stops the agent runtime.
    ///
    /// Consumes the session so that a stopped runtime cannot be used again by
    /// mistake; a fresh [`Self::connect`] is required.
    ///
    /// `session_finished` is recorded only when the shutdown actually succeeded,
    /// so `finished: true` in a replay means the session ended deliberately —
    /// not merely that it stopped.
    pub async fn shutdown(self) -> Result<(), AgentError> {
        let Self { server, trace, .. } = self;
        let outcome = server.shutdown().await;
        if outcome.is_ok()
            && let Some(trace) = &trace
        {
            trace.try_record(&TraceEvent::SessionFinished { at_ms: now_ms() });
        }
        outcome
    }
}

/// Wall-clock milliseconds, or 0 when the clock is unavailable.
///
/// Used only for ordering trace events and audit records: a missing clock must
/// not fail a turn, and a 0 timestamp still orders correctly relative to other
/// 0s.
fn now_ms() -> u64 {
    crate::market::unix_timestamp_ms().unwrap_or_default()
}

/// Asks the owner about one approval request and builds the reply.
///
/// Every path out of this function produces a verdict; none of them leaves the
/// runtime waiting. The deadline is applied **here** rather than inside the
/// decider, so the fail-closed guarantee does not depend on implementers
/// remembering to honour it.
#[allow(clippy::too_many_arguments)]
async fn decide_approval(
    decider: &dyn ApprovalDecider,
    id: i64,
    kind: ApprovalKind,
    method: &str,
    params: &Value,
    persistence: approval::Persistence,
    limits: &TurnLimits,
    turn_deadline: tokio::time::Instant,
) -> (ApprovalRecord, Value) {
    let request = ApprovalRequest {
        id,
        method: method.to_string(),
        kind,
        summary: summarise(kind, params),
        details: params.clone(),
        advertised: advertised_decisions(params),
        options: approval::available_decisions(kind, params, persistence),
    };

    let started = tokio::time::Instant::now();
    // Never wait past the turn's own budget either.
    let approval_deadline = (started + limits.approval_timeout).min(turn_deadline);
    let verdict = match tokio::time::timeout_at(approval_deadline, decider.decide(&request)).await {
        Ok(decision) => (decision, DecisionSource::Decider),
        // No verdict in time is a refusal. This is the property that makes the
        // gate fail closed rather than open.
        Err(_elapsed) => (Decision::Deny, DecisionSource::Timeout),
    };
    let waited_ms = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);

    let (decision, source) = verdict;
    let (payload, decision, source) = match response_payload(kind, decision, params) {
        Ok(payload) => (payload, decision, source),
        Err(error) => {
            // The verdict could not be expressed on the wire, so refusal is the
            // only safe substitute. The reason is recorded rather than hidden:
            // an `Allow` that silently became a `Deny` would be indistinguishable
            // from the owner having refused.
            tracing::error!(target: "agent", %error, "unrepresentable approval verdict");
            let refusal = response_payload(kind, Decision::Deny, params).unwrap_or_else(|_| {
                // Every kind has a representable refusal; reaching here would
                // mean the vocabulary itself changed.
                json!({ "decision": "decline" })
            });
            (refusal, Decision::Deny, DecisionSource::Unsupported)
        }
    };

    (
        ApprovalRecord {
            request_id: id,
            method: request.method,
            kind,
            summary: request.summary,
            decision,
            source,
            decided_at_ms: crate::market::unix_timestamp_ms().unwrap_or_default(),
            waited_ms,
            advertised: request.advertised,
        },
        payload,
    )
}

/// Dispatches one `item/tool/call`, returning the record and the reply payload.
///
/// The blocking tool body runs on a blocking thread so that a multi-second
/// archive replay cannot park the async runtime.
async fn dispatch_tool_call(
    tools: &Arc<ToolRegistry>,
    params: &Value,
) -> (ToolCallRecord, ToolOutcome) {
    let call_id = params
        .get("callId")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let tool = params
        .get("tool")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);

    let registry = Arc::clone(tools);
    let tool_for_task = tool.clone();
    let arguments_for_task = arguments.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        match registry.dispatch(&tool_for_task, &arguments_for_task) {
            Ok(outcome) => outcome,
            // A tool the host never registered is refused here, with the
            // available names attached. There is no fallback path: the model is
            // told the call failed rather than being silently given something
            // else.
            Err(error) => ToolOutcome::failed(error.to_string()),
        }
    })
    .await
    .unwrap_or_else(|join_error| {
        ToolOutcome::failed(format!("tool execution panicked: {join_error}"))
    });

    let record = ToolCallRecord {
        call_id,
        tool,
        arguments,
        success: outcome.success,
        output: outcome.first_text().unwrap_or_default().to_string(),
    };
    (record, outcome)
}

/// Calls the sink, tolerating its absence.
fn report(progress: Option<&(dyn Fn(TurnProgress) + Send + Sync)>, event: TurnProgress) {
    if let Some(progress) = progress {
        progress(event);
    }
}

/// Translates the notifications that carry visible progress.
///
/// Only the handful that move the view are forwarded; the runtime sends eighty-odd
/// methods per session and mirroring all of them would bury the signal. Each
/// branch reads fields defensively: a notification whose shape changed should
/// drop out of the stream, not panic the turn.
fn forward_progress(
    progress: Option<&(dyn Fn(TurnProgress) + Send + Sync)>,
    method: &str,
    params: Option<&Value>,
) {
    let Some(progress) = progress else {
        return;
    };
    let Some(params) = params else {
        return;
    };
    let item_id = |params: &Value| {
        params
            .get("itemId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let delta = |params: &Value| {
        params
            .get("delta")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };

    match method {
        "item/reasoning/summaryTextDelta" | "item/reasoning/textDelta" => {
            progress(TurnProgress::ReasoningDelta {
                item_id: item_id(params),
                text: delta(params),
            });
        }
        "item/agentMessage/delta" => {
            progress(TurnProgress::MessageDelta {
                item_id: item_id(params),
                text: delta(params),
            });
        }
        "item/commandExecution/outputDelta"
        | "command/exec/outputDelta"
        | "process/outputDelta"
        | "item/fileChange/outputDelta" => {
            progress(TurnProgress::ItemOutput {
                item_id: item_id(params),
                text: delta(params),
            });
        }
        "item/started" => {
            if let Some(item) = params.get("item") {
                progress(TurnProgress::ItemStarted(progress_item(item)));
            }
        }
        "item/completed" => {
            if let Some(item) = params.get("item") {
                progress(TurnProgress::ItemFinished {
                    item_id: item
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    state: item
                        .get("status")
                        .and_then(Value::as_str)
                        .map(ItemState::from_wire)
                        .unwrap_or(ItemState::Completed),
                    output: item
                        .get("aggregatedOutput")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    exit_code: item.get("exitCode").and_then(Value::as_i64),
                    duration_ms: item.get("durationMs").and_then(Value::as_u64),
                });
            }
        }
        "thread/tokenUsage/updated" => {
            progress(TurnProgress::Tokens(token_usage(params)));
        }
        "turn/completed" | "turn/failed" => progress(TurnProgress::Stage(Stage::Done)),
        _ => {}
    }
}

/// Builds the visible form of an item from the runtime's payload.
fn progress_item(item: &Value) -> ProgressItem {
    let kind = item
        .get("type")
        .and_then(Value::as_str)
        .map(ItemKind::from_wire)
        .unwrap_or(ItemKind::Other);
    // A command is identified by its command line; a tool call by its name.
    // Neither is guaranteed present, so fall back to the type rather than to an
    // empty label the interface would render as a blank row.
    let title = item
        .get("command")
        .and_then(Value::as_str)
        .or_else(|| item.get("tool").and_then(Value::as_str))
        .or_else(|| item.get("name").and_then(Value::as_str))
        .or_else(|| item.get("type").and_then(Value::as_str))
        .unwrap_or("item")
        .to_string();
    let detail = item.get("cwd").and_then(Value::as_str).map(str::to_string);
    ProgressItem {
        item_id: item
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        kind,
        title,
        detail,
        state: item
            .get("status")
            .and_then(Value::as_str)
            .map(ItemState::from_wire)
            .unwrap_or(ItemState::Running),
        output: item
            .get("aggregatedOutput")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        exit_code: item.get("exitCode").and_then(Value::as_i64),
        duration_ms: item.get("durationMs").and_then(Value::as_u64),
    }
}

/// Reads the runtime's token accounting.
fn token_usage(params: &Value) -> TokenUsage {
    let usage = params.get("tokenUsage").unwrap_or(params);
    let last = usage.get("last").unwrap_or(usage);
    let field = |name: &str| {
        last.get(name)
            .or_else(|| usage.get(name))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    TokenUsage {
        input_tokens: field("inputTokens"),
        cached_input_tokens: field("cachedInputTokens"),
        output_tokens: field("outputTokens"),
        reasoning_output_tokens: field("reasoningOutputTokens"),
        total_tokens: field("totalTokens"),
        context_window: usage.get("modelContextWindow").and_then(Value::as_u64),
    }
}

/// Text of a completed `agentMessage` item, if this notification is one.
fn completed_agent_message(params: Option<&Value>) -> Option<String> {
    let item = params?.get("item")?;
    if item.get("type").and_then(Value::as_str) != Some("agentMessage") {
        return None;
    }
    item.get("text").and_then(Value::as_str).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::app_server::AppServerConfig;
    use crate::agent::approval::{
        self, ApprovalDecider, ApprovalKind, ApprovalLog, ApprovalRequest, Decision, DecisionSource,
    };
    use crate::agent::protocol::ApprovalPolicy;
    use crate::agent::tools::{AgentTool, ToolOutcome};
    use crate::agent::trace::{RecordedTurnStatus, SandboxMode};
    use serde_json::json;
    use std::path::PathBuf;

    /// The `codex` binary, or `None` when it is not installed.
    ///
    /// `None` rather than a failure so the suite stays green on a machine
    /// without codex, while still exercising the real runtime where it exists.
    /// Shares [`discover_program`] with the readiness check so both agree on
    /// what "installed" means.
    fn codex_program() -> Option<PathBuf> {
        discover_program()
    }

    fn fast_config(program: PathBuf) -> AppServerConfig {
        AppServerConfig {
            program,
            request_timeout: Duration::from_secs(60),
            ..AppServerConfig::default()
        }
    }

    /// A tool that exists only to exercise the mechanism.
    ///
    /// The host deliberately ships **no** tools. The one it had
    /// (`taoli_shadow_report`) reported the arbitrage observation archive, which
    /// is the wrong scope for stock analysis — an analysis of a listed company
    /// has nothing to do with cross-venue arbitrage bookkeeping. The mechanism
    /// stays, and stays tested, so a correctly-scoped tool has somewhere to go.
    struct EchoTool;

    impl AgentTool for EchoTool {
        fn name(&self) -> &'static str {
            "taoli_test_echo"
        }

        fn description(&self) -> &'static str {
            "Return a fixed marker. Exists only for tests."
        }

        fn input_schema(&self) -> serde_json::Value {
            json!({
                "type": "object",
                "properties": {},
                "required": [],
                "additionalProperties": false,
            })
        }

        fn call(&self, _arguments: &serde_json::Value) -> ToolOutcome {
            ToolOutcome::text("MARKER-42")
        }
    }

    fn echo_registry() -> Arc<ToolRegistry> {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(EchoTool));
        Arc::new(registry)
    }

    /// Approves every request once.
    ///
    /// The timeout tests need the command to actually run: with a refusal the
    /// turn ends immediately and there is no silence to measure.
    struct AllowOnce;

    #[async_trait::async_trait]
    impl ApprovalDecider for AllowOnce {
        async fn decide(&self, _request: &ApprovalRequest) -> Decision {
            Decision::AllowOnce
        }
    }

    /// A context that lets commands run, for tests about time rather than about
    /// the gate.
    fn allowing_context(tools: Arc<ToolRegistry>, limits: TurnLimits) -> TurnContext {
        TurnContext {
            tools,
            approvals: Arc::new(AllowOnce),
            audit: None,
            persistence: approval::Persistence::Blocked,
            output_schema: None,
            progress: None,
            limits,
        }
    }

    /// Options that make every side-effecting action reach the owner.
    ///
    /// `UnlessTrusted` is required for that: a read-only sandbox alone produces
    /// no approval traffic, as measured in round C.
    fn restricted_options(trace_dir: Option<PathBuf>) -> ThreadOptions {
        ThreadOptions {
            approval_policy: ApprovalPolicy::UnlessTrusted,
            trace_dir,
            ..ThreadOptions::default()
        }
    }

    /// A context whose approval channel refuses everything, as an unwired
    /// session must.
    fn deny_context(tools: Arc<ToolRegistry>) -> TurnContext {
        TurnContext {
            tools,
            approvals: Arc::new(approval::DenyAll),
            audit: None,
            // Tests default to the confined state, which is what the command
            // layer uses: permanent grants are unavailable.
            persistence: approval::Persistence::Blocked,
            output_schema: None,
            progress: None,
            limits: TurnLimits::default(),
        }
    }

    #[test]
    fn completed_agent_message_reads_only_agent_messages() {
        let agent = json!({ "item": { "type": "agentMessage", "text": "hi" } });
        assert_eq!(
            completed_agent_message(Some(&agent)),
            Some("hi".to_string())
        );

        let other = json!({ "item": { "type": "userMessage", "text": "hi" } });
        assert_eq!(completed_agent_message(Some(&other)), None);
        assert_eq!(completed_agent_message(None), None);
    }

    #[tokio::test]
    async fn handshake_succeeds_against_the_real_runtime() {
        let Some(program) = codex_program() else {
            eprintln!("skipping: codex is not installed on PATH");
            return;
        };
        let session = AgentSession::connect(fast_config(program))
            .await
            .expect("handshake with the real codex app-server must succeed");
        assert!(
            session.handshake().is_object(),
            "initialize must return an object, got {}",
            session.handshake()
        );
        session.shutdown().await.expect("shutdown must succeed");
    }

    #[tokio::test]
    async fn connect_fails_fast_when_the_binary_is_absent() {
        let config = AppServerConfig {
            program: PathBuf::from("/nonexistent/codex-not-installed"),
            ..AppServerConfig::default()
        };
        let error = AgentSession::connect(config)
            .await
            .err()
            .expect("connect must fail");
        assert!(matches!(error, AgentError::Spawn { .. }), "got {error:?}");
        assert!(error.to_string().contains("Install codex"));
    }

    /// A child that dies must fail subsequent requests instead of leaving the
    /// caller parked on a response that can never arrive.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_killed_child_fails_requests_instead_of_hanging() {
        let Some(program) = codex_program() else {
            eprintln!("skipping: codex is not installed on PATH");
            return;
        };
        let mut session = AgentSession::connect(fast_config(program))
            .await
            .expect("handshake must succeed");

        let pid = session.child_id().expect("child has a pid while running");
        // Signal the whole group: with a shim-installed codex the direct child
        // is a launcher and the real runtime is its grandchild.
        let killed = std::process::Command::new("kill")
            .args(["-KILL", &format!("-{pid}")])
            .status()
            .expect("kill must be runnable");
        assert!(killed.success(), "kill -KILL -{pid} failed");

        // The server emits ordinary notifications before EOF, so drain until
        // the channel closes; that is the reliable "runtime is gone" signal.
        let observed_eof = tokio::time::timeout(Duration::from_secs(20), async {
            while session.next_message().await.is_some() {}
        })
        .await;
        observed_eof.expect("EOF must be observed within 20s after kill -KILL");

        let outcome = tokio::time::timeout(
            Duration::from_secs(20),
            session.request(
                "initialize",
                json!({ "clientInfo": { "name": "taoli", "version": "0.1.0" } }),
            ),
        )
        .await
        .expect("the request must not hang after the child is killed");

        let error = outcome.expect_err("a dead child cannot answer");
        assert!(
            matches!(
                error,
                AgentError::Closed { .. } | AgentError::Transport { .. }
            ),
            "expected Closed or Transport, got {error:?}"
        );
    }

    /// Shutting down must leave nothing behind in the agent's process group.
    #[cfg(unix)]
    #[tokio::test]
    async fn shutdown_leaves_no_process_in_the_agent_group() {
        let Some(program) = codex_program() else {
            eprintln!("skipping: codex is not installed on PATH");
            return;
        };
        let session = AgentSession::connect(fast_config(program))
            .await
            .expect("handshake must succeed");
        let pgid = session.child_id().expect("child has a pid while running");
        session.shutdown().await.expect("shutdown must succeed");

        let remaining = processes_in_group(pgid);
        assert!(
            remaining.is_empty(),
            "shutdown left processes in group {pgid}: {remaining:?}"
        );
    }

    #[cfg(unix)]
    fn processes_in_group(pgid: u32) -> Vec<String> {
        let output = std::process::Command::new("ps")
            .args(["-o", "pid=,command=", "-g", &pgid.to_string()])
            .output()
            .expect("ps must be runnable");
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect()
    }

    /// Acceptance (round B): the model calls a host tool and the host answers.
    #[tokio::test]
    async fn turn_calls_the_registered_tool() {
        let Some(program) = codex_program() else {
            eprintln!("skipping: codex is not installed on PATH");
            return;
        };
        let tools = echo_registry();
        let mut session = AgentSession::connect(fast_config(program))
            .await
            .expect("handshake must succeed");
        session
            .start_thread(&ThreadOptions::default(), Arc::clone(&tools))
            .await
            .expect("thread must start");

        let outcome = session
            .run_turn(
                "Call the taoli_test_echo tool and report exactly what it returns.",
                deny_context(tools),
            )
            .await
            .expect("turn must run to completion");

        assert!(outcome.succeeded(), "status was {:?}", outcome.status);
        assert_eq!(
            outcome.tool_calls.len(),
            1,
            "expected exactly one tool call, got {:?}",
            outcome.tool_calls
        );
        let call = &outcome.tool_calls[0];
        assert_eq!(call.tool, "taoli_test_echo");
        assert!(call.success, "tool {call:?} must succeed");
        assert_eq!(
            call.output, "MARKER-42",
            "the host's answer must reach the model"
        );
        assert!(
            outcome.final_message.is_some(),
            "the model must produce a final message"
        );

        session.shutdown().await.expect("shutdown must succeed");
    }

    /// Acceptance (round B): a tool call the host cannot serve is refused with an
    /// error, and nothing stands in for it.
    ///
    /// The runtime only routes calls for tools it advertised, so an unknown name
    /// is reachable only when the advertised set and the dispatching registry
    /// disagree — which this test constructs deliberately. That divergence must
    /// **fail closed**: the model is told the call failed, and the turn still
    /// completes instead of hanging on an unanswered request.
    #[tokio::test]
    async fn turn_refuses_a_tool_the_dispatcher_does_not_have() {
        let Some(program) = codex_program() else {
            eprintln!("skipping: codex is not installed on PATH");
            return;
        };
        // Advertised to the runtime...
        let advertised = echo_registry();
        // ...but nothing is dispatchable.
        let dispensable = Arc::new(ToolRegistry::new());

        let mut session = AgentSession::connect(fast_config(program))
            .await
            .expect("handshake must succeed");
        session
            .start_thread(&ThreadOptions::default(), Arc::clone(&advertised))
            .await
            .expect("thread must start");

        let outcome = session
            .run_turn(
                "Call the taoli_test_echo tool and report exactly what it returns.",
                deny_context(dispensable),
            )
            .await
            .expect("turn must run to completion despite the refused call");

        assert!(outcome.succeeded(), "status was {:?}", outcome.status);
        assert_eq!(
            outcome.tool_calls.len(),
            1,
            "expected the advertised tool to be called once, got {:?}",
            outcome.tool_calls
        );
        let call = &outcome.tool_calls[0];
        assert_eq!(call.tool, "taoli_test_echo");
        assert!(!call.success, "an unknown tool must be refused: {call:?}");
        assert!(
            call.output.contains("unknown tool"),
            "refusal must explain itself, got {:?}",
            call.output
        );
        // The refusal travelled over the wire as this exact payload.
        let payload = ToolOutcome::failed(call.output.clone()).to_wire();
        assert_eq!(payload["success"], json!(false));

        session.shutdown().await.expect("shutdown must succeed");
    }

    /// A temporary directory unique to one test.
    fn scratch_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "taoli-agent-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    /// Acceptance (round C): a side-effecting action is put to the owner, the
    /// owner's refusal is what the runtime receives, and the turn survives it.
    #[tokio::test]
    async fn a_refused_command_is_denied_and_the_turn_survives() {
        let Some(program) = codex_program() else {
            eprintln!("skipping: codex is not installed on PATH");
            return;
        };
        let scratch = scratch_dir("deny");
        let audit = Arc::new(ApprovalLog::new(scratch.join("approvals.ndjson")));

        let tools = Arc::new(ToolRegistry::new());
        let mut session = AgentSession::connect(fast_config(program))
            .await
            .expect("handshake must succeed");
        session
            .start_thread(&restricted_options(None), Arc::clone(&tools))
            .await
            .expect("thread must start");

        let outcome = session
            .run_turn(
                "Run the shell command `echo hello-from-round-c` and report its output.",
                TurnContext {
                    tools,
                    approvals: Arc::new(approval::DenyAll),
                    audit: Some(Arc::clone(&audit)),
                    persistence: approval::Persistence::Blocked,
                    output_schema: None,
                    progress: None,
                    limits: TurnLimits::default(),
                },
            )
            .await
            .expect("turn must run to completion despite the refusal");

        assert!(outcome.succeeded(), "status was {:?}", outcome.status);
        assert!(
            !outcome.approvals.is_empty(),
            "the model tried a shell command, so an approval must have been requested"
        );

        let record = &outcome.approvals[0];
        assert!(
            matches!(
                record.kind,
                ApprovalKind::CommandExecution | ApprovalKind::ExecCommand
            ),
            "unexpected kind {:?}",
            record.kind
        );
        assert_eq!(record.decision, Decision::Deny, "the owner refused");
        assert_eq!(record.source, DecisionSource::Decider);
        assert!(
            record.summary.contains("echo hello-from-round-c"),
            "summary must show what was asked: {:?}",
            record.summary
        );

        // The audit trail must contain exactly what the outcome reported.
        let written = ApprovalLog::read_all(audit.path()).expect("audit readable");
        assert_eq!(written, outcome.approvals, "audit must match the outcome");

        session.shutdown().await.expect("shutdown must succeed");
        let _ = std::fs::remove_dir_all(&scratch);
    }

    /// Acceptance (round C): a decider that never answers results in a refusal,
    /// not in a hang and not in consent.
    #[tokio::test]
    async fn an_approval_that_times_out_is_refused_not_granted() {
        let Some(program) = codex_program() else {
            eprintln!("skipping: codex is not installed on PATH");
            return;
        };
        let scratch = scratch_dir("timeout");
        let audit = Arc::new(ApprovalLog::new(scratch.join("approvals.ndjson")));

        // Never returns.
        struct NeverAnswers;
        #[async_trait::async_trait]
        impl ApprovalDecider for NeverAnswers {
            async fn decide(&self, _request: &ApprovalRequest) -> Decision {
                std::future::pending::<()>().await;
                unreachable!()
            }
        }

        let tools = Arc::new(ToolRegistry::new());
        let mut session = AgentSession::connect(fast_config(program))
            .await
            .expect("handshake must succeed");
        session
            .start_thread(&restricted_options(None), Arc::clone(&tools))
            .await
            .expect("thread must start");

        let outcome = session
            .run_turn(
                "Run the shell command `echo timeout-probe` and report its output.",
                TurnContext {
                    tools,
                    approvals: Arc::new(NeverAnswers),
                    audit: Some(Arc::clone(&audit)),
                    persistence: approval::Persistence::Blocked,
                    output_schema: None,
                    progress: None,
                    limits: TurnLimits {
                        // Short enough to keep the suite quick, long enough for
                        // the runtime to raise the request.
                        approval_timeout: Duration::from_secs(2),
                        ..TurnLimits::default()
                    },
                },
            )
            .await
            .expect("the turn must finish; a silent decider must not hang it");

        assert!(
            !outcome.approvals.is_empty(),
            "the approval request must have been raised"
        );
        let record = &outcome.approvals[0];
        assert_eq!(
            record.decision,
            Decision::Deny,
            "a missing verdict must become a refusal"
        );
        assert_eq!(
            record.source,
            DecisionSource::Timeout,
            "the source must make the timeout visible to the auditor"
        );
        assert!(
            record.waited_ms >= 1_000,
            "the verdict really waited for the deadline: {} ms",
            record.waited_ms
        );
        // And the denial is on record.
        let written = ApprovalLog::read_all(audit.path()).expect("audit readable");
        assert_eq!(written[0].decision, Decision::Deny);
        assert_eq!(written[0].source, DecisionSource::Timeout);

        session.shutdown().await.expect("shutdown must succeed");
        let _ = std::fs::remove_dir_all(&scratch);
    }

    /// Acceptance (round C): a refusal is not a suggestion — the agent cannot
    /// obtain the effect by asking again.
    #[tokio::test]
    async fn a_refusal_cannot_be_bypassed_by_asking_again() {
        let Some(program) = codex_program() else {
            eprintln!("skipping: codex is not installed on PATH");
            return;
        };
        // Count the requests; every one must be refused.
        struct CountingDeny {
            seen: std::sync::atomic::AtomicUsize,
        }
        #[async_trait::async_trait]
        impl ApprovalDecider for CountingDeny {
            async fn decide(&self, _request: &ApprovalRequest) -> Decision {
                self.seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Decision::Deny
            }
        }

        let tools = Arc::new(ToolRegistry::new());
        let mut session = AgentSession::connect(fast_config(program))
            .await
            .expect("handshake must succeed");
        session
            .start_thread(&restricted_options(None), Arc::clone(&tools))
            .await
            .expect("thread must start");

        let decider = Arc::new(CountingDeny {
            seen: std::sync::atomic::AtomicUsize::new(0),
        });
        let outcome = session
            .run_turn(
                "Run the shell command `echo bypass-attempt-1`, then if it fails run \
                 `echo bypass-attempt-2` instead. Report what happened.",
                TurnContext {
                    tools,
                    approvals: Arc::clone(&decider) as Arc<dyn ApprovalDecider>,
                    audit: None,
                    persistence: approval::Persistence::Blocked,
                    output_schema: None,
                    progress: None,
                    limits: TurnLimits::default(),
                },
            )
            .await
            .expect("turn must run to completion");

        assert!(
            !outcome.approvals.is_empty(),
            "at least one approval must have been requested"
        );
        for record in &outcome.approvals {
            assert_eq!(
                record.decision,
                Decision::Deny,
                "every request must be refused, including retries: {record:?}"
            );
        }
        // The decider saw every request: nothing was auto-approved on a retry.
        let seen = decider.seen.load(std::sync::atomic::Ordering::SeqCst);
        assert_eq!(
            seen,
            outcome.approvals.len(),
            "every request must have gone to the owner"
        );

        session.shutdown().await.expect("shutdown must succeed");
    }

    /// Acceptance (round D): a live session is traced to disk and replays into
    /// the chain it recorded, cross-checked against what the turn reported.
    #[tokio::test]
    async fn a_live_session_replays_into_the_chain_it_recorded() {
        let Some(program) = codex_program() else {
            eprintln!("skipping: codex is not installed on PATH");
            return;
        };
        let scratch = scratch_dir("trace-live");
        let tools = echo_registry();
        let mut session = AgentSession::connect(fast_config(program))
            .await
            .expect("handshake must succeed");

        let options = ThreadOptions {
            trace_dir: Some(scratch.clone()),
            ..ThreadOptions::default()
        };
        let thread_id = session
            .start_thread(&options, Arc::clone(&tools))
            .await
            .expect("thread must start");

        let outcome = session
            .run_turn(
                "Call the taoli_test_echo tool and then repeat what it returned.",
                deny_context(tools),
            )
            .await
            .expect("turn must run to completion");

        let trace_path = session
            .trace_path()
            .expect("tracing was enabled")
            .to_path_buf();
        let expected_path = scratch.join(format!("{thread_id}.jsonl"));
        assert_eq!(trace_path, expected_path, "trace is addressed by thread id");

        session.shutdown().await.expect("shutdown must succeed");

        // Replay what the host wrote...
        let trace = crate::agent::trace::replay(&trace_path).expect("trace replays");
        assert_eq!(trace.thread_id, thread_id);
        assert_eq!(trace.sandbox, SandboxMode::ReadOnly);
        assert_eq!(trace.approval_policy, ApprovalPolicy::UnlessTrusted);
        assert_eq!(trace.turns.len(), 1, "one turn was run");

        let turn = &trace.turns[0];
        assert_eq!(turn.turn_id, outcome.turn_id);
        assert_eq!(turn.status, Some(RecordedTurnStatus::Completed));
        assert_eq!(turn.final_message, outcome.final_message);

        // ...and it must agree with what the turn itself reported. This is the
        // property that makes the trace usable as evidence: the two records of
        // the same events cannot disagree.
        assert_eq!(turn.tool_calls.len(), outcome.tool_calls.len());
        for (recorded, reported) in turn.tool_calls.iter().zip(&outcome.tool_calls) {
            assert_eq!(recorded.call_id, reported.call_id);
            assert_eq!(recorded.tool, reported.tool);
            assert_eq!(recorded.arguments, reported.arguments);
            assert_eq!(recorded.success, reported.success);
            assert_eq!(recorded.output, reported.output);
        }
        assert_eq!(turn.approvals, outcome.approvals);
        assert_eq!(turn.refused_requests, outcome.refused_requests);
        assert_eq!(
            trace.tool_call_counts().get("taoli_test_echo"),
            Some(&1),
            "the tool call must be attributable by name"
        );

        let _ = std::fs::remove_dir_all(&scratch);
    }

    /// Acceptance (round D): tracing works for an ephemeral thread too, which
    /// leaves nothing on the runtime's side.
    #[tokio::test]
    async fn an_ephemeral_thread_still_produces_a_host_trace() {
        let Some(program) = codex_program() else {
            eprintln!("skipping: codex is not installed on PATH");
            return;
        };
        let scratch = scratch_dir("trace-ephemeral");
        let tools = Arc::new(ToolRegistry::new());
        let mut session = AgentSession::connect(fast_config(program))
            .await
            .expect("handshake must succeed");

        // Ephemeral is the default: the runtime persists nothing for it.
        let options = ThreadOptions {
            ephemeral: true,
            trace_dir: Some(scratch.clone()),
            ..ThreadOptions::default()
        };
        session
            .start_thread(&options, Arc::clone(&tools))
            .await
            .expect("thread must start");
        session
            .run_turn("Reply with the single word: traced.", deny_context(tools))
            .await
            .expect("turn must run");

        let trace_path = session.trace_path().expect("traced").to_path_buf();
        session.shutdown().await.expect("shutdown must succeed");

        let trace = crate::agent::trace::replay(&trace_path).expect("replays");
        assert!(trace.finished, "the session was shut down cleanly");
        assert_eq!(trace.turns.len(), 1);
        assert_eq!(trace.turns[0].status, Some(RecordedTurnStatus::Completed));
        assert_eq!(trace.turns[0].prompt, "Reply with the single word: traced.");

        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[tokio::test]
    async fn an_unusable_trace_directory_fails_at_thread_start() {
        let Some(program) = codex_program() else {
            eprintln!("skipping: codex is not installed on PATH");
            return;
        };
        let tools = Arc::new(ToolRegistry::new());
        let mut session = AgentSession::connect(fast_config(program))
            .await
            .expect("handshake must succeed");

        // A path whose parent is a regular file can never become a directory.
        let scratch = scratch_dir("trace-bad");
        let blocker = scratch.join("blocker");
        std::fs::write(&blocker, b"not a directory").expect("write blocker");

        let options = ThreadOptions {
            trace_dir: Some(blocker.join("trace")),
            ..ThreadOptions::default()
        };
        let error = session
            .start_thread(&options, tools)
            .await
            .expect_err("an unusable trace directory must fail the thread start");
        assert!(
            matches!(error, AgentError::TraceFile { .. }),
            "expected TraceFile, got {error:?}"
        );

        let _ = std::fs::remove_dir_all(&scratch);
    }

    /// Acceptance (round F): the runtime is started inside this project's own
    /// confinement, and still works.
    #[tokio::test]
    async fn the_runtime_starts_under_our_own_confinement() {
        let Some(program) = codex_program() else {
            eprintln!("skipping: codex is not installed on PATH");
            return;
        };
        if !crate::agent::sandbox::probe().is_available() {
            eprintln!("skipping: seatbelt confinement is unavailable here");
            return;
        }

        let codex_home = PathBuf::from(std::env::var("HOME").expect("HOME")).join(".codex");
        let policy = crate::agent::sandbox::codex_child_policy(&codex_home, None);
        let config = AppServerConfig {
            program,
            request_timeout: Duration::from_secs(60),
            confinement: Some(policy),
            ..AppServerConfig::default()
        };

        let session = AgentSession::connect(config)
            .await
            .expect("the confined runtime must still complete the handshake");
        assert!(session.handshake().is_object());
        session.shutdown().await.expect("shutdown must succeed");
    }

    /// A confinement that cannot be applied must stop the launch, not be skipped.
    ///
    /// The policy used here is **deliberately unbuildable** (a relative writable
    /// root), so the confinement fails whichever way the availability probe
    /// goes. That matters: an earlier version of this test branched on a
    /// separate `probe()` call, and since `connect` probes again internally the
    /// two could disagree — a real flake, not a hypothetical one.
    #[tokio::test]
    async fn an_unbuildable_confinement_refuses_to_launch() {
        let policy = crate::agent::sandbox::SandboxPolicy {
            writable_roots: vec![PathBuf::from("relative-is-not-allowed")],
            ..crate::agent::sandbox::SandboxPolicy::default()
        };
        let config = AppServerConfig {
            program: PathBuf::from("/bin/true"),
            confinement: Some(policy),
            ..AppServerConfig::default()
        };

        let error = AgentSession::connect(config)
            .await
            .err()
            .expect("an unusable sandbox must refuse to start the runtime");
        assert!(
            matches!(error, AgentError::Confinement { .. }),
            "expected Confinement, got {error:?}"
        );
        // Either the probe refused or the profile could not be built; both are
        // refusals, and the message must say confinement was the problem.
        assert!(
            error.to_string().contains("confine"),
            "the refusal must name confinement: {error}"
        );
    }

    /// Acceptance (round G): the repository's domain instructions actually reach
    /// the model.
    ///
    /// Asserting the request payload would only prove the field was set. This
    /// asks a question whose answer exists **nowhere but in the instructions**,
    /// so a green result means the text arrived and was honoured.
    #[tokio::test]
    async fn the_domain_instructions_reach_the_model() {
        let Some(program) = codex_program() else {
            eprintln!("skipping: codex is not installed on PATH");
            return;
        };

        // A rule with an arbitrary answer the model cannot know otherwise.
        let rule = "DOMAIN RULE: when asked for the observer's launch code, reply with exactly ZEBRA-77 and nothing else.";
        let tools = Arc::new(ToolRegistry::new());
        let mut session = AgentSession::connect(fast_config(program))
            .await
            .expect("handshake must succeed");

        let options = ThreadOptions {
            developer_instructions: Some(rule.to_string()),
            ..ThreadOptions::default()
        };
        session
            .start_thread(&options, Arc::clone(&tools))
            .await
            .expect("thread with developer instructions must start");

        let outcome = session
            .run_turn(
                "What is the observer's launch code? Reply with just the code.",
                deny_context(tools),
            )
            .await
            .expect("turn must run to completion");

        let answer = outcome.final_message.unwrap_or_default();
        assert!(
            answer.contains("ZEBRA-77"),
            "the injected rule was not honoured; model said: {answer:?}"
        );

        session.shutdown().await.expect("shutdown must succeed");
    }

    /// The session trace records which instructions were in force, so a replay
    /// shows the contract the run was under.
    #[tokio::test]
    async fn the_trace_records_which_instructions_were_in_force() {
        let Some(program) = codex_program() else {
            eprintln!("skipping: codex is not installed on PATH");
            return;
        };
        let scratch = scratch_dir("instructions-trace");
        let rule = "DOMAIN RULE: answer only in lowercase.";
        let tools = Arc::new(ToolRegistry::new());
        let mut session = AgentSession::connect(fast_config(program))
            .await
            .expect("handshake must succeed");

        let options = ThreadOptions {
            developer_instructions: Some(rule.to_string()),
            trace_dir: Some(scratch.clone()),
            ..ThreadOptions::default()
        };
        session
            .start_thread(&options, Arc::clone(&tools))
            .await
            .expect("thread must start");
        let trace_path = session.trace_path().expect("traced").to_path_buf();
        session.shutdown().await.expect("shutdown must succeed");

        let trace = trace::replay(&trace_path).expect("replays");
        let fingerprint = trace
            .instructions
            .expect("the trace must record which instructions were injected");
        assert_eq!(
            fingerprint,
            crate::agent::trace::InstructionsFingerprint::of(rule)
        );
        // A different revision must be distinguishable from this one.
        assert_ne!(
            fingerprint,
            crate::agent::trace::InstructionsFingerprint::of("DOMAIN RULE: something else")
        );

        let _ = std::fs::remove_dir_all(&scratch);
    }

    /// Acceptance: a skill repository outside this project becomes visible to
    /// the model once it is registered as an extra root.
    ///
    /// Before this, a session saw only the runtime's own skills — which is why
    /// an installed skill repository was never invoked: it was in another
    /// directory, and nothing told the runtime to look there.
    #[tokio::test]
    async fn registering_a_skill_root_makes_its_skills_visible() {
        let Some(program) = codex_program() else {
            eprintln!("skipping: codex is not installed on PATH");
            return;
        };
        // Point at a repository with a skill layout. Absent means skip rather
        // than fail: the root is machine-specific, like the codex binary.
        let Some(root) = std::env::var_os("TAOLI_TEST_SKILL_ROOT").map(PathBuf::from) else {
            eprintln!("skipping: TAOLI_TEST_SKILL_ROOT is not set");
            return;
        };
        if !root.is_dir() {
            eprintln!("skipping: {} is not a directory", root.display());
            return;
        }

        let tools = Arc::new(ToolRegistry::new());
        let mut session = AgentSession::connect(fast_config(program))
            .await
            .expect("handshake must succeed");

        // Baseline: without a root, the repository's skills are invisible.
        session
            .start_thread(&ThreadOptions::default(), Arc::clone(&tools))
            .await
            .expect("thread must start");
        let without: Vec<String> = session.skills().iter().map(|s| s.name.clone()).collect();
        assert!(
            !without.iter().any(|name| name.ends_with(":uzi")),
            "the baseline must not already see the registered skill: {without:?}"
        );

        // With the root registered, they appear — namespaced by source.
        let options = ThreadOptions {
            skill_roots: vec![root.clone()],
            ..ThreadOptions::default()
        };
        session
            .start_thread(&options, Arc::clone(&tools))
            .await
            .expect("thread must start");
        let with: Vec<String> = session.skills().iter().map(|s| s.name.clone()).collect();
        assert!(
            with.iter().any(|name| name.ends_with(":uzi")),
            "registering {} must surface its skills, got {with:?}",
            root.display()
        );
        // The name is namespaced, which is how a project skill is told apart
        // from a built-in one.
        assert!(
            with.iter().any(|name| name.contains(':')),
            "extra-root skills carry a namespace: {with:?}"
        );
        // Every reported skill must carry a path we could open; a skill the
        // model cannot read is not really available.
        for skill in session.skills() {
            assert!(skill.path.is_file(), "{skill:?}");
            assert!(!skill.description.is_empty(), "{skill:?}");
        }

        session.shutdown().await.expect("shutdown must succeed");
    }

    /// A root that does not exist must not take the session down with it.
    #[tokio::test]
    async fn an_unusable_skill_root_does_not_break_the_session() {
        let Some(program) = codex_program() else {
            eprintln!("skipping: codex is not installed on PATH");
            return;
        };
        let tools = Arc::new(ToolRegistry::new());
        let mut session = AgentSession::connect(fast_config(program))
            .await
            .expect("handshake must succeed");

        let options = ThreadOptions {
            skill_roots: vec![PathBuf::from("/nonexistent/taoli-skill-root")],
            ..ThreadOptions::default()
        };
        session
            .start_thread(&options, Arc::clone(&tools))
            .await
            .expect("an irrelevant skill root must not prevent the session");
        // Whatever the runtime decides about the root, the session is usable.
        assert!(session.thread_id().is_some());

        session.shutdown().await.expect("shutdown must succeed");
    }

    /// The fixed bug, as a controlled A/B on one identical command.
    ///
    /// A real analysis was cut off at 300s while it was still reporting progress.
    /// The instrument was wrong: a **total** budget measures duration, but what
    /// marks a turn stuck is *silence*. The same chatty command is run twice,
    /// changing only which limit binds:
    ///
    /// * bounded by a total budget (the old shape) → killed mid-work;
    /// * bounded by idleness (the new shape, with a ceiling far away) → finishes.
    ///
    /// The command emits a line every second for ~10s, so it never goes quiet for
    /// the 3s idle limit while far exceeding the 4s total one.
    #[tokio::test]
    async fn the_same_working_turn_survives_idleness_but_not_a_total_budget() {
        let Some(program) = codex_program() else {
            eprintln!("skipping: codex is not installed on PATH");
            return;
        };
        let command = "Run exactly this shell command and nothing else:                        for i in 1 2 3 4 5 6 7 8 9 10; do echo tick-$i; sleep 1; done";

        // (a) The old instrument: a total budget shorter than the work.
        {
            let tools = Arc::new(ToolRegistry::new());
            let mut session = AgentSession::connect(fast_config(program.clone()))
                .await
                .expect("handshake");
            session
                .start_thread(&restricted_options(None), Arc::clone(&tools))
                .await
                .expect("thread");
            let error = session
                .run_turn(
                    command,
                    allowing_context(
                        tools,
                        TurnLimits {
                            idle_timeout: Duration::from_secs(600),
                            max_duration: Duration::from_secs(4),
                            approval_timeout: Duration::from_secs(30),
                            ..TurnLimits::default()
                        },
                    ),
                )
                .await
                .expect_err("a total budget shorter than the work must cut the turn off");
            assert!(
                matches!(
                    error,
                    AgentError::TurnTimeout {
                        reason: TurnTimeoutReason::MaxDuration,
                        ..
                    }
                ),
                "expected the ceiling to fire, got {error:?}"
            );
            let _ = session.shutdown().await;
        }

        // (b) The new instrument: idleness, with the ceiling far away.
        {
            let tools = Arc::new(ToolRegistry::new());
            let mut session = AgentSession::connect(fast_config(program))
                .await
                .expect("handshake");
            session
                .start_thread(&restricted_options(None), Arc::clone(&tools))
                .await
                .expect("thread");
            let started = std::time::Instant::now();
            let outcome = session
                .run_turn(
                    command,
                    allowing_context(
                        tools,
                        TurnLimits {
                            idle_timeout: Duration::from_secs(3),
                            max_duration: Duration::from_secs(600),
                            approval_timeout: Duration::from_secs(30),
                            ..TurnLimits::default()
                        },
                    ),
                )
                .await
                .expect("a turn that keeps reporting must not be called stuck");
            let elapsed = started.elapsed();
            assert!(outcome.succeeded(), "status was {:?}", outcome.status);
            assert!(
                elapsed > Duration::from_secs(5),
                "the turn must have outlived the idle limit to prove the point: {elapsed:?}"
            );
            let _ = session.shutdown().await;
        }
    }

    /// The other half: a turn that goes silent *is* stopped, and the reason says
    /// which limit fired.
    #[tokio::test]
    async fn a_silent_turn_is_interrupted_and_says_why() {
        let Some(program) = codex_program() else {
            eprintln!("skipping: codex is not installed on PATH");
            return;
        };
        let tools = Arc::new(ToolRegistry::new());
        let mut session = AgentSession::connect(fast_config(program))
            .await
            .expect("handshake must succeed");
        session
            .start_thread(&restricted_options(None), Arc::clone(&tools))
            .await
            .expect("thread must start");

        // `sleep` with no output: the runtime reports nothing while it runs.
        let error = session
            .run_turn(
                "Run exactly this shell command: sleep 60. Then say done.",
                allowing_context(
                    Arc::clone(&tools),
                    TurnLimits {
                        idle_timeout: Duration::from_secs(3),
                        max_duration: Duration::from_secs(300),
                        approval_timeout: Duration::from_secs(30),
                        ..TurnLimits::default()
                    },
                ),
            )
            .await
            .expect_err("a silent turn must be stopped");

        match &error {
            AgentError::TurnTimeout { reason, .. } => {
                assert_eq!(*reason, TurnTimeoutReason::Idle);
            }
            other => panic!("expected an idle timeout, got {other:?}"),
        }
        // The message must say it was interrupted, not merely abandoned.
        assert!(error.to_string().contains("interrupted"), "{error}");

        // And the thread must be usable again: measured, a thread left mid-turn
        // silently ignores the next `turn/start` and re-reports the running
        // turn.
        let next = session
            .run_turn(
                "Reply with the single word: ok",
                allowing_context(
                    tools,
                    TurnLimits {
                        idle_timeout: Duration::from_secs(30),
                        max_duration: Duration::from_secs(120),
                        ..TurnLimits::default()
                    },
                ),
            )
            .await
            .expect("the thread must accept a new turn after the interrupt");
        assert_ne!(next.turn_id, "unset", "a fresh turn must have been started");

        session.shutdown().await.expect("shutdown must succeed");
    }

    /// The ceiling fires even while the turn is still reporting.
    ///
    /// A short ceiling and a long idle limit isolate the two: only the ceiling
    /// can end this turn.
    #[tokio::test]
    async fn the_ceiling_ends_a_turn_that_is_still_working() {
        let Some(program) = codex_program() else {
            eprintln!("skipping: codex is not installed on PATH");
            return;
        };
        let tools = Arc::new(ToolRegistry::new());
        let mut session = AgentSession::connect(fast_config(program))
            .await
            .expect("handshake must succeed");
        session
            .start_thread(&restricted_options(None), Arc::clone(&tools))
            .await
            .expect("thread must start");

        let error = session
            .run_turn(
                "Run exactly this shell command: for i in $(seq 1 40); do echo tick-$i; sleep 1; done. Then say done.",
                allowing_context(
                    tools,
                    TurnLimits {
                        // Long idle: the turn is chatty, so idleness never fires.
                        idle_timeout: Duration::from_secs(60),
                        // Short ceiling: this is the only limit that can end it.
                        max_duration: Duration::from_secs(6),
                        approval_timeout: Duration::from_secs(30),
                        ..TurnLimits::default()
                    },
                ),
            )
            .await
            .expect_err("the ceiling must end the turn");

        match &error {
            AgentError::TurnTimeout { reason, .. } => {
                assert_eq!(*reason, TurnTimeoutReason::MaxDuration, "{error}");
            }
            other => panic!("expected a ceiling timeout, got {other:?}"),
        }

        session.shutdown().await.expect("shutdown must succeed");
    }

    /// The schema reaches the runtime and the result comes back structured.
    ///
    /// Measured: with `outputSchema` set, the model's final message *is* the JSON —
    /// no surrounding prose — while any progress it reported earlier stays text.
    /// Without it, the same question yields prose and no report.
    #[tokio::test]
    async fn a_report_turn_returns_structured_output() {
        let Some(program) = codex_program() else {
            eprintln!("skipping: codex is not installed on PATH");
            return;
        };
        let Ok(schema) = crate::agent::report::load_default() else {
            eprintln!("skipping: the report schema is absent");
            return;
        };

        let tools = Arc::new(ToolRegistry::new());
        let mut session = AgentSession::connect(fast_config(program))
            .await
            .expect("handshake must succeed");
        session
            .start_thread(&restricted_options(None), Arc::clone(&tools))
            .await
            .expect("thread must start");

        let outcome = session
            .run_turn(
                "分析 002600.SZ。按给定结构输出；取不到数据就留空，不要编造。",
                TurnContext {
                    tools,
                    approvals: Arc::new(approval::DenyAll),
                    audit: None,
                    persistence: approval::Persistence::Blocked,
                    output_schema: Some(schema),
                    progress: None,
                    limits: TurnLimits::default(),
                },
            )
            .await
            .expect("turn must run");

        let report = outcome.report.as_ref().unwrap_or_else(|| {
            panic!(
                "a turn asked for structure must yield a parsed report; status={:?} messages={:?}",
                outcome.status, outcome.messages
            )
        });
        // The three properties the interface keys on.
        for key in crate::agent::report::REQUIRED_PROPERTIES {
            assert!(
                report.get(key).is_some(),
                "report is missing `{key}`: {report}"
            );
        }
        assert_eq!(report["ticker"], serde_json::json!("002600.SZ"));
        // And the raw text is kept alongside, so a rendering failure cannot lose
        // the answer.
        assert!(
            outcome
                .final_message
                .as_deref()
                .is_some_and(|text| text.contains("002600.SZ")),
            "the raw message must survive: {:?}",
            outcome.final_message
        );

        let _ = session.shutdown().await;
    }

    /// A turn that was **not** asked for structure must not invent a report from
    /// prose that happens to be JSON-shaped, nor fail when it is not.
    #[tokio::test]
    async fn a_conversational_turn_yields_no_report() {
        let Some(program) = codex_program() else {
            eprintln!("skipping: codex is not installed on PATH");
            return;
        };
        let tools = Arc::new(ToolRegistry::new());
        let mut session = AgentSession::connect(fast_config(program))
            .await
            .expect("handshake must succeed");
        session
            .start_thread(&restricted_options(None), Arc::clone(&tools))
            .await
            .expect("thread must start");

        let outcome = session
            .run_turn(
                "用一句话说明你有哪些能力。不要输出 JSON。",
                TurnContext {
                    tools,
                    approvals: Arc::new(approval::DenyAll),
                    audit: None,
                    persistence: approval::Persistence::Blocked,
                    output_schema: None,
                    progress: None,
                    limits: TurnLimits::default(),
                },
            )
            .await
            .expect("turn must run");

        assert!(
            outcome.report.is_none(),
            "no schema was requested, so there is no report: {:?}",
            outcome.report
        );
        assert!(outcome.final_message.is_some());

        let _ = session.shutdown().await;
    }
}
