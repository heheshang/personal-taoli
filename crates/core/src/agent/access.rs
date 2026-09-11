//! How much the agent may do without asking (PORT-01-L).
//!
//! Three levels, matching the Codex desktop app's permissions dropdown
//! (“应如何批准 Codex 操作？”). Each level is a **combination** of settings, not
//! a single knob: the app's presets are defined in its own source as
//! `(approval policy, permission profile)` pairs, and the reviewer — who gets
//! asked, if anyone — is a third field. Modelling the level as three
//! independent knobs would let a caller build combinations the app never offers
//! and this project has not reasoned about, so the level *is* the unit here.
//!
//! The app's own labels and descriptions are reused verbatim so the two
//! interfaces describe the same thing in the same words:
//!
//! | Level | App label | App description |
//! |---|---|---|
//! | [`AccessLevel::Ask`] | 请求批准 | 编辑外部文件和使用互联网时始终询问 |
//! | [`AccessLevel::AutoApprove`] | 帮我批准 | 仅对检测到的风险操作请求批准 |
//! | [`AccessLevel::FullAccess`] | 完全访问权限 | 可不受限制地访问互联网和你电脑上的任何文件 |
//!
//! What each level means **here** is the conjunction of two layers: what the
//! runtime is told, and what this project's own seatbelt confinement does. Both
//! must agree, or the label lies:
//!
//! | Level | Runtime (`sandbox` / `approvalPolicy` / `approvalsReviewer`) | Our confinement |
//! |---|---|---|
//! | `Ask` | `read-only` / `untrusted` / `user` | writable: runtime home, trace dir, explicitly configured roots |
//! | `AutoApprove` | `workspace-write` / `on-request` / `auto_review` | as above **plus the workspace** |
//! | `FullAccess` | `danger-full-access` / `never` / `user` | **none** — any confinement would contradict the label |
//!
//! Two consequences worth stating plainly, because they are the kind of thing a
//! label can hide:
//!
//! * Under `AutoApprove` the **runtime's own subagent decides**, so this
//!   project's [`super::approval::ApprovalDecider`] is not consulted. The
//!   interface shows that level's description rather than pretending the owner
//!   still reviews each action.
//! * `FullAccess` removes the confinement entirely: the agent may write anywhere
//!   and reach the network without asking. It is offered because the owner asked
//!   for parity with the app, and it says so on the tin.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::protocol::ApprovalPolicy;
use super::sandbox::{self, SandboxPolicy};
use super::trace::SandboxMode;

/// Who reviews an approval request.
///
/// Mirrors the runtime's `ApprovalsReviewer`. `AutoReview` makes the runtime ask
/// a carefully prompted subagent instead of the client, which is what the app's
/// “帮我批准” does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalsReviewer {
    /// Requests come to this client, which puts them to the owner.
    ///
    /// The default because it is the only choice that keeps a human in the loop:
    /// defaulting to auto-review would let a missing field quietly remove the
    /// gate.
    #[default]
    User,
    /// The runtime reviews risk itself; this client is not asked.
    AutoReview,
}

impl ApprovalsReviewer {
    pub fn as_wire(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::AutoReview => "auto_review",
        }
    }
}

/// How much the agent may do without asking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AccessLevel {
    /// 请求批准 — edit files outside the workspace or use the network only with
    /// approval. The project's default, and the only level that keeps the
    /// original read-only boundary.
    #[default]
    Ask,
    /// 帮我批准 — the runtime approves the low-risk actions itself and asks only
    /// about what its risk framework flags.
    AutoApprove,
    /// 完全访问权限 — unrestricted files and network, no approvals.
    FullAccess,
}

impl AccessLevel {
    pub const ALL: [Self; 3] = [Self::Ask, Self::AutoApprove, Self::FullAccess];

