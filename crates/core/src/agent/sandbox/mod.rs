//! Confining the agent runtime with macOS Seatbelt.
//!
//! This is the project's *own* confinement of the codex child process, applied
//! regardless of what the child does about its own sandbox. The point is not to
//! replace codex's sandbox but to stop depending on it: with this in place, the
//! bound on what the child can write comes from a profile this project controls.
//!
//! Two properties are enforced here, and neither is inherited from upstream:
//!
//! * **Availability is probed, never assumed.** Upstream's
//!   `get_platform_sandbox` returns a sandbox type without checking whether it
//!   can actually be applied. It can fail in practice — `sandbox-exec` reports
//!   `sandbox_apply: Operation not permitted` when the process is already inside
//!   a sandbox, and Apple has marked the tool deprecated. A caller that assumed
//!   confinement was in force would be wrong in exactly that case, so
//!   [`probe`] runs the thing and reports what happened.
//! * **Failure to confine is a refusal, not a warning.** [`confine_command`]
//!   returns an error when the mechanism is unusable; there is deliberately no
//!   "run it anyway" path.
//!
//! The policy text itself is copied from upstream and must not be rewritten by
//! hand — see `PROVENANCE.md`.

pub mod seatbelt;

use std::path::{Path, PathBuf};
use std::process::Command;

pub use seatbelt::{AbsolutePathBuf, PathError, SandboxError, SandboxPolicy, sandbox_command};

/// Whether Seatbelt can actually be applied on this machine, right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    /// A confinement of this process was applied and observed to be in force.
    Available,
    /// Not macOS at all.
    UnsupportedPlatform,
    /// The `sandbox-exec` binary is missing.
    MissingTool { path: PathBuf },
    /// The tool exists but refused to apply a profile.
    ///
    /// The usual cause is nesting: a process that is already sandboxed cannot
    /// enter a second Seatbelt.
    CannotApply { detail: String },
}

impl Availability {
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available)
    }
}

impl std::fmt::Display for Availability {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Available => write!(formatter, "seatbelt is available"),
            Self::UnsupportedPlatform => {
                write!(formatter, "seatbelt confinement requires macOS")
            }
            Self::MissingTool { path } => {
                write!(formatter, "{} is missing", path.display())
            }
            Self::CannotApply { detail } => {
                write!(formatter, "seatbelt refused to apply a profile: {detail}")
            }
        }
    }
}

/// Why confinement could not be applied.
#[derive(Debug)]
pub enum ConfineError {
    /// The mechanism is unusable; see [`Availability`].
    Unavailable(Availability),
    /// The profile could not be built.
    Profile(SandboxError),
}

impl std::fmt::Display for ConfineError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(availability) => {
                write!(formatter, "refusing to run unconfined: {availability}")
            }
            Self::Profile(error) => write!(formatter, "cannot build sandbox profile: {error}"),
        }
    }
}

impl std::error::Error for ConfineError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Profile(error) => Some(error),
            Self::Unavailable(_) => None,
        }
    }
}

/// Applies a throwaway confinement and reports what actually happened.
///
/// The probe must *test the mechanism*, not inspect the environment. Three
/// tempting shortcuts all fail:
///
/// * Checking that the binary exists misses the real failure mode: the tool is
///   present and refuses to apply a profile (nesting reports
///   `sandbox_apply: Operation not permitted`).
/// * Running a command under `(deny default)` proves nothing, because a process
///   cannot even reach its dynamic linker under that profile and dies by signal
///   whether or not confinement works.
/// * Probing with a path taken straight from [`std::env::temp_dir`] proves
///   nothing either, and this one is easy to miss: on macOS `/var` and `/tmp`
///   are symlinks, and Seatbelt matches on the **resolved** path. A rule naming
///   `/var/folders/...` never matches anything, so a deny written for the probe
///   file silently permits it and the probe reports a false failure.
///
/// So the probe resolves its paths first, then checks **both** directions: a
/// write inside the granted root must succeed, and a write outside it must not.
/// Only the pair distinguishes "confinement works" from "the profile was never
/// applied" and from "the profile denies everything".
pub fn probe() -> Availability {
    if !cfg!(target_os = "macos") {
        return Availability::UnsupportedPlatform;
    }
    let tool = Path::new(seatbelt::SANDBOX_EXEC_PATH);
    if !tool.is_file() {
        return Availability::MissingTool {
            path: tool.to_path_buf(),
        };
    }

    let (Some(allowed_dir), Some(denied_dir)) = (resolved_temp_dir(), resolved_home_dir()) else {
        return Availability::CannotApply {
            detail: "cannot resolve a temporary or home directory for the probe".to_string(),
        };
    };
    let allowed = allowed_dir.join(format!("taoli-probe-ok-{}", std::process::id()));
    let denied = denied_dir.join(format!("taoli-probe-no-{}", std::process::id()));
    let _ = std::fs::remove_file(&allowed);
    let _ = std::fs::remove_file(&denied);

    // Paths reach the profile as `-D` parameters, so they cannot break out of
    // their own literals.
    let profile = "(version 1)\n\
                   (deny default)\n\
                   (allow file-read*)\n\
                   (allow process-exec)\n\
                   (allow file-write* (subpath (param \"TAOLI_ALLOWED\")))\n";
    let run = |target: &Path| {
        Command::new(tool)
            .args([
                "-p",
                profile,
                &format!("-DTAOLI_ALLOWED={}", allowed_dir.display()),
                "--",
                "/bin/sh",
                "-c",
                r#"echo probe > "$1""#,
                "sh",
                &target.to_string_lossy(),
            ])
            .output()
    };

    let inside = run(&allowed);
    let wrote_inside = allowed.is_file();
    let _ = std::fs::remove_file(&allowed);

    let outside = run(&denied);
    let wrote_outside = denied.is_file();
    let _ = std::fs::remove_file(&denied);

    match (inside, outside) {
        (Err(source), _) => Availability::CannotApply {
            detail: source.to_string(),
        },
        (Ok(_), Err(_)) => Availability::CannotApply {
            detail: "cannot run the probe command outside the sandbox either".to_string(),
        },
        (Ok(_), Ok(_)) => {
            if !wrote_inside {
                Availability::CannotApply {
                    detail: format!(
                        "a write inside the granted root was refused; the profile is too strict \
                         or a path alias was not resolved ({} )",
                        allowed_dir.display()
                    ),
                }
            } else if wrote_outside {
                Availability::CannotApply {
                    detail: format!(
                        "a write outside the granted root reached the filesystem; \
                         confinement is not in force ({})",
                        denied.display()
                    ),
                }
            } else {
                Availability::Available
            }
        }
    }
}

