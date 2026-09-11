//! Human approval gate for side-effecting agent actions (PORT-01 round C).
//!
//! The runtime asks the client before it does anything with an external effect.
//! Those requests arrive as JSON-RPC requests and must be answered; this module
//! turns them into something the owner can judge, and turns the owner's verdict
//! back into the wire vocabulary the runtime expects.
//!
//! Three properties are load-bearing, and each is enforced here rather than
//! promised by a caller:
//!
//! * **Fail closed.** The default decider refuses everything, and a decision
//!   that does not arrive inside the deadline is treated as a refusal — never
//!   as consent. See [`DenyAll`] and the timeout handling in
//!   [`super::AgentSession::run_turn`].
//! * **Per action, never standing.** Only "approve this action" and "refuse this
//!   action" are representable. The wire protocol also offers session-scoped
//!   caches and execpolicy/network-policy amendments, which would let the agent
//!   widen its own future permissions without a further human gate; those are
//!   deliberately **not** reachable through [`Decision`].
//! * **Auditable.** Every request and its verdict can be appended to an
//!   append-only log, including requests that were refused because no decider
//!   was configured.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// What the runtime is asking permission to do.
///
/// Both protocol generations are represented because the runtime still emits
/// the older pair (`execCommandApproval`, `applyPatchApproval`) alongside the
/// newer `item/…/requestApproval` methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalKind {
    /// Run a shell command (v2 `item/commandExecution/requestApproval`).
    CommandExecution,
    /// Change files (v2 `item/fileChange/requestApproval`).
    FileChange,
    /// Widen sandbox permissions (v2 `item/permissions/requestApproval`).
    Permissions,
    /// Run a shell command (v1 `execCommandApproval`).
    ExecCommand,
    /// Apply a patch (v1 `applyPatchApproval`).
    ApplyPatch,
}

impl ApprovalKind {
    /// Classifies a server request method, or `None` if it is not an approval.
    pub fn from_method(method: &str) -> Option<Self> {
        match method {
            "item/commandExecution/requestApproval" => Some(Self::CommandExecution),
            "item/fileChange/requestApproval" => Some(Self::FileChange),
            "item/permissions/requestApproval" => Some(Self::Permissions),
            "execCommandApproval" => Some(Self::ExecCommand),
            "applyPatchApproval" => Some(Self::ApplyPatch),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::CommandExecution => "command_execution",
            Self::FileChange => "file_change",
            Self::Permissions => "permissions",
            Self::ExecCommand => "exec_command",
            Self::ApplyPatch => "apply_patch",
        }
    }
}

/// One approval request, normalised for the owner.
#[derive(Debug, Clone)]
pub struct ApprovalRequest {
    /// JSON-RPC id that must be answered.
    pub id: i64,
    /// Method name as received, kept verbatim for the audit trail.
    pub method: String,
    pub kind: ApprovalKind,
    /// One-line description for the owner: the command line, or what changes.
    pub summary: String,
    /// Raw params, retained so the verdict can be audited against what was
    /// actually asked.
    pub details: Value,
    /// Decisions the runtime advertised, when it populated the field.
    ///
    /// Recorded for audit: the runtime does not always offer an explicit
    /// refusal token, and a future protocol change would show up here.
    pub advertised: Vec<String>,
}

/// The owner's verdict. Deliberately binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// Approve this action, once.
    Allow,
    /// Refuse this action.
    Deny,
}

/// Where a verdict came from. Recorded so a refusal forced by a timeout is
/// distinguishable from a refusal the owner actually made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionSource {
    /// A decider returned a verdict in time.
    Decider,
    /// No verdict arrived inside the deadline; treated as a refusal.
    Timeout,
    /// The host could not represent the requested approval (see
    /// [`ApprovalError::Unrepresentable`]); treated as a refusal.
    Unsupported,
}

/// Decides approval requests.
///
/// Implementations are `async` so that a real owner channel (a UI, a queue, a
/// command line) can await a person. The **deadline is applied by the session**,
/// not by the implementation: a decider that blocks forever still results in a
/// refusal.
#[async_trait]
pub trait ApprovalDecider: Send + Sync {
    async fn decide(&self, request: &ApprovalRequest) -> Decision;
}

/// The default decider: refuses everything.
///
/// This is what a session gets when no owner channel is attached, which is the
/// state of the read-only boundary today. Its existence is why an unwired
/// session cannot silently become a permissive one.
pub struct DenyAll;

#[async_trait]
impl ApprovalDecider for DenyAll {
    async fn decide(&self, _request: &ApprovalRequest) -> Decision {
        Decision::Deny
    }
}