    /// The label the app uses, so both interfaces say the same thing.
    pub fn label(self) -> &'static str {
        match self {
            Self::Ask => "请求批准",
            Self::AutoApprove => "帮我批准",
            Self::FullAccess => "完全访问权限",
        }
    }

    /// The app's one-line description for this level.
    pub fn description(self) -> &'static str {
        match self {
            Self::Ask => "编辑外部文件和使用互联网时始终询问",
            Self::AutoApprove => "仅对检测到的风险操作请求批准",
            Self::FullAccess => "可不受限制地访问互联网和你电脑上的任何文件",
        }
    }

    /// The stable token used between the interface and this crate.
    pub fn token(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::AutoApprove => "auto_approve",
            Self::FullAccess => "full_access",
        }
    }

    pub fn parse(token: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|level| level.token() == token)
    }

    /// What the runtime is told about sandboxing.
    pub fn sandbox(self) -> SandboxMode {
        match self {
            Self::Ask => SandboxMode::ReadOnly,
            Self::AutoApprove => SandboxMode::WorkspaceWrite,
            Self::FullAccess => SandboxMode::DangerFullAccess,
        }
    }

    /// When the runtime asks at all.
    ///
    /// `untrusted` for `Ask` rather than the runtime's own `on-request` default:
    /// `on-request` lets the *model* decide whether to ask, which was measured
    /// to run a harmless-looking command with no prompt at all. The owner asked
    /// to be the gate, so the gate is not left to the model's judgement.
    pub fn approval_policy(self) -> ApprovalPolicy {
        match self {
            Self::Ask => ApprovalPolicy::UnlessTrusted,
            Self::AutoApprove => ApprovalPolicy::OnRequest,
            Self::FullAccess => ApprovalPolicy::Never,
        }
    }

    /// Who reviews what the policy does escalate.
    pub fn reviewer(self) -> ApprovalsReviewer {
        match self {
            Self::Ask | Self::FullAccess => ApprovalsReviewer::User,
            Self::AutoApprove => ApprovalsReviewer::AutoReview,
        }
    }

    /// Whether this level asks this client anything at all.
    ///
    /// `FullAccess` never asks (policy `never`), and `AutoApprove` asks the
    /// runtime's own reviewer, so in neither case does this project's decider
    /// see a request.
    pub fn consults_this_client(self) -> bool {
        self.reviewer() == ApprovalsReviewer::User
            && self.approval_policy() != ApprovalPolicy::Never
    }

    /// This project's confinement for the level, or `None` for no confinement.
    ///
    /// `FullAccess` returns `None` deliberately: applying any profile would make
    /// the level's description ("可不受限制地…") false. Dropping the confinement
    /// is the honest implementation of the label, and the level's own
    /// description is what the interface shows next to it.
    ///
    /// `workspace` is what `AutoApprove` gains write access to — the project
    /// root, since that is the directory the agent's work lives in.
    pub fn confinement(
        self,
        codex_home: &Path,
        trace_dir: Option<&Path>,
        workspace: Option<&Path>,
    ) -> Option<SandboxPolicy> {
        match self {
            Self::Ask => Some(sandbox::codex_child_policy(codex_home, trace_dir)),
            Self::AutoApprove => {
                let mut policy = sandbox::codex_child_policy(codex_home, trace_dir);
                if let Some(workspace) = workspace {
                    policy.writable_roots.push(workspace.to_path_buf());
                }
                Some(policy)
            }
            Self::FullAccess => None,
        }
    }
}

/// Where the level is kept for a session.
///
/// A session-level setting, like the app's per-composer dropdown: it is applied
/// when a thread starts and cannot be changed under a running one, because the
/// runtime takes these values at `thread/start` and a sandbox cannot be
/// retrofitted onto a running process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AccessSetting {
    pub level: AccessLevel,
}

impl AccessSetting {
    pub fn new(level: AccessLevel) -> Self {
        Self { level }
    }

    /// Whether `candidate` may be adopted while a session is running.
    ///
    /// Only the level already in force may be "changed" under a live session;
    /// anything else must wait for a new one.
    pub fn accepts_change_to(self, candidate: AccessLevel) -> bool {
        self.level == candidate
    }
}