/// [`std::env::temp_dir`] with symlinks resolved.
///
/// Seatbelt matches resolved paths, and on macOS the temporary directory is
/// reached through the `/var` symlink; a rule naming the unresolved path matches
/// nothing.
fn resolved_temp_dir() -> Option<PathBuf> {
    let dir = std::env::temp_dir();
    Some(dir.canonicalize().unwrap_or(dir))
}

/// The home directory with symlinks resolved, for the same reason.
fn resolved_home_dir() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    Some(home.canonicalize().unwrap_or(home))
}

/// Wraps `command` in the confinement described by `policy`, failing closed.
///
/// Returns the argv to spawn (`sandbox-exec -p … -- <command>`) rather than a
/// process handle, so this module does not pick a process API on the caller's
/// behalf — the agent runtime is spawned through tokio, the probe is not.
///
/// The availability check lives here, not with the caller: a caller that had to
/// remember it could forget, and forgetting means running unconfined while
/// believing otherwise. There is deliberately no "wrapped or plain" return.
pub fn confined_argv(
    command: &[String],
    policy: &SandboxPolicy,
) -> Result<Vec<String>, ConfineError> {
    match probe() {
        Availability::Available => {}
        unavailable => return Err(ConfineError::Unavailable(unavailable)),
    }
    sandbox_command(command, policy).map_err(ConfineError::Profile)
}

/// The confinement this project applies to the codex child process.
///
/// Writes are limited to what the runtime demonstrably needs: its own home
/// (session and state storage — measured: the child fails to start without it),
/// the platform scratch directories its runtime expects, and the trace
/// directory. **The workspace is deliberately absent**: this integration's
/// boundary is read-only, so no workspace write is granted.
///
/// One carve-out is intrinsic to the child: `<codex_home>/rules` is denied
/// write access. The runtime persists an "approve forever" verdict there as a
/// `prefix_rule`, which changes behaviour for **every later Codex session on
/// this machine** — outside the scope of one analysis session and outside what
/// this project's confinement is meant to control. Denying the write keeps the
/// rule local to the session that asked for it: the approval still takes effect
/// for the command at hand, it just cannot outlive the process.
pub fn codex_child_policy(codex_home: &Path, trace_dir: Option<&Path>) -> SandboxPolicy {
    let mut writable_roots = vec![codex_home.to_path_buf()];
    if let Some(trace_dir) = trace_dir {
        writable_roots.push(trace_dir.to_path_buf());
    }
    SandboxPolicy {
        writable_roots,
        full_disk_read: true,
        // The child must reach a model endpoint; this project runs no proxy.
        network: true,
        platform_defaults: true,
        deny_write_subpaths: vec![codex_home.join(RULES_SUBDIR)],
    }
}

impl SandboxPolicy {
    /// Whether this policy stops the runtime from persisting execpolicy rules.
    ///
    /// Asked rather than assumed: the answer decides whether a **permanent**
    /// approval can mean anything. If the rules directory is not writable, a
    /// "forever" verdict grants only the current action and is therefore not a
    /// verdict the interface may offer — see
    /// `approval::available_decisions`.
    pub fn blocks_rule_persistence(&self) -> bool {
        self.deny_write_subpaths
            .iter()
            .any(|path| path.ends_with(RULES_SUBDIR))
    }
}