/// Adapts a closure into a decider, for tests and for callers that already have
/// a channel to the owner.
pub struct CallbackDecider<F> {
    callback: F,
}

impl<F> CallbackDecider<F> {
    pub fn new(callback: F) -> Self {
        Self { callback }
    }
}

#[async_trait]
impl<F> ApprovalDecider for CallbackDecider<F>
where
    F: Fn(&ApprovalRequest) -> Decision + Send + Sync,
{
    async fn decide(&self, request: &ApprovalRequest) -> Decision {
        (self.callback)(request)
    }
}

/// Why an owner verdict could not be expressed on the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalError {
    /// The runtime asked for something this client cannot grant or refuse in a
    /// single-action form.
    Unrepresentable {
        kind: ApprovalKind,
        decision: Decision,
    },
}

impl std::fmt::Display for ApprovalError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unrepresentable { kind, decision } => write!(
                formatter,
                "cannot express {decision:?} for a {:?} approval in a single-action form",
                kind
            ),
        }
    }
}

impl std::error::Error for ApprovalError {}

/// Builds the response payload for `request` given `decision`.
///
/// Approvals are single-action by construction: an `Allow` authorises exactly
/// the action described in `request`, and a `Deny` refuses exactly that action.
/// Session-wide caches and policy amendments are unreachable here on purpose —
/// they would widen future authority without a new gate.
pub fn response_payload(
    kind: ApprovalKind,
    decision: Decision,
    request_details: &Value,
) -> Result<Value, ApprovalError> {
    match kind {
        // v2 command/file-change use the same accept/decline vocabulary.
        ApprovalKind::CommandExecution | ApprovalKind::FileChange => Ok(json!({
            "decision": match decision {
                Decision::Allow => "accept",
                Decision::Deny => "decline",
            }
        })),
        // v1 vocabulary.
        ApprovalKind::ExecCommand | ApprovalKind::ApplyPatch => Ok(json!({
            "decision": match decision {
                Decision::Allow => "approved",
                Decision::Deny => "denied",
            }
        })),
        // Granting permissions is a *widening*; only the explicit refusal is
        // representable without echoing back the requested profile, and this
        // module does not grant for the owner.
        ApprovalKind::Permissions => match decision {
            // An empty profile grants nothing, which is the refusal.
            Decision::Deny => Ok(json!({ "permissions": {} })),
            Decision::Allow => {
                // Echoing the requested profile would grant it. That is a
                // policy decision this module must not make implicitly, so it
                // is reported rather than guessed: the session refuses and
                // records the fact.
                let requested = request_details.get("permissions").cloned();
                match requested {
                    Some(permissions) if !permissions.is_null() => {
                        Ok(json!({ "permissions": permissions }))
                    }
                    _ => Err(ApprovalError::Unrepresentable { kind, decision }),
                }
            }
        },
    }
}

/// One audited approval decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalRecord {
    pub request_id: i64,
    pub method: String,
    pub kind: ApprovalKind,
    pub summary: String,
    pub decision: Decision,
    pub source: DecisionSource,
    pub decided_at_ms: u64,
    /// How long the owner (or the deadline) took to produce the verdict.
    pub waited_ms: u64,
    /// Decisions the runtime advertised, so a protocol change is visible.
    pub advertised: Vec<String>,
}

/// Append-only JSONL log of approval decisions.
///
/// Append-only because the point of the record is to be trustworthy after the
/// fact: an audit trail that can be rewritten in place proves nothing.
pub struct ApprovalLog {
    path: PathBuf,
}

impl ApprovalLog {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Appends one record, creating parent directories on first use.
    pub fn append(&self, record: &ApprovalRecord) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let line = serde_json::to_string(record)?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        use std::io::Write;
        writeln!(file, "{line}")
    }

    /// Reads every record, failing on the first malformed line.
    ///
    /// Used by verification: the log is evidence, so silently skipping a line it
    /// cannot parse would make the evidence unreliable.
    pub fn read_all(path: impl AsRef<Path>) -> std::io::Result<Vec<ApprovalRecord>> {
        let contents = std::fs::read_to_string(path)?;
        let mut records = VecDeque::new();
        for (index, line) in contents.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let record = serde_json::from_str(line).map_err(|error| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("line {}: {error}", index + 1),
                )
            })?;
            records.push_back(record);
        }
        Ok(records.into())
    }
}

