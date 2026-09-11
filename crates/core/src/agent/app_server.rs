//! Driving a `codex app-server` child process over newline-delimited JSON-RPC.
//!
//! Scope is PORT-01 round A: start the server, complete the `initialize`
//! handshake, and stop it again. No turn is sent, no tool is registered and
//! nothing is persisted — those arrive in later rounds.
//!
//! Two properties are load-bearing here and are enforced rather than assumed:
//!
//! * **Credential isolation.** The child is started with `env_clear()` plus an
//!   explicit allowlist, so host secrets cannot reach the agent runtime by
//!   inheritance. See [`child_env`].
//! * **No hangs.** A child that dies (or is killed) makes its stdout reach
//!   EOF; every in-flight request is then failed with the stderr tail
//!   attached, so a caller can never wait on a reply that cannot arrive.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::timeout;

use super::AgentError;
use super::protocol::{
    Incoming, METHOD_NOT_IMPLEMENTED, RequestId, RpcError, RpcRequest, RpcResponse, classify,
    initialize_params, thread_start_params, turn_start_params,
};
use super::trace::ThreadOptions;

/// Client identity reported in the `initialize` handshake.
const CLIENT_NAME: &str = "taoli";
const CLIENT_TITLE: &str = "Taoli Observer";

/// Names copied from the host environment into the child.
///
/// Deliberately minimal: everything absent from this list is *not* passed, and
/// `TAOLI_*` names are refused even if added here later (see [`child_env`]).
pub const ENV_WHITELIST: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "TMPDIR",
    "LANG",
    "LC_ALL",
    "TERM",
    "CODEX_HOME",
];

/// Environment prefixes that must never reach the agent runtime.
const FORBIDDEN_ENV_PREFIX: &str = "TAOLI_";

/// Bounded stderr tail kept for diagnostics. Large enough to hold a panic
/// message and its backtrace header, small enough to be harmless.
const STDERR_TAIL_LINES: usize = 200;

/// One inbound message from the server that is not a response to our request.
///
/// Requests and notifications are separated rather than merged because only
/// requests require a reply; a caller that ignored the distinction would leave
/// the server waiting for `item/tool/call` forever.
#[derive(Debug, Clone)]
pub enum ServerMessage {
    /// The server asks this client to act. Must be answered via
    /// [`CodexAppServer::respond`] or [`CodexAppServer::respond_error`].
    Request {
        id: RequestId,
        method: String,
        params: Value,
    },
    /// Fire-and-forget; recorded for diagnostics only.
    Notification {
        method: String,
        params: Option<Value>,
    },
}

impl ServerMessage {
    pub fn method(&self) -> &str {
        match self {
            Self::Request { method, .. } | Self::Notification { method, .. } => method,
        }
    }
}

/// One skill the runtime reports as available to the model.
///
/// The name may be namespaced (e.g. `stock-deep-analyzer:uzi`) when it came from
/// a registered extra root rather than the runtime's own skill directory: the
/// prefix identifies the source, which is what makes it possible to tell a
/// project skill from a built-in one.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SkillInfo {
    pub name: String,
    pub path: PathBuf,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

/// How to launch the child, and how long to wait at each boundary.
#[derive(Debug, Clone)]
pub struct AppServerConfig {
    pub program: PathBuf,
    /// Upper bound on a single request/response round trip.
    pub request_timeout: Duration,
    /// Upper bound on graceful exit before the child is killed.
    pub shutdown_timeout: Duration,
    /// Confinement to apply to the child.
    ///
    /// `None` leaves the child with whatever confinement the runtime applies to
    /// itself. `Some` wraps the launch in this project's own Seatbelt profile,
    /// and a machine where that cannot be applied is a hard failure rather than
    /// a silent downgrade to running unconfined — see [`Self::program`].
    pub confinement: Option<super::sandbox::SandboxPolicy>,
}

impl Default for AppServerConfig {
    fn default() -> Self {
        Self {
            program: PathBuf::from("codex"),
            request_timeout: Duration::from_secs(30),
            shutdown_timeout: Duration::from_secs(10),
            confinement: None,
        }
    }
}