/// Where the runtime persists execpolicy rules, relative to its home.
///
/// Named once because two places depend on the same layout: the carve-out above,
/// and the verification that the carve-out actually prevents a write.
pub const RULES_SUBDIR: &str = "rules";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_probe_runs_a_real_command_and_reports_a_verdict() {
        // Whatever the platform, the probe must reach a definite verdict rather
        // than erroring or claiming availability it did not observe.
        let availability = probe();

        // `is_available` must agree with the variant, or every caller that
        // gates on it would be reading a different answer than the one shown.
        assert_eq!(
            availability.is_available(),
            matches!(availability, Availability::Available),
        );

        match &availability {
            // Nothing further to check: the variant itself is the evidence,
            // and the probe only produces it after observing a denied write.
            Availability::Available => {}
            Availability::MissingTool { path } => {
                assert_eq!(path, Path::new(seatbelt::SANDBOX_EXEC_PATH));
            }
            Availability::CannotApply { detail } => {
                // A refusal that cannot be diagnosed is a refusal nobody can
                // act on.
                assert!(!detail.is_empty(), "a refusal must explain itself");
            }
            Availability::UnsupportedPlatform => {}
        }
        // The rendering must also always produce text, because it is what the
        // interface shows next to a refusal.
        assert!(!availability.to_string().is_empty());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn confinement_is_applied_or_refused_but_never_skipped() {
        let policy = SandboxPolicy {
            // Resolved, because Seatbelt matches resolved paths and the
            // temporary directory is reached through the `/var` symlink.
            writable_roots: vec![
                std::env::temp_dir()
                    .canonicalize()
                    .unwrap_or_else(|_| std::env::temp_dir()),
            ],
            full_disk_read: true,
            network: false,
            platform_defaults: true,
            deny_write_subpaths: Vec::new(),
        };
        let command = vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            "echo confined".to_string(),
        ];
        match confined_argv(&command, &policy) {
            Ok(argv) => {
                // Wrapped means the program really is sandbox-exec.
                assert_eq!(argv[0], seatbelt::SANDBOX_EXEC_PATH);
                let output = Command::new(&argv[0])
                    .args(&argv[1..])
                    .output()
                    .expect("runs");
                assert!(
                    output.status.success(),
                    "the echo must succeed inside the sandbox: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                assert!(String::from_utf8_lossy(&output.stdout).contains("confined"));
            }
            Err(error) => {
                // The only acceptable failure is a refusal, never a silent
                // fallback to running unconfined.
                assert!(
                    matches!(error, ConfineError::Unavailable(_)),
                    "unexpected error: {error:?}"
                );
                assert!(error.to_string().contains("refusing to run unconfined"));
            }
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_codex_child_policy_grants_no_workspace_write() {
        let home = PathBuf::from("/tmp/taoli-home");
        let trace = PathBuf::from("/tmp/taoli-traces");
        let policy = codex_child_policy(&home, Some(&trace));
        assert_eq!(policy.writable_roots, vec![home, trace]);
        // If this ever contains a workspace path, the read-only boundary has
        // been widened by accident.
        assert!(
            !policy
                .writable_roots
                .iter()
                .any(|root| root == Path::new(".") || root == Path::new("/")),
            "no broad write root: {:?}",
            policy.writable_roots
        );
    }

    #[test]
    fn a_missing_trace_directory_still_yields_a_policy() {
        let policy = codex_child_policy(Path::new("/tmp/taoli-home"), None);
        assert_eq!(policy.writable_roots.len(), 1);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_codex_child_policy_denies_writes_to_the_rules_directory() {
        let home = PathBuf::from("/tmp/taoli-home");
        let policy = codex_child_policy(&home, None);
        assert_eq!(
            policy.deny_write_subpaths,
            vec![home.join(RULES_SUBDIR)],
            "the rules directory must be carved out of the writable home"
        );
        // The home itself stays writable: the runtime stores session and state
        // there and fails to start without it (measured).
        assert!(policy.writable_roots.contains(&home));
    }

    /// The tie that keeps honesty intact: the policy used for the codex child
    /// blocks rule persistence, and that is what the approval layer keys on when
    /// deciding whether a permanent verdict may be offered.
    ///
    /// If someone removes the carve-out, this fails — rather than a "始终允许"
    /// button quietly reappearing while the sandbox still ignores it.
    #[test]
    fn the_child_policy_reports_that_rule_persistence_is_blocked() {
        let policy = codex_child_policy(Path::new("/tmp/taoli-home"), None);
        assert!(
            policy.blocks_rule_persistence(),
            "the child must not be able to write the runtime's rule store: {:?}",
            policy.deny_write_subpaths
        );
    }

    #[test]
    fn a_policy_without_the_carve_out_does_not_claim_to_block_persistence() {
        let policy = SandboxPolicy {
            writable_roots: vec![PathBuf::from("/tmp")],
            ..SandboxPolicy::default()
        };
        assert!(!policy.blocks_rule_persistence());
    }
}
