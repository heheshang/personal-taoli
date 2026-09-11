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

pub mod app_server;
pub mod approval;
pub mod instructions;
pub mod protocol;
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
    /// The turn did not finish within its deadline.
    TurnTimeout { turn_id: String, timeout: Duration },
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
            Self::TurnTimeout { turn_id, timeout } => write!(
                formatter,
                "turn {turn_id} did not finish within {}s",
                timeout.as_secs()
            ),
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

/// Limits the host places on one turn.
///
/// All three are host policy: a runaway model must not be able to spend
/// unbounded time or money, and none of the bounds can be widened by anything
/// the model says.
#[derive(Debug, Clone)]
pub struct TurnLimits {
    /// Wall-clock budget for the whole turn, including tool execution and any
    /// time spent waiting for the owner.
    pub timeout: Duration,
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
            timeout: Duration::from_secs(300),
            max_tool_calls: 32,
            // Generous relative to the turn budget: the owner is a person.
            approval_timeout: Duration::from_secs(120),
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
    /// Text of the last completed `agentMessage` item.
    pub final_message: Option<String>,
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

        let thread_id = self
            .server
            .start_thread(
                &options.cwd,
                options.ephemeral,
                options.sandbox.as_wire(),
                options.approval_policy,
                options.developer_instructions.as_deref(),
                tools.specs(),
            )
            .await?;
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
            limits,
        } = context;

        let thread_id = self
            .thread_id
            .clone()
            .ok_or_else(|| AgentError::UnexpectedResponse {
                method: "turn/start".to_string(),
                detail: "no thread has been started on this session".to_string(),
            })?;
        let turn_id = self.server.start_turn(&thread_id, prompt).await?;

        let mut outcome = TurnOutcome {
            turn_id: turn_id.clone(),
            status: TurnStatus::Completed,
            tool_calls: Vec::new(),
            approvals: Vec::new(),
            final_message: None,
            refused_requests: Vec::new(),
        };

        if let Some(trace) = &self.trace {
            trace.try_record(&TraceEvent::TurnStarted {
                turn_id: turn_id.clone(),
                prompt: prompt.to_string(),
                at_ms: now_ms(),
            });
        }

        let deadline = tokio::time::Instant::now() + limits.timeout;
        let result = self
            .drive_turn(&mut outcome, &tools, &*approvals, &audit, &limits, deadline)
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
        limits: &TurnLimits,
        deadline: tokio::time::Instant,
    ) -> Result<(), AgentError> {
        let turn_id = outcome.turn_id.clone();
        loop {
            let message = match tokio::time::timeout_at(deadline, self.server.next_message()).await
            {
                Err(_elapsed) => {
                    return Err(AgentError::TurnTimeout {
                        turn_id,
                        timeout: limits.timeout,
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
                        let (record, payload) = decide_approval(
                            approvals, id, kind, &method, &params, limits, deadline,
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
                ServerMessage::Notification { method, params } => match method.as_str() {
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
                            outcome.final_message = Some(text.clone());
                            if let Some(trace) = &self.trace {
                                trace.try_record(&TraceEvent::Item {
                                    turn_id: turn_id.clone(),
                                    kind: "agent_message".to_string(),
                                    text,
                                    at_ms: now_ms(),
                                });
                            }
                        }
                    }
                    _ => {}
                },
            }
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
async fn decide_approval(
    decider: &dyn ApprovalDecider,
    id: i64,
    kind: ApprovalKind,
    method: &str,
    params: &Value,
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
    /// The policy here is valid but the *availability* is forced unavailable, so
    /// the failure comes from the probe rather than the profile.
    #[tokio::test]
    async fn an_unavailable_confinement_refuses_to_launch() {
        if crate::agent::sandbox::probe().is_available() {
            eprintln!(
                "skipping: this machine can apply seatbelt, so the refusal path is unreachable"
            );
            return;
        }
        let config = AppServerConfig {
            program: PathBuf::from("/bin/true"),
            confinement: Some(crate::agent::sandbox::SandboxPolicy::default()),
            ..AppServerConfig::default()
        };
        let error = AgentSession::connect(config)
            .await
            .err()
            .expect("an unusable sandbox must refuse");
        assert!(
            matches!(error, AgentError::Confinement { .. }),
            "expected Confinement, got {error:?}"
        );
        assert!(error.to_string().contains("refusing to run unconfined"));
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
}