/// Builds the child environment from [`ENV_WHITELIST`].
///
/// Only variables that are both allowlisted *and* present in the host
/// environment are returned, so the caller can log the exact set without
/// guessing.
pub fn child_env() -> Vec<(String, String)> {
    ENV_WHITELIST
        .iter()
        .filter(|name| !name.starts_with(FORBIDDEN_ENV_PREFIX))
        .filter_map(|name| {
            std::env::var(name)
                .ok()
                .map(|value| ((*name).to_string(), value))
        })
        .collect()
}

/// An `initialize`d connection to a `codex app-server` child process.
pub struct CodexAppServer {
    child: Child,
    /// `None` once shutdown has closed it, which also signals EOF to the child.
    stdin: Option<ChildStdin>,
    /// Process-group id captured at spawn.
    ///
    /// Captured because [`Child::id`] returns `None` after the direct child is
    /// reaped, yet the group must still be swept afterwards: grandchildren can
    /// outlive it (codex spawns `git` for its plugin marketplace).
    group_id: Option<u32>,
    pending: PendingMap,
    next_id: AtomicI64,
    messages: mpsc::UnboundedReceiver<ServerMessage>,
    stderr_tail: Arc<Mutex<VecDeque<String>>>,
    reader: JoinHandle<()>,
    stderr_reader: JoinHandle<()>,
    request_timeout: Duration,
    shutdown_timeout: Duration,
}

/// In-flight requests, keyed by id, awaiting their response.
type PendingMap = Arc<Mutex<HashMap<RequestId, oneshot::Sender<Result<Value, AgentError>>>>>;

impl CodexAppServer {
    /// Starts the child and wires up the reader tasks. Does **not** perform the
    /// handshake — call [`Self::initialize`] next.
    ///
    /// On Unix the child is placed in its own process group. That matters
    /// because a shim-installed `codex` (for example the Node launcher under
    /// `~/.bun/bin`) is a *direct* child that spawns the real native binary as
    /// a grandchild: signalling only the direct child would leave the agent
    /// runtime running with the pipes still open.
    pub async fn spawn(config: AppServerConfig) -> Result<Self, AgentError> {
        let mut command = match &config.confinement {
            // Confined launch: the wrapping happens before anything is spawned,
            // so an unusable sandbox fails here rather than producing an
            // unconfined child that only looks confined.
            Some(policy) => {
                let target = vec![
                    config.program.to_string_lossy().into_owned(),
                    "app-server".to_string(),
                    "--listen".to_string(),
                    "stdio://".to_string(),
                ];
                let argv = super::sandbox::confined_argv(&target, policy)
                    .map_err(|source| AgentError::Confinement { source })?;
                let (program, args) = argv
                    .split_first()
                    .expect("confined_argv always returns a program");
                let mut command = Command::new(program);
                command.args(args);
                command
            }
            None => {
                let mut command = Command::new(&config.program);
                command.args(["app-server", "--listen", "stdio://"]);
                command
            }
        };
        command
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // A dropped client must not leave an orphaned agent runtime behind.
            .kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);
        for (name, value) in child_env() {
            command.env(name, value);
        }

        let mut child = command.spawn().map_err(|source| AgentError::Spawn {
            program: config.program.clone(),
            source,
        })?;