/// Extracts a one-line owner-facing summary from an approval request.
pub fn summarise(kind: ApprovalKind, params: &Value) -> String {
    match kind {
        ApprovalKind::CommandExecution | ApprovalKind::ExecCommand => {
            if let Some(command) = params.get("command").and_then(Value::as_str) {
                return command.to_string();
            }
            // v1 carries the command as an argument vector.
            if let Some(parts) = params.get("command").and_then(Value::as_array) {
                let joined = parts
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" ");
                if !joined.is_empty() {
                    return joined;
                }
            }
            "<unreadable command>".to_string()
        }
        ApprovalKind::FileChange | ApprovalKind::ApplyPatch => {
            let count = params
                .get("fileChanges")
                .and_then(Value::as_object)
                .map_or(0, serde_json::Map::len);
            format!("{count} file change(s)")
        }
        ApprovalKind::Permissions => {
            let reason = params
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("additional permissions requested");
            format!("permission request: {reason}")
        }
    }
}

/// The decisions the runtime advertised, if it populated them.
pub fn advertised_decisions(params: &Value) -> Vec<String> {
    params
        .get("availableDecisions")
        .and_then(Value::as_array)
        .map(|decisions| {
            decisions
                .iter()
                .map(|decision| {
                    decision
                        .as_str()
                        .map(str::to_string)
                        // Compound decisions (policy amendments) have no
                        // single string form; name the variant instead.
                        .unwrap_or_else(|| {
                            decision
                                .as_object()
                                .and_then(|object| object.keys().next().cloned())
                                .unwrap_or_else(|| "<opaque>".to_string())
                        })
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_every_approval_method_and_nothing_else() {
        assert_eq!(
            ApprovalKind::from_method("item/commandExecution/requestApproval"),
            Some(ApprovalKind::CommandExecution)
        );
        assert_eq!(
            ApprovalKind::from_method("item/fileChange/requestApproval"),
            Some(ApprovalKind::FileChange)
        );
        assert_eq!(
            ApprovalKind::from_method("item/permissions/requestApproval"),
            Some(ApprovalKind::Permissions)
        );
        assert_eq!(
            ApprovalKind::from_method("execCommandApproval"),
            Some(ApprovalKind::ExecCommand)
        );
        assert_eq!(
            ApprovalKind::from_method("applyPatchApproval"),
            Some(ApprovalKind::ApplyPatch)
        );
        // Not approvals: must not be routed to the owner.
        assert_eq!(ApprovalKind::from_method("item/tool/call"), None);
        assert_eq!(
            ApprovalKind::from_method("item/tool/requestUserInput"),
            None
        );
    }

    #[test]
    fn each_kind_maps_to_the_vocabulary_of_its_protocol_generation() {
        // Documents, executably, which generation each method belongs to: the
        // v2 `item/…` methods speak accept/decline, the v1 methods speak
        // approved/denied. Both directions are asserted so a swapped mapping
        // cannot pass.
        for (method, allow, deny) in [
            ("item/commandExecution/requestApproval", "accept", "decline"),
            ("item/fileChange/requestApproval", "accept", "decline"),
            ("execCommandApproval", "approved", "denied"),
            ("applyPatchApproval", "approved", "denied"),
        ] {
            let kind = ApprovalKind::from_method(method).expect("known method");
            for (decision, expected) in [(Decision::Allow, allow), (Decision::Deny, deny)] {
                let token = response_payload(kind, decision, &json!({})).expect("representable")
                    ["decision"]
                    .as_str()
                    .expect("token is a string")
                    .to_string();
                assert_eq!(
                    token, expected,
                    "{method} must answer {decision:?} with `{expected}`"
                );
            }
        }
    }

    /// The whole point of the gate: only these two verdicts exist. Anything that
    /// would widen future authority has no representation, so it cannot be sent
    /// by accident.
    #[test]
    fn approval_is_single_action_and_never_standing() {
        for kind in [
            ApprovalKind::CommandExecution,
            ApprovalKind::FileChange,
            ApprovalKind::ExecCommand,
            ApprovalKind::ApplyPatch,
        ] {
            for decision in [Decision::Allow, Decision::Deny] {
                let payload = response_payload(kind, decision, &json!({})).expect("representable");
                let rendered = payload.to_string();
                for widening in [
                    "acceptForSession",
                    "approved_for_session",
                    "execpolicy",
                    "networkPolicy",
                    "network_policy",
                ] {
                    assert!(
                        !rendered.contains(widening),
                        "{kind:?}/{decision:?} must not carry `{widening}`: {rendered}"
                    );
                }
            }
        }
    }

    #[test]
    fn denying_permissions_grants_nothing() {
        // An empty profile is a refusal, not a grant.
        assert_eq!(
            response_payload(
                ApprovalKind::Permissions,
                Decision::Deny,
                &json!({ "permissions": { "network": { "enabled": true } } })
            )
            .expect("representable"),
            json!({ "permissions": {} })
        );
    }

    #[test]
    fn allowing_permissions_echoes_only_what_was_requested() {
        let requested = json!({ "fileSystem": { "entries": [] } });
        let payload = response_payload(
            ApprovalKind::Permissions,
            Decision::Allow,
            &json!({ "permissions": requested }),
        )
        .expect("representable");
        assert_eq!(payload, json!({ "permissions": requested }));
    }

    #[test]
    fn allowing_permissions_without_a_requested_profile_is_refused_not_guessed() {
        // No profile to echo means we cannot express consent; reporting that is
        // safer than inventing a grant.
        let error = response_payload(ApprovalKind::Permissions, Decision::Allow, &json!({}))
            .expect_err("cannot be represented");
        assert!(matches!(error, ApprovalError::Unrepresentable { .. }));
    }

    #[test]
    fn summary_of_a_command_is_the_command_line() {
        let params = json!({ "command": "/bin/zsh -c 'echo hello'" });
        assert_eq!(
            summarise(ApprovalKind::CommandExecution, &params),
            "/bin/zsh -c 'echo hello'"
        );
    }

    #[test]
    fn summary_of_a_legacy_command_joins_the_argument_vector() {
        let params = json!({ "command": ["/bin/zsh", "-c", "echo hello"] });
        assert_eq!(
            summarise(ApprovalKind::ExecCommand, &params),
            "/bin/zsh -c echo hello"
        );
    }

    #[test]
    fn summary_of_a_patch_counts_the_files() {
        let params = json!({ "fileChanges": { "a.rs": [], "b.rs": [] } });
        assert_eq!(
            summarise(ApprovalKind::ApplyPatch, &params),
            "2 file change(s)"
        );
    }

    #[test]
    fn summary_of_an_unreadable_command_says_so() {
        // Better an explicit placeholder than an empty summary the owner might
        // read as "nothing to approve".
        assert_eq!(
            summarise(ApprovalKind::CommandExecution, &json!({})),
            "<unreadable command>"
        );
    }

    #[test]
    fn advertised_decisions_handle_compound_entries_and_absence() {
        let params = json!({
            "availableDecisions": [
                "accept",
                { "acceptWithExecpolicyAmendment": { "execpolicy_amendment": ["echo"] } },
                "cancel"
            ]
        });
        assert_eq!(
            advertised_decisions(&params),
            vec![
                "accept".to_string(),
                "acceptWithExecpolicyAmendment".to_string(),
                "cancel".to_string()
            ]
        );
        // The field is optional; absence must not be an error.
        assert!(advertised_decisions(&json!({})).is_empty());
    }

    #[test]
    fn the_default_decider_refuses() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let request = ApprovalRequest {
            id: 1,
            method: "item/commandExecution/requestApproval".to_string(),
            kind: ApprovalKind::CommandExecution,
            summary: "rm -rf /".to_string(),
            details: json!({}),
            advertised: Vec::new(),
        };
        let decision = runtime.block_on(DenyAll.decide(&request));
        assert_eq!(decision, Decision::Deny);
    }

    #[test]
    fn the_log_appends_and_reads_back_verbatim() {
        let dir = std::env::temp_dir().join(format!("taoli-approval-{}", std::process::id()));
        let path = dir.join("approvals.ndjson");
        let _ = std::fs::remove_dir_all(&dir);
        let log = ApprovalLog::new(&path);

        let first = ApprovalRecord {
            request_id: 1,
            method: "item/commandExecution/requestApproval".to_string(),
            kind: ApprovalKind::CommandExecution,
            summary: "echo hello".to_string(),
            decision: Decision::Deny,
            source: DecisionSource::Decider,
            decided_at_ms: 1_700_000_000_000,
            waited_ms: 12,
            advertised: vec!["accept".to_string(), "cancel".to_string()],
        };
        let mut second = first.clone();
        second.request_id = 2;
        second.decision = Decision::Allow;
        second.source = DecisionSource::Timeout;

        log.append(&first).expect("append first");
        log.append(&second).expect("append second");

        let read = ApprovalLog::read_all(&path).expect("read back");
        assert_eq!(read, vec![first, second], "round trip must be exact");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupt_log_line_is_an_error_not_a_silent_skip() {
        let dir = std::env::temp_dir().join(format!("taoli-approval-bad-{}", std::process::id()));
        let path = dir.join("approvals.ndjson");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(&path, "{\"not\":\"a record\"}\n").expect("write");

        let error = ApprovalLog::read_all(&path).expect_err("malformed line must fail");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("line 1"), "{error}");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
