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
    /// The verdicts this request can be answered with, in offer order.
    ///
    /// The owner channel renders these; see [`available_decisions`].
    pub options: Vec<Decision>,
}

/// The owner's verdict.
///
/// These four mirror the Codex desktop app's approval card, which offers
/// **允许一次 / 允许此对话 / 始终允许 / 拒绝**
/// (`approvalRequestCard.{allowOnce,allowConversation,alwaysAllow,deny}` in its
/// bundled `zh-CN` locale). Parity is deliberate: an owner moving between the
/// app and this page sees the same choices, and the wire vocabulary behind each
/// one is the runtime's own (`accept` / `acceptForSession` /
/// `acceptWithExecpolicyAmendment` / `decline`, verified against the live
/// runtime).
///
/// The distinction between them is *scope of the grant*, so the wording matters:
///
/// * `AllowOnce` — this action, now. Nothing is remembered.
/// * `AllowForSession` — this action, and equivalent ones for the rest of this
///   session, without prompting again.
/// * `AllowAlways` — this action, and equivalent ones permanently, by adding a
///   rule to the runtime's execpolicy. Only offered when the runtime proposed a
///   concrete rule to add.
/// * `Deny` — this action, now. Nothing is remembered.
///
/// The default when nothing is wired, and when a verdict does not arrive, is
/// [`Decision::Deny`]: a session that cannot ask must not assume consent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// 允许一次 — this action, nothing remembered.
    AllowOnce,
    /// 允许此对话 — this action and equivalents, for this session.
    AllowForSession,
    /// 始终允许 — this action and equivalents, permanently, via a proposed rule.
    AllowAlways,
    /// 拒绝 — refuse this action.
    Deny,
}

impl Decision {
    /// Whether this verdict grants anything.
    pub fn is_grant(self) -> bool {
        !matches!(self, Self::Deny)
    }
}

/// Whether a permanent ("始终允许") verdict can mean anything here.
///
/// It cannot when the runtime is prevented from writing its rule store: the
/// verdict is then accepted and **silently does nothing** — measured, the next
/// identical command is gated again. Offering it anyway would be a button whose
/// label promises more than it can deliver, so it is not offered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Persistence {
    /// The runtime's rule store is writable; a permanent verdict persists.
    Allowed,
    /// The rule store is confined; permanent verdicts have no lasting effect.
    Blocked,
}

/// The verdicts this request can actually be answered with.
///
/// Computed from the request rather than taken from the runtime's
/// `availableDecisions`: that field is optional, was observed to omit `decline`
/// (which the runtime nonetheless accepts), and varies per kind. Deciding here
/// keeps one place that knows what each kind supports, and keeps the interface
/// from offering a button whose answer cannot be expressed — or, in the case of
/// [`Persistence::Blocked`], whose answer cannot take effect.
pub fn available_decisions(
    kind: ApprovalKind,
    params: &Value,
    persistence: Persistence,
) -> Vec<Decision> {
    let mut decisions = vec![Decision::AllowOnce];
    if supports_session_scope(kind) {
        decisions.push(Decision::AllowForSession);
    }
    if persistence == Persistence::Allowed && proposed_amendment(kind, params).is_some() {
        decisions.push(Decision::AllowAlways);
    }
    decisions.push(Decision::Deny);
    decisions
}

/// Whether the kind's wire vocabulary has a session-scoped variant.
fn supports_session_scope(kind: ApprovalKind) -> bool {
    // Every kind does: `acceptForSession`, `approved_for_session`, and for
    // permissions the `scope: "session"` field.
    matches!(
        kind,
        ApprovalKind::CommandExecution
            | ApprovalKind::FileChange
            | ApprovalKind::Permissions
            | ApprovalKind::ExecCommand
            | ApprovalKind::ApplyPatch
    )
}

/// The rule the runtime proposed adding, when it proposed one.
///
/// Only the persistent ("始终允许") verdict needs this, and only some kinds
/// carry it: the v2 command-execution request and the legacy pair. Without a
/// concrete proposal there is nothing to remember, so the option is not
/// offered — an "always allow" that remembered nothing would be a lie.
fn proposed_amendment(kind: ApprovalKind, params: &Value) -> Option<Vec<String>> {
    let field = match kind {
        ApprovalKind::CommandExecution | ApprovalKind::ExecCommand | ApprovalKind::ApplyPatch => {
            "proposedExecpolicyAmendment"
        }
        // Neither the file-change nor the permissions request carries a rule
        // proposal in this protocol version.
        ApprovalKind::FileChange | ApprovalKind::Permissions => return None,
    };
    let proposal = params.get(field)?.as_array()?;
    let parts: Vec<String> = proposal
        .iter()
        .filter_map(|part| part.as_str().map(str::to_string))
        .collect();
    if parts.is_empty() { None } else { Some(parts) }
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
    /// The request cannot be answered with this verdict: either the kind has no
    /// vocabulary for it (a permanent grant on a file change), or the runtime
    /// proposed nothing to remember.
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
                "cannot express {decision:?} for a {kind:?} approval: the protocol has no \
                 vocabulary for it, or the runtime proposed nothing to remember"
            ),
        }
    }
}