        let stdout = child
            .stdout
            .take()
            .ok_or(AgentError::MissingPipe("stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or(AgentError::MissingPipe("stderr"))?;
        let stdin = child.stdin.take().ok_or(AgentError::MissingPipe("stdin"))?;

        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let stderr_tail = Arc::new(Mutex::new(VecDeque::with_capacity(STDERR_TAIL_LINES)));
        let (messages_tx, messages) = mpsc::unbounded_channel();

        let stderr_reader = spawn_stderr_reader(stderr, Arc::clone(&stderr_tail));
        let reader = spawn_stdout_reader(
            stdout,
            Arc::clone(&pending),
            messages_tx,
            Arc::clone(&stderr_tail),
        );

        Ok(Self {
            group_id: child.id(),
            child,
            stdin: Some(stdin),
            pending,
            next_id: AtomicI64::new(1),
            messages,
            stderr_tail,
            reader,
            stderr_reader,
            request_timeout: config.request_timeout,
            shutdown_timeout: config.shutdown_timeout,
        })
    }

    /// Performs the `initialize` handshake and returns the server's result
    /// verbatim, so callers can assert against the schema-emitted shape.
    pub async fn initialize(&mut self) -> Result<Value, AgentError> {
        let params = initialize_params(CLIENT_NAME, CLIENT_TITLE, env!("CARGO_PKG_VERSION"));
        self.request("initialize", params).await
    }

    /// Sends `method` and waits for its response, bounded by
    /// [`AppServerConfig::request_timeout`].
    pub async fn request(&mut self, method: &str, params: Value) -> Result<Value, AgentError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (respond, response) = oneshot::channel();
        lock(&self.pending).insert(id, respond);

        let frame = RpcRequest { id, method, params };
        if let Err(error) = self.write_frame(&frame).await {
            // The request will never be answered; do not leave it behind.
            lock(&self.pending).remove(&id);
            return Err(error);
        }

        match timeout(self.request_timeout, response).await {
            Ok(Ok(result)) => result,
            // The sender is dropped only when the reader task exits, which
            // means the child is gone.
            Ok(Err(_recv)) => Err(self.closed("child exited while awaiting a response")),
            Err(_elapsed) => {
                lock(&self.pending).remove(&id);
                Err(AgentError::Timeout {
                    method: method.to_string(),
                    timeout: self.request_timeout,
                })
            }
        }
    }

    /// Yields the next server-initiated message, or `None` once the child is
    /// gone (stdout EOF).
    ///
    /// `None` is the reliable "the runtime has exited" signal; the process id
    /// stays set until the child is reaped.
    pub async fn next_message(&mut self) -> Option<ServerMessage> {
        self.messages.recv().await
    }

    /// Answers a server request with a result.
    pub async fn respond(&mut self, id: RequestId, result: Value) -> Result<(), AgentError> {
        self.write_frame(&RpcResponse { id, result }).await
    }

    /// Answers a server request with a JSON-RPC error.
    ///
    /// This is how an unsupported request is **refused**: the runtime turns it
    /// into a failed operation, which is the fail-closed outcome. Silently not
    /// answering would leave the server waiting.
    pub async fn respond_error(
        &mut self,
        id: RequestId,
        code: i64,
        message: impl Into<String>,
    ) -> Result<(), AgentError> {
        self.write_frame(&RpcError::new(id, code, message)).await
    }

    /// Refuses a request this client does not implement.
    pub async fn respond_not_implemented(
        &mut self,
        id: RequestId,
        method: &str,
    ) -> Result<(), AgentError> {
        self.respond_error(
            id,
            METHOD_NOT_IMPLEMENTED,
            format!("`{method}` is not implemented by this client"),
        )
        .await
    }