/// Extra roots a level grants write access to beyond its own rule.
///
/// Kept separate from [`AccessLevel::confinement`] so the command layer can add
/// operator-configured roots without re-deriving the level's own decision.
pub fn with_extra_write_roots(
    policy: &mut SandboxPolicy,
    roots: impl IntoIterator<Item = PathBuf>,
) {
    policy.writable_roots.extend(roots);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_level_round_trips_through_its_token() {
        let mut tokens: Vec<&str> = AccessLevel::ALL.iter().map(|level| level.token()).collect();
        for level in AccessLevel::ALL {
            assert_eq!(AccessLevel::parse(level.token()), Some(level));
        }
        tokens.sort_unstable();
        tokens.dedup();
        assert_eq!(tokens.len(), 3, "tokens must be distinct");
        assert_eq!(AccessLevel::parse("everything"), None);
        assert_eq!(AccessLevel::parse("Ask"), None, "tokens are exact");
    }

    /// The labels are the app's, verbatim: two interfaces describing the same
    /// setting must not drift into different words.
    #[test]
    fn the_labels_are_the_ones_the_app_uses() {
        assert_eq!(AccessLevel::Ask.label(), "请求批准");
        assert_eq!(AccessLevel::AutoApprove.label(), "帮我批准");
        assert_eq!(AccessLevel::FullAccess.label(), "完全访问权限");
        for level in AccessLevel::ALL {
            assert!(!level.description().is_empty(), "{level:?}");
        }
    }

    /// Each level is a coherent triple, in the app's own mapping.
    #[test]
    fn each_level_maps_to_the_runtimes_own_settings() {
        // 请求批准: read-only sandbox, and the gate belongs to the owner.
        assert_eq!(AccessLevel::Ask.sandbox().as_wire(), "read-only");
        assert_eq!(AccessLevel::Ask.approval_policy().as_wire(), "untrusted");
        assert_eq!(AccessLevel::Ask.reviewer().as_wire(), "user");

        // 帮我批准: workspace-write, the model may ask, the runtime reviews.
        assert_eq!(
            AccessLevel::AutoApprove.sandbox().as_wire(),
            "workspace-write"
        );
        assert_eq!(
            AccessLevel::AutoApprove.approval_policy().as_wire(),
            "on-request"
        );
        assert_eq!(AccessLevel::AutoApprove.reviewer().as_wire(), "auto_review");

        // 完全访问权限: nothing is sandboxed and nothing is asked.
        assert_eq!(
            AccessLevel::FullAccess.sandbox().as_wire(),
            "danger-full-access"
        );
        assert_eq!(AccessLevel::FullAccess.approval_policy().as_wire(), "never");
        assert_eq!(AccessLevel::FullAccess.reviewer().as_wire(), "user");
    }

    /// Only the asking level consults this client; the other two do not, and the
    /// interface must not imply otherwise.
    #[test]
    fn only_the_asking_level_consults_this_client() {
        assert!(AccessLevel::Ask.consults_this_client());
        assert!(
            !AccessLevel::AutoApprove.consults_this_client(),
            "auto_review sends requests to the runtime's reviewer, not to us"
        );
        assert!(!AccessLevel::FullAccess.consults_this_client());
    }

    /// The level's confinement must agree with its label.
    #[test]
    fn confinement_widens_with_the_level_and_vanishes_at_full_access() {
        let home = Path::new("/tmp/taoli-home");
        let trace = Path::new("/tmp/taoli-traces");
        let workspace = Path::new("/tmp/taoli-workspace");

        let ask = AccessLevel::Ask
            .confinement(home, Some(trace), Some(workspace))
            .expect("the asking level is confined");
        assert!(ask.writable_roots.contains(&home.to_path_buf()));
        assert!(ask.writable_roots.contains(&trace.to_path_buf()));
        assert!(
            !ask.writable_roots.contains(&workspace.to_path_buf()),
            "请求批准 must not grant the workspace: {ask:?}"
        );
        // The rule store stays carved out, so a permanent verdict cannot persist.
        assert!(ask.blocks_rule_persistence());

        let auto = AccessLevel::AutoApprove
            .confinement(home, Some(trace), Some(workspace))
            .expect("confined");
        assert!(
            auto.writable_roots.contains(&workspace.to_path_buf()),
            "帮我批准 grants the workspace: {auto:?}"
        );
        // Still carved out: the K decision holds at every confined level.
        assert!(auto.blocks_rule_persistence());

        // 完全访问权限 applies no confinement at all — anything less would make
        // the level's description untrue.
        assert!(
            AccessLevel::FullAccess
                .confinement(home, Some(trace), Some(workspace))
                .is_none(),
            "full access must not be confined"
        );
    }

    #[test]
    fn auto_approve_without_a_workspace_still_yields_a_policy() {
        let policy = AccessLevel::AutoApprove
            .confinement(Path::new("/tmp/home"), None, None)
            .expect("confined");
        assert_eq!(policy.writable_roots, vec![PathBuf::from("/tmp/home")]);
    }

    #[test]
    fn a_running_session_accepts_only_its_own_level() {
        let setting = AccessSetting::new(AccessLevel::Ask);
        assert!(setting.accepts_change_to(AccessLevel::Ask));
        assert!(
            !setting.accepts_change_to(AccessLevel::FullAccess),
            "a level cannot be retrofitted onto a running session"
        );
    }

    #[test]
    fn extra_write_roots_are_additive() {
        let mut policy = AccessLevel::Ask
            .confinement(Path::new("/tmp/home"), None, None)
            .expect("confined");
        let before = policy.writable_roots.len();
        with_extra_write_roots(&mut policy, [PathBuf::from("/tmp/skill-repo")]);
        assert_eq!(policy.writable_roots.len(), before + 1);
        assert!(
            policy
                .writable_roots
                .contains(&PathBuf::from("/tmp/skill-repo"))
        );
    }
}