impl std::error::Error for ApprovalError {}

/// Builds the response payload for `kind` given `decision`.
///
/// Each verdict maps to the runtime's own vocabulary for that kind. Two details
/// are worth stating because getting them wrong fails silently at the protocol
/// layer:
///
/// * The **legacy** vocabulary expresses refusal as an object
///   (`{"denied":{"rejection": …}}`), not the string `"denied"` — the schema
///   requires the `rejection` field.
/// * The **persistent** verdict must echo the rule the runtime proposed; there
///   is no way to invent one, so its absence is reported rather than papered
///   over.
pub fn response_payload(
    kind: ApprovalKind,
    decision: Decision,
    request_details: &Value,
) -> Result<Value, ApprovalError> {
    let amendment = || {
        proposed_amendment(kind, request_details)
            .ok_or(ApprovalError::Unrepresentable { kind, decision })
    };

    match kind {
        // v2 command execution.
        ApprovalKind::CommandExecution => match decision {
            Decision::AllowOnce => Ok(json!({ "decision": "accept" })),
            Decision::AllowForSession => Ok(json!({ "decision": "acceptForSession" })),
            Decision::AllowAlways => Ok(json!({
                "decision": {
                    "acceptWithExecpolicyAmendment": { "execpolicy_amendment": amendment()? }
                }
            })),
            Decision::Deny => Ok(json!({ "decision": "decline" })),
        },
        // v2 file change: the type has no amendment variant.
        ApprovalKind::FileChange => match decision {
            Decision::AllowOnce => Ok(json!({ "decision": "accept" })),
            Decision::AllowForSession => Ok(json!({ "decision": "acceptForSession" })),
            Decision::AllowAlways => Err(ApprovalError::Unrepresentable { kind, decision }),
            Decision::Deny => Ok(json!({ "decision": "decline" })),
        },
        // Legacy pair, whose refusal is an object and whose session variant has
        // its own token.
        ApprovalKind::ExecCommand | ApprovalKind::ApplyPatch => match decision {
            Decision::AllowOnce => Ok(json!({ "decision": "approved" })),
            Decision::AllowForSession => Ok(json!({ "decision": "approved_for_session" })),
            Decision::AllowAlways => Ok(json!({
                "decision": {
                    "approved_execpolicy_amendment": {
                        "proposed_execpolicy_amendment": amendment()?
                    }
                }
            })),
            Decision::Deny => Ok(json!({
                "decision": { "denied": { "rejection": "declined by the operator" } }
            })),
        },
        // Permissions: consent is the requested profile, scope is how long it
        // lasts. An empty profile is the refusal.
        ApprovalKind::Permissions => match decision {
            Decision::Deny => Ok(json!({ "permissions": {} })),
            Decision::AllowOnce | Decision::AllowForSession => {
                let requested = request_details.get("permissions").cloned();
                let Some(permissions) = requested.filter(|value| !value.is_null()) else {
                    // Nothing was requested, so there is nothing to grant; the
                    // caller must not have this option.
                    return Err(ApprovalError::Unrepresentable { kind, decision });
                };
                let mut payload = json!({ "permissions": permissions });
                if decision == Decision::AllowForSession {
                    payload["scope"] = Value::String("session".to_string());
                }
                Ok(payload)
            }
            // Widening permissions permanently is a policy change that this
            // protocol offers no field for.
            Decision::AllowAlways => Err(ApprovalError::Unrepresentable { kind, decision }),
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
        // v2 `item/…` methods speak accept/decline; the v1 pair speaks
        // approved, and refuses with an **object** rather than a token.
        for (method, allow) in [
            ("item/commandExecution/requestApproval", "accept"),
            ("item/fileChange/requestApproval", "accept"),
            ("execCommandApproval", "approved"),
            ("applyPatchApproval", "approved"),
        ] {
            let kind = ApprovalKind::from_method(method).expect("known method");
            let token = response_payload(kind, Decision::AllowOnce, &json!({}))
                .expect("representable")["decision"]
                .as_str()
                .expect("the affirmative is a token")
                .to_string();
            assert_eq!(
                token, allow,
                "{method} must answer its affirmative with `{allow}`"
            );
        }

        // Refusals, per generation.
        for (method, expected) in [
            ("item/commandExecution/requestApproval", json!("decline")),
            ("item/fileChange/requestApproval", json!("decline")),
            (
                "execCommandApproval",
                json!({ "denied": { "rejection": "declined by the operator" } }),
            ),
            (
                "applyPatchApproval",
                json!({ "denied": { "rejection": "declined by the operator" } }),
            ),
        ] {
            let kind = ApprovalKind::from_method(method).expect("known method");
            let payload =
                response_payload(kind, Decision::Deny, &json!({})).expect("representable");
            assert_eq!(payload, json!({ "decision": expected }), "{method}");
        }
    }

    /// The three affirmatives are distinguishable on the wire: a caller cannot
    /// accidentally send "forever" when it meant "once".
    #[test]
    fn each_verdict_maps_to_its_own_wire_form() {
        let params = json!({ "proposedExecpolicyAmendment": ["echo", "hi"] });

        let once = response_payload(ApprovalKind::CommandExecution, Decision::AllowOnce, &params)
            .expect("representable");
        assert_eq!(once, json!({ "decision": "accept" }));

        let session = response_payload(
            ApprovalKind::CommandExecution,
            Decision::AllowForSession,
            &params,
        )
        .expect("representable");
        assert_eq!(session, json!({ "decision": "acceptForSession" }));

        let always = response_payload(
            ApprovalKind::CommandExecution,
            Decision::AllowAlways,
            &params,
        )
        .expect("representable");
        assert_eq!(
            always,
            json!({ "decision": { "acceptWithExecpolicyAmendment": {
                "execpolicy_amendment": ["echo", "hi"] } } })
        );

        // All four render differently; a mapping that collapsed two of them
        // would be a silent privilege change.
        let rendered: Vec<String> = [
            Decision::AllowOnce,
            Decision::AllowForSession,
            Decision::AllowAlways,
            Decision::Deny,
        ]
        .iter()
        .map(|d| {
            response_payload(ApprovalKind::CommandExecution, *d, &params)
                .unwrap()
                .to_string()
        })
        .collect();
        let mut unique = rendered.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(
            unique.len(),
            rendered.len(),
            "verdicts collapsed: {rendered:?}"
        );
    }

    /// The persistent verdict needs a concrete rule to add. Without one it must
    /// be reported, not silently turned into a weaker grant.
    #[test]
    fn a_permanent_grant_without_a_proposed_rule_is_refused_not_downgraded() {
        let error = response_payload(
            ApprovalKind::CommandExecution,
            Decision::AllowAlways,
            &json!({}),
        )
        .expect_err("nothing to remember");
        assert!(
            matches!(error, ApprovalError::Unrepresentable { .. }),
            "{error:?}"
        );
        // And it must not be offered either.
        assert!(
            !available_decisions(
                ApprovalKind::CommandExecution,
                &json!({}),
                Persistence::Allowed
            )
            .contains(&Decision::AllowAlways)
        );
    }

    /// A file change has no amendment in its vocabulary, so the option is not
    /// offered and not representable.
    #[test]
    fn kinds_without_a_rule_proposal_do_not_offer_a_permanent_grant() {
        for kind in [ApprovalKind::FileChange, ApprovalKind::Permissions] {
            let options = available_decisions(
                kind,
                &json!({ "proposedExecpolicyAmendment": ["echo"] }),
                Persistence::Allowed,
            );
            assert!(
                !options.contains(&Decision::AllowAlways),
                "{kind:?} must not offer 始终允许: {options:?}"
            );
        }
    }

    /// Everything offered must be answerable — an offered button that cannot be
    /// expressed is a dead end for the operator.
    #[test]
    fn every_offered_verdict_is_representable() {
        let cases = [
            (
                ApprovalKind::CommandExecution,
                json!({ "proposedExecpolicyAmendment": ["echo"] }),
            ),
            (ApprovalKind::CommandExecution, json!({})),
            (ApprovalKind::FileChange, json!({})),
            (
                ApprovalKind::Permissions,
                json!({ "permissions": { "network": { "enabled": true } } }),
            ),
            (
                ApprovalKind::ExecCommand,
                json!({ "proposedExecpolicyAmendment": ["ls"] }),
            ),
            (ApprovalKind::ApplyPatch, json!({})),
        ];
        for (kind, params) in cases {
            for decision in available_decisions(kind, &params, Persistence::Allowed) {
                assert!(
                    response_payload(kind, decision, &params).is_ok(),
                    "{kind:?} offers {decision:?} but cannot express it"
                );
            }
            // And every offered set ends with the refusal, so the operator can
            // always say no.
            let options = available_decisions(kind, &params, Persistence::Allowed);
            assert_eq!(
                options.last(),
                Some(&Decision::Deny),
                "{kind:?}: {options:?}"
            );
            assert!(
                options.contains(&Decision::AllowOnce),
                "{kind:?}: {options:?}"
            );
        }
    }

    /// The legacy refusal is an object, not the string `"denied"`: the schema
    /// requires `rejection`, so the string form would be rejected outright.
    #[test]
    fn the_legacy_refusal_carries_its_rejection_field() {
        let payload = response_payload(ApprovalKind::ExecCommand, Decision::Deny, &json!({}))
            .expect("representable");
        assert_eq!(
            payload,
            json!({ "decision": { "denied": { "rejection": "declined by the operator" } } })
        );
        assert!(payload["decision"]["denied"]["rejection"].is_string());
    }

    /// Session scope is expressed differently per kind, and must survive.
    #[test]
    fn session_scope_is_expressed_per_kind() {
        assert_eq!(
            response_payload(
                ApprovalKind::ExecCommand,
                Decision::AllowForSession,
                &json!({})
            )
            .expect("representable"),
            json!({ "decision": "approved_for_session" })
        );
        assert_eq!(
            response_payload(
                ApprovalKind::FileChange,
                Decision::AllowForSession,
                &json!({})
            )
            .expect("representable"),
            json!({ "decision": "acceptForSession" })
        );
        // Permissions carry the scope as a sibling field.
        let payload = response_payload(
            ApprovalKind::Permissions,
            Decision::AllowForSession,
            &json!({ "permissions": { "network": { "enabled": true } } }),
        )
        .expect("representable");
        assert_eq!(payload["scope"], json!("session"));
        assert_eq!(payload["permissions"]["network"]["enabled"], json!(true));
    }

    #[test]
    fn allowing_permissions_echoes_only_what_was_requested() {
        let requested = json!({ "fileSystem": { "entries": [] } });
        let payload = response_payload(
            ApprovalKind::Permissions,
            Decision::AllowOnce,
            &json!({ "permissions": requested }),
        )
        .expect("representable");
        assert_eq!(payload, json!({ "permissions": requested }));
    }

    #[test]
    fn allowing_permissions_without_a_requested_profile_is_refused_not_guessed() {
        // No profile to echo means we cannot express consent; reporting that is
        // safer than inventing a grant.
        let error = response_payload(ApprovalKind::Permissions, Decision::AllowOnce, &json!({}))
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
            options: available_decisions(
                ApprovalKind::CommandExecution,
                &json!({}),
                Persistence::Allowed,
            ),
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
        second.decision = Decision::AllowForSession;
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

    /// Regression: the legacy refusal is an object, and `"denied"` as a bare
    /// string is **not** a valid `ReviewDecision` — the schema requires the
    /// `denied.rejection` field. Sending the string form would be rejected at
    /// the protocol layer, i.e. the operator's "refuse" would fail open.
    #[test]
    fn the_legacy_refusal_is_never_the_bare_string() {
        for method in ["execCommandApproval", "applyPatchApproval"] {
            let kind = ApprovalKind::from_method(method).expect("known method");
            let payload =
                response_payload(kind, Decision::Deny, &json!({})).expect("representable");
            assert_ne!(
                payload["decision"],
                json!("denied"),
                "{method} must not send the bare string"
            );
            assert!(
                payload["decision"]["denied"]["rejection"].is_string(),
                "{method} must carry a rejection: {payload}"
            );
        }
    }

    /// A permanent verdict must not be offered when it cannot take effect.
    ///
    /// Measured: with the rule store confined, `acceptWithExecpolicyAmendment`
    /// is accepted and then does nothing — a second identical command is gated
    /// again. Offering it would promise permanence the session cannot deliver.
    #[test]
    fn the_permanent_grant_is_not_offered_when_persistence_is_blocked() {
        let params = json!({ "proposedExecpolicyAmendment": ["echo", "hi"] });
        for kind in [
            ApprovalKind::CommandExecution,
            ApprovalKind::ExecCommand,
            ApprovalKind::ApplyPatch,
        ] {
            let blocked = available_decisions(kind, &params, Persistence::Blocked);
            assert!(
                !blocked.contains(&Decision::AllowAlways),
                "{kind:?} offered 始终允许 while persistence is confined: {blocked:?}"
            );
            assert!(
                blocked.contains(&Decision::AllowOnce),
                "{kind:?}: {blocked:?}"
            );
            assert!(
                blocked.contains(&Decision::AllowForSession),
                "{kind:?}: {blocked:?}"
            );
            assert_eq!(
                blocked.last(),
                Some(&Decision::Deny),
                "{kind:?}: {blocked:?}"
            );

            // And nothing left the door open: the option is the only route, so
            // the request cannot be answered permanently either.
            let allowed = available_decisions(kind, &params, Persistence::Allowed);
            assert!(allowed.contains(&Decision::AllowAlways), "{kind:?}");
        }
    }
}