    /// Starts a thread and returns its id.
    ///
    /// Takes the host's [`ThreadOptions`] rather than the individual fields: the
    /// set grows whenever a session-level knob is added, and seven positional
    /// arguments of which three are strings is a call site waiting to be
    /// transposed.
    pub async fn start_thread(
        &mut self,
        options: &ThreadOptions,
        dynamic_tools: Vec<Value>,
    ) -> Result<String, AgentError> {
        let params = thread_start_params(
            &options.cwd,
            options.ephemeral,
            options.sandbox.as_wire(),
            options.approval_policy,
            Some(options.approvals_reviewer),
            options.developer_instructions.as_deref(),
            dynamic_tools,
        );
        let result = self.request("thread/start", params).await?;
        result
            .get("thread")
            .and_then(|thread| thread.get("id"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| AgentError::UnexpectedResponse {
                method: "thread/start".to_string(),
                detail: format!("no result.thread.id in {result}"),
            })
    }

    /// Starts a turn carrying one text input and returns the turn id.
    pub async fn start_turn(&mut self, thread_id: &str, text: &str) -> Result<String, AgentError> {
        let params = turn_start_params(thread_id, text);
        let result = self.request("turn/start", params).await?;
        result
            .get("turn")
            .and_then(|turn| turn.get("id"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| AgentError::UnexpectedResponse {
                method: "turn/start".to_string(),
                detail: format!("no result.turn.id in {result}"),
            })
    }

    /// Registers `roots` as extra skill directories.
    ///
    /// The runtime discovers skills under `<root>/SKILL.md` and
    /// `<root>/*/SKILL.md`, which is the layout a skill repository uses. Roots
    /// are per-runtime-process, so this must be called on every session; a
    /// registered root also survives until the process exits, which is why the
    /// call is idempotent-by-replacement rather than additive.
    pub async fn set_skill_roots(&mut self, roots: &[PathBuf]) -> Result<(), AgentError> {
        let roots: Vec<String> = roots
            .iter()
            .map(|root| root.to_string_lossy().into_owned())
            .collect();
        self.request(
            "skills/extraRoots/set",
            super::protocol::skill_roots_params(&roots),
        )
        .await
        .map(|_| ())
    }

    /// Lists the skills the runtime will offer the model in `cwds`.
    ///
    /// `force_reload` bypasses the runtime's own cache: without it a list
    /// cached before [`Self::set_skill_roots`] would not show the new roots.
    pub async fn list_skills(
        &mut self,
        cwds: &[String],
        force_reload: bool,
    ) -> Result<Vec<SkillInfo>, AgentError> {
        let result = self
            .request(
                "skills/list",
                super::protocol::skills_list_params(cwds, force_reload),
            )
            .await?;

        // The reply is `{data: [{cwd, skills, errors}]}`. Skills are flattened
        // across cwds and deduplicated by name: the same skill can be reachable
        // from several working directories, and listing it twice would overstate
        // what the model has.
        let mut skills: Vec<SkillInfo> = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for entry in result
            .get("data")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            for skill in entry
                .get("skills")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let Ok(parsed) = serde_json::from_value::<SkillInfo>(skill.clone()) else {
                    // A skill entry we cannot read is skipped rather than
                    // failing the whole list: reporting the others is more
                    // useful than reporting none.
                    tracing::debug!(target: "agent", entry = %skill, "unreadable skill entry");
                    continue;
                };
                if seen.insert(parsed.name.clone()) {
                    skills.push(parsed);
                }
            }
        }
        skills.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(skills)
    }

    /// Returns the retained stderr tail, newest last.
    pub fn stderr_tail(&self) -> Vec<String> {
        lock(&self.stderr_tail).iter().cloned().collect()
    }

    /// Process-group id of the agent runtime, when it was assigned one.
    ///
    /// Exposed for diagnostics and supervision: an operator (or test) needs to
    /// identify the whole agent tree without walking the host process table.
    pub fn child_id(&self) -> Option<u32> {
        self.group_id
    }

    /// Stops the child and everything it spawned.
    ///
    /// Closing stdin is the normal path: the server treats EOF as a shutdown
    /// request and arms its own bounded deadline, so no signal is required for
    /// the child itself. The process group is then swept **unconditionally**,
    /// because a graceful exit of the direct child does not reap grandchildren
    /// — codex, for instance, leaves `git` running while fetching its plugin
    /// marketplace, and those processes keep both pipes open.
    pub async fn shutdown(mut self) -> Result<(), AgentError> {
        self.stdin.take();

        match timeout(self.shutdown_timeout, self.child.wait()).await {
            Ok(Ok(_status)) => {}
            Ok(Err(source)) => return Err(AgentError::Wait { source }),
            Err(_elapsed) => {
                // The child ignored EOF; take it down before sweeping.
                self.child
                    .kill()
                    .await
                    .map_err(|source| AgentError::Wait { source })?;
                let _ = timeout(self.shutdown_timeout, self.child.wait()).await;
            }
        }

        // Sweep last so that grandchildren cannot survive a graceful exit.
        self.kill_group();
        Ok(())
    }

    /// Best-effort immediate termination of the child's process group.
    ///
    /// `kill(-pgid, SIGKILL)` is issued through the `kill` utility rather than a
    /// `libc` dependency. This is synchronous so that it is also usable from
    /// [`Drop`], where awaiting is impossible.
    ///
    /// The exit status is logged but never propagated: `kill` reports failure
    /// when *any* target could not be signalled, which routinely happens for a
    /// group that is already gone. The invariant that actually matters — no
    /// process left in the group — is asserted by
    /// `shutdown_leaves_no_process_in_the_agent_group`.
    fn kill_group(&self) {
        #[cfg(unix)]
        if let Some(pid) = self.group_id {
            // A negative argument addresses the process group, which
            // `process_group(0)` made identical to the child's pid.
            match std::process::Command::new("kill")
                .args(["-KILL", &format!("-{pid}")])
                // `kill`'s own complaint ("Operation not permitted",
                // "No such process") is expected for a partially dead group and
                // would otherwise spray the terminal during teardown.
                .stderr(Stdio::null())
                .status()
            {
                Ok(status) => tracing::debug!(
                    target: "agent",
                    pid,
                    success = status.success(),
                    "issued process-group kill"
                ),
                Err(error) => tracing::warn!(
                    target: "agent",
                    pid,
                    %error,
                    "could not run kill; agent group may outlive this process"
                ),
            }
        }
    }

    async fn write_frame<T: Serialize>(&mut self, frame: &T) -> Result<(), AgentError> {
        let stdin = self.stdin.as_mut().ok_or_else(|| AgentError::Closed {
            detail: "stdin already closed by shutdown".to_string(),
        })?;
        let mut line =
            serde_json::to_string(frame).map_err(|source| AgentError::Encode { source })?;
        // Newline-delimited framing: the transport reads stdin line by line.
        line.push('\n');
        stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|source| AgentError::Transport { source })?;
        stdin
            .flush()
            .await
            .map_err(|source| AgentError::Transport { source })
    }

    fn closed(&self, detail: &str) -> AgentError {
        let mut detail = detail.to_string();
        for line in self.stderr_tail() {
            detail.push_str("\n  stderr: ");
            detail.push_str(&line);
        }
        AgentError::Closed { detail }
    }
}

impl Drop for CodexAppServer {
    fn drop(&mut self) {
        // `kill_on_drop` only reaps the direct child. Kill the group as well so
        // that a shim-spawned grandchild cannot outlive the client.
        self.kill_group();
        // Stopping the readers avoids leaving tasks parked on pipes that will
        // never produce more data.
        self.reader.abort();
        self.stderr_reader.abort();
    }
}

fn spawn_stdout_reader(
    stdout: tokio::process::ChildStdout,
    pending: PendingMap,
    messages: mpsc::UnboundedSender<ServerMessage>,
    stderr_tail: Arc<Mutex<VecDeque<String>>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        // Unparseable lines are skipped rather than treated as fatal: the
        // stream may carry diagnostics too. `next_line` returning `Err` or
        // `None` both mean the stream is finished.
        while let Ok(Some(line)) = lines.next_line().await {
            dispatch_line(&line, &pending, &messages);
        }
        // EOF means the child is gone. Fail every in-flight request so that no
        // caller can block on a response that will never arrive.
        fail_all(&pending, &stderr_tail);
    })
}

fn dispatch_line(
    line: &str,
    pending: &PendingMap,
    messages: &mpsc::UnboundedSender<ServerMessage>,
) {
    let Ok(message) = serde_json::from_str::<Value>(line) else {
        tracing::debug!(target: "agent", line, "skipping unparseable app-server line");
        return;
    };
    match classify(&message) {
        Some(Incoming::Response { id, result }) => {
            if let Some(respond) = lock(pending).remove(&id) {
                let _ = respond.send(Ok(result.clone()));
            }
        }
        Some(Incoming::Error { id, error }) => {
            if let Some(respond) = lock(pending).remove(&id) {
                let _ = respond.send(Err(AgentError::Remote {
                    id,
                    error: error.clone(),
                }));
            }
        }
        Some(Incoming::Request { id, method, params }) => {
            let _ = messages.send(ServerMessage::Request {
                id,
                method: method.to_string(),
                params: params.cloned().unwrap_or(Value::Null),
            });
        }
        Some(Incoming::Notification { method, params }) => {
            let _ = messages.send(ServerMessage::Notification {
                method: method.to_string(),
                params: params.cloned(),
            });
        }
        None => tracing::debug!(
            target: "agent",
            line,
            "app-server sent a message matching no known shape"
        ),
    }
}

fn fail_all(pending: &PendingMap, stderr_tail: &Arc<Mutex<VecDeque<String>>>) {
    let mut tail = String::new();
    for line in lock(stderr_tail).iter() {
        tail.push_str("\n  stderr: ");
        tail.push_str(line);
    }
    let mut pending = lock(pending);
    for (_, respond) in pending.drain() {
        let _ = respond.send(Err(AgentError::Closed {
            detail: format!("app-server stdout reached EOF (child exited){tail}"),
        }));
    }
}

fn spawn_stderr_reader(
    stderr: tokio::process::ChildStderr,
    stderr_tail: Arc<Mutex<VecDeque<String>>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            tracing::debug!(target: "agent", line, "app-server stderr");
            let mut tail = lock(&stderr_tail);
            if tail.len() == STDERR_TAIL_LINES {
                tail.pop_front();
            }
            tail.push_back(line);
        }
    })
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_env_never_carries_host_credentials() {
        // Set a credential the way the real app does, then prove the child
        // environment excludes it. This is the invariant that keeps the agent
        // runtime outside the trading path.
        // SAFETY: single-threaded test setup; no other thread reads the env here.
        unsafe {
            std::env::set_var("TAOLI_BINANCE_API_SECRET", "must-not-leak");
            std::env::set_var("TAOLI_DATABASE_URL", "postgres://must-not-leak");
        }

        let env = child_env();
        for (name, value) in &env {
            assert!(!name.starts_with(FORBIDDEN_ENV_PREFIX), "leaked {name}");
            assert!(!value.contains("must-not-leak"), "leaked value via {name}");
        }
        assert!(
            !env.iter()
                .any(|(name, _)| name == "TAOLI_BINANCE_API_SECRET")
        );
        assert!(!env.iter().any(|(name, _)| name == "TAOLI_DATABASE_URL"));

        // SAFETY: as above.
        unsafe {
            std::env::remove_var("TAOLI_BINANCE_API_SECRET");
            std::env::remove_var("TAOLI_DATABASE_URL");
        }
    }

    #[test]
    fn env_whitelist_contains_no_forbidden_name() {
        // Guards the allowlist itself, not just its use: adding a TAOLI_ name
        // to ENV_WHITELIST must fail here rather than silently leak at runtime.
        for name in ENV_WHITELIST {
            assert!(
                !name.starts_with(FORBIDDEN_ENV_PREFIX),
                "ENV_WHITELIST must not contain {name}"
            );
        }
    }

    #[test]
    fn spawning_a_missing_program_fails_instead_of_hanging() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let config = AppServerConfig {
            program: PathBuf::from("/nonexistent/codex-does-not-exist"),
            ..AppServerConfig::default()
        };
        let Err(error) = runtime.block_on(CodexAppServer::spawn(config)) else {
            panic!("spawn must fail");
        };
        assert!(matches!(error, AgentError::Spawn { .. }), "got {error:?}");
    }

    /// When stdout reaches EOF, every waiting caller must receive an error.
    ///
    /// This is the property that makes a killed child non-hanging: without it
    /// the pending senders are simply dropped when the map is dropped, which is
    /// indistinguishable from a hang to the awaiting caller only by accident.
    #[test]
    fn eof_fails_every_pending_request_with_the_stderr_tail() {
        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let stderr_tail = Arc::new(Mutex::new(VecDeque::new()));
        lock(&stderr_tail).push_back("boom: child died".to_string());

        let mut receivers = Vec::new();
        for id in [1, 2, 3] {
            let (respond, received) = oneshot::channel();
            lock(&pending).insert(id, respond);
            receivers.push(received);
        }
        assert_eq!(lock(&pending).len(), 3);

        fail_all(&pending, &stderr_tail);
        assert!(lock(&pending).is_empty(), "pending map must be drained");

        for received in receivers {
            // `expect_err` is usable here because `Value` is `Debug`.
            let error = received
                .blocking_recv()
                .expect("sender must be invoked, not dropped")
                .expect_err("EOF cannot produce a successful response");
            match error {
                AgentError::Closed { detail } => {
                    assert!(
                        detail.contains("boom: child died"),
                        "stderr tail must travel with the error, got {detail}"
                    );
                }
                other => panic!("expected Closed, got {other:?}"),
            }
        }
    }
}
