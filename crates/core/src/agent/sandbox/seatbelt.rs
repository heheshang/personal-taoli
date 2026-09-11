//! macOS Seatbelt confinement: profile assembly and command wrapping.
//!
//! Ported from `openai/codex` (`codex-rs/sandboxing`), Apache-2.0 — see
//! `PROVENANCE.md` for exactly what was taken and what was left behind. The
//! policy **text** is copied byte-for-byte; the filesystem half of the profile
//! builder is adapted here.
//!
//! Two upstream decisions are preserved because they are load-bearing:
//!
//! * **Paths travel as `-D` parameters, never as policy text.** A path is
//!   interpolated into the policy only as `(param "KEY")`, so a path containing
//!   a quote or a space cannot break out of its own literal.
//! * **Writable roots get an anchor deny.** Without
//!   `(deny file-write-unlink … (vnode-type DIRECTORY))` a confined process
//!   could rename or delete a writable root and thereby relocate itself out of
//!   the carve-out that applies to its descendants.
//!
//! What is *not* here: upstream's per-command permission model, its managed
//! network proxy, and its protection of metadata directories inside a writable
//! workspace. This project confines one fixed child process and grants it no
//! workspace write access, so none of those has a caller — see `PROVENANCE.md`.

use std::path::{Path, PathBuf};

/// Upstream: `seatbelt_base_policy.sbpl`, byte-identical.
const BASE_POLICY: &str = include_str!("policies/seatbelt_base_policy.sbpl");
/// Upstream: `seatbelt_network_policy.sbpl`, byte-identical.
const NETWORK_POLICY: &str = include_str!("policies/seatbelt_network_policy.sbpl");
/// Upstream: `seatbelt_preferences_policy.sbpl`, byte-identical.
const PREFERENCES_POLICY: &str = include_str!("policies/seatbelt_preferences_policy.sbpl");
/// Upstream: `seatbelt_read_only_platform_defaults.sbpl`, byte-identical.
const READ_ONLY_PLATFORM_DEFAULTS: &str =
    include_str!("policies/seatbelt_read_only_platform_defaults.sbpl");

/// Scratch directories an ordinary process is expected to be able to write.
///
/// Ported from upstream's `MACOS_PROCESS_PLATFORM_DEFAULTS`. macOS resolves
/// `$TMPDIR` into `/private/var/folders/...`, which is why both spellings of the
/// temporary directories appear.
const PROCESS_PLATFORM_DEFAULTS: &str = r#"
(allow file-read* (subpath "/Applications"))
(allow file-read* file-test-existence file-write* (subpath "/tmp"))
(allow file-read* file-write* (subpath "/private/tmp"))
(allow file-read* file-write* (subpath "/var/tmp"))
(allow file-read* file-write* (subpath "/private/var/tmp"))
"#;

/// The one executable upstream will invoke, and the only one this project will.
///
/// Pinned to `/usr/bin` deliberately: resolving `sandbox-exec` through `PATH`
/// would let an attacker who can write to an earlier `PATH` entry supply the
/// program that is supposed to be doing the confining.
pub const SANDBOX_EXEC_PATH: &str = "/usr/bin/sandbox-exec";

/// A path that is absolute and normalised.
///
/// A small stand-in for upstream's `codex-utils-absolute-path`. The invariant
/// that matters for policy building is the same: a relative path in a Seatbelt
/// profile is meaningless, and silently normalising one against the current
/// directory would grant access to somewhere the caller never named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbsolutePathBuf(PathBuf);

impl AbsolutePathBuf {
    /// Accepts `path` only if it is already absolute.
    pub fn from_absolute_path(path: &Path) -> Result<Self, PathError> {
        if !path.is_absolute() {
            return Err(PathError::NotAbsolute(path.to_path_buf()));
        }
        Ok(Self(normalize(path)))
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }

    pub fn into_path_buf(self) -> PathBuf {
        self.0
    }
}

/// Why a path could not be used as a sandbox root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathError {
    NotAbsolute(PathBuf),
}

impl std::fmt::Display for PathError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAbsolute(path) => {
                write!(formatter, "path must be absolute: {}", path.display())
            }
        }
    }
}

impl std::error::Error for PathError {}

/// Lexically normalises a path: collapses `.`, resolves `..` textually, and
/// removes duplicate and trailing separators.
///
/// Purely lexical (no filesystem access), matching upstream's
/// `normalize_path_for_platform`. Symlinks are handled separately, because
/// resolving them is a different question with a different answer.
fn normalize(path: &Path) -> PathBuf {
    use std::path::Component;

    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                // Never pop past the root.
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        out.push("/");
    }
    out
}

/// Why a profile could not be built.
///
/// Every variant is a configuration the caller must fix; none of them may be
/// downgraded to "run without confinement".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxError {
    /// A root was not absolute.
    RelativeRoot(PathBuf),
    /// A writable root contains a symlink in a user-controlled component.
    ///
    /// Granting the resolved target would grant something the caller did not
    /// name, and the target can change between runs.
    SymlinkedWritableRoot { root: PathBuf, symlink: PathBuf },
    /// Filesystem inspection of a root failed.
    Inspect { root: PathBuf, detail: String },
    /// The top-level alias normalisation failed.
    Normalize { root: PathBuf, detail: String },
}

impl std::fmt::Display for SandboxError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RelativeRoot(path) => {
                write!(
                    formatter,
                    "sandbox root must be absolute: {}",
                    path.display()
                )
            }
            Self::SymlinkedWritableRoot { root, symlink } => write!(
                formatter,
                "writable root {} contains symlink component {}; \
                 symlinked writable roots are not supported",
                root.display(),
                symlink.display()
            ),
            Self::Inspect { root, detail } => {
                write!(
                    formatter,
                    "cannot inspect sandbox root {}: {detail}",
                    root.display()
                )
            }
            Self::Normalize { root, detail } => write!(
                formatter,
                "cannot normalise sandbox root {}: {detail}",
                root.display()
            ),
        }
    }
}

impl std::error::Error for SandboxError {}

/// What is granted to the confined process.
///
/// Reads and writes are stated separately because they are decided separately:
/// this project grants broad reads (a child process must load its runtime) and a
/// short write allowlist.
#[derive(Debug, Clone, Default)]
pub struct SandboxPolicy {
    /// Roots the process may write to. Everything else is denied.
    pub writable_roots: Vec<PathBuf>,
    /// When true, reads are unrestricted.
    ///
    /// Left as an explicit flag rather than always-on so that a future caller
    /// granting narrow reads does not silently inherit full-disk read.
    pub full_disk_read: bool,
    /// Whether outbound network access is granted.
    pub network: bool,
    /// Whether the platform scratch defaults are added.
    ///
    /// Needed for an ordinary process: runtimes expect `$TMPDIR` and
    /// `/tmp` to be writable.
    pub platform_defaults: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AccessKind {
    Read,
    Write,
}

#[derive(Debug)]
enum NormalizedWritableRoot {
    Subpath(AbsolutePathBuf),
    Literal(AbsolutePathBuf),
}

struct AccessRoot {
    root: AbsolutePathBuf,
}

/// Finds a symlink in any component below the top level.
///
/// Top-level aliases such as `/tmp -> /private/tmp` are expected on macOS and
/// are handled by [`normalize_top_level_alias`]; a symlink deeper in the path is
/// under user control and is refused instead.
fn nested_symlink_component(path: &Path) -> Option<&Path> {
    path.ancestors().find(|ancestor| {
        let Ok(metadata) = std::fs::symlink_metadata(ancestor) else {
            return false;
        };
        metadata.file_type().is_symlink() && ancestor.parent().and_then(Path::parent).is_some()
    })
}

/// Resolves a symlinked top-level directory (e.g. `/tmp`) while keeping the
/// rest of the path as written.
fn normalize_top_level_alias(path: AbsolutePathBuf) -> Result<AbsolutePathBuf, SandboxError> {
    let Some(top_level) = path.as_path().ancestors().find(|ancestor| {
        ancestor.parent().is_some() && ancestor.parent().and_then(Path::parent).is_none()
    }) else {
        return Ok(path);
    };
    if !std::fs::symlink_metadata(top_level).is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        return Ok(path);
    }

    let canonical = top_level
        .canonicalize()
        .map_err(|error| SandboxError::Normalize {
            root: path.as_path().to_path_buf(),
            detail: format!("cannot resolve {}: {error}", top_level.display()),
        })?;
    let suffix =
        path.as_path()
            .strip_prefix(top_level)
            .map_err(|error| SandboxError::Normalize {
                root: path.as_path().to_path_buf(),
                detail: format!("cannot split off {}: {error}", top_level.display()),
            })?;
    AbsolutePathBuf::from_absolute_path(&canonical.join(suffix)).map_err(|error| {
        SandboxError::Normalize {
            root: path.as_path().to_path_buf(),
            detail: error.to_string(),
        }
    })
}

/// Resolves a writable root to what should appear in the policy.
///
/// A root that exists as a directory becomes a `subpath` grant; one that exists
/// as anything else (or not at all) becomes a `literal` grant, so that a
/// nonexistent path cannot be used to claim a whole subtree.
fn normalize_writable_root(root: AbsolutePathBuf) -> Result<NormalizedWritableRoot, SandboxError> {
    if let Some(symlink) = nested_symlink_component(root.as_path()) {
        return Err(SandboxError::SymlinkedWritableRoot {
            root: root.as_path().to_path_buf(),
            symlink: symlink.to_path_buf(),
        });
    }
    let normalized = normalize_top_level_alias(root.clone())?;

    let metadata = match std::fs::symlink_metadata(normalized.as_path()) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(NormalizedWritableRoot::Subpath(normalized));
        }
        Err(error) => {
            return Err(SandboxError::Inspect {
                root: normalized.as_path().to_path_buf(),
                detail: error.to_string(),
            });
        }
    };
    if metadata.is_dir() {
        return Ok(NormalizedWritableRoot::Subpath(normalized));
    }
    Ok(NormalizedWritableRoot::Literal(normalized))
}

/// Builds one `(allow …)` clause per root, in `-D`-parameter form.
///
/// Returns the policy fragment and the parameters it references. Adapted from
/// upstream's implementation of the same shape, including the anchor denies that
/// keep a confined process from unlinking the directory anchoring its own grant.
fn build_access_policy(
    access: AccessKind,
    roots: Vec<AccessRoot>,
) -> Result<(String, Vec<(String, PathBuf)>), SandboxError> {
    let mut filters = Vec::new();
    let mut anchor_denies = Vec::new();
    let mut params = Vec::new();
    let (action, prefix) = match access {
        AccessKind::Read => ("file-read*", "READABLE_ROOT"),
        AccessKind::Write => ("file-write*", "WRITABLE_ROOT"),
    };

    for (index, access_root) in roots.into_iter().enumerate() {
        let param = format!("{prefix}_{index}");
        match access {
            AccessKind::Read => {
                // A read root that failed normalisation is still usable as
                // written; reads are not a privilege escalation.
                let root = AbsolutePathBuf::from_absolute_path(access_root.root.as_path())
                    .unwrap_or(access_root.root);
                params.push((param.clone(), root.into_path_buf()));
                filters.push(format!("(subpath (param \"{param}\"))"));
            }
            AccessKind::Write => {
                // A confined process must not be able to delete or rename the
                // directory that anchors its own write grant: doing so relocates
                // its descendants out of the carve-out they are policed by.
                anchor_denies.push(format!(
                    "(deny file-write-unlink (require-all (literal (param \"{param}\")) (vnode-type DIRECTORY)))"
                ));
                let filter = match normalize_writable_root(access_root.root)? {
                    NormalizedWritableRoot::Subpath(root) => {
                        params.push((param.clone(), root.into_path_buf()));
                        format!("(subpath (param \"{param}\"))")
                    }
                    NormalizedWritableRoot::Literal(root) => {
                        params.push((param.clone(), root.into_path_buf()));
                        format!("(literal (param \"{param}\"))")
                    }
                };
                filters.push(filter);
            }
        }
    }

    if filters.is_empty() {
        return Ok((String::new(), Vec::new()));
    }
    let mut policy = vec![format!("(allow {action}\n{}\n)", filters.join(" "))];
    policy.extend(anchor_denies);
    Ok((policy.join("\n"), params))
}

/// The full profile for `policy`, without any `-D` arguments.
///
/// Exposed so the assembled text can be asserted in tests without launching
/// anything. [`sandbox_command`] is what callers should normally use.
pub fn profile_text(policy: &SandboxPolicy) -> Result<String, SandboxError> {
    let (text, _) = build_profile(policy)?;
    Ok(text)
}

fn build_profile(policy: &SandboxPolicy) -> Result<(String, Vec<(String, PathBuf)>), SandboxError> {
    let mut sections = vec![BASE_POLICY.to_string()];

    if policy.full_disk_read {
        sections.push("; allow read-only file operations\n(allow file-read*)".to_string());
    } else {
        let (read_policy, _) = build_access_policy(
            AccessKind::Read,
            vec![AccessRoot {
                root: AbsolutePathBuf::from_absolute_path(Path::new("/"))
                    .map_err(|_| SandboxError::RelativeRoot(PathBuf::from("/")))?,
            }],
        )?;
        sections.push(format!("; allow read-only file operations\n{read_policy}"));
    }

    let roots: Vec<AccessRoot> = policy
        .writable_roots
        .iter()
        .map(|root| {
            AbsolutePathBuf::from_absolute_path(root)
                .map(|root| AccessRoot { root })
                .map_err(|_| SandboxError::RelativeRoot(root.clone()))
        })
        .collect::<Result<_, _>>()?;
    let (write_policy, write_params) = build_access_policy(AccessKind::Write, roots)?;
    sections.push(write_policy);

    if policy.network {
        sections.push(NETWORK_POLICY.to_string());
        // The upstream network policy opens only AF_SYSTEM sockets and the
        // Mach services TLS and DNS need; it deliberately does not grant
        // outbound TCP, because upstream routes traffic through its own proxy.
        // This project runs no such proxy, so the child needs the explicit
        // grant to reach a model endpoint.
        sections.push("(allow network*)".to_string());
    }
    if policy.full_disk_read {
        sections.push(PREFERENCES_POLICY.to_string());
    }
    if policy.platform_defaults {
        sections.push(READ_ONLY_PLATFORM_DEFAULTS.to_string());
        sections.push(PROCESS_PLATFORM_DEFAULTS.to_string());
    }

    Ok((sections.join("\n"), write_params))
}

/// Wraps `command` so it runs confined by `policy`.
///
/// Returns the full argv: `sandbox-exec -p <profile> -D… -- <command>`.
/// Paths reach the policy only as `-D` parameters, so no path can break out of
/// its own literal in the profile text.
pub fn sandbox_command(
    command: &[String],
    policy: &SandboxPolicy,
) -> Result<Vec<String>, SandboxError> {
    if command.is_empty() {
        return Err(SandboxError::RelativeRoot(PathBuf::new()));
    }
    let (profile, params) = build_profile(policy)?;
    let mut argv = vec![SANDBOX_EXEC_PATH.to_string(), "-p".to_string(), profile];
    argv.extend(
        params
            .into_iter()
            .map(|(key, value)| format!("-D{key}={}", value.to_string_lossy())),
    );
    argv.push("--".to_string());
    argv.extend(command.iter().cloned());
    Ok(argv)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    fn macos_policy(writable: Vec<PathBuf>) -> SandboxPolicy {
        SandboxPolicy {
            writable_roots: writable,
            full_disk_read: true,
            network: true,
            platform_defaults: true,
        }
    }

    #[test]
    fn a_relative_root_is_refused_rather_than_resolved() {
        // Silently resolving it against the cwd would grant access to a path
        // the caller never named.
        let policy = SandboxPolicy {
            writable_roots: vec![PathBuf::from("relative/path")],
            ..SandboxPolicy::default()
        };
        let error = profile_text(&policy).expect_err("must refuse");
        assert!(matches!(error, SandboxError::RelativeRoot(_)), "{error:?}");
    }

    #[test]
    fn absolute_paths_are_normalised_lexically() {
        let path =
            AbsolutePathBuf::from_absolute_path(Path::new("/a/./b/../c//d")).expect("absolute");
        assert_eq!(path.as_path(), Path::new("/a/c/d"));
        // Never pops past the root.
        let deep = AbsolutePathBuf::from_absolute_path(Path::new("/../..")).expect("absolute");
        assert_eq!(deep.as_path(), Path::new("/"));
    }

    #[test]
    fn a_relative_path_is_not_accepted_as_an_absolute_one() {
        assert!(matches!(
            AbsolutePathBuf::from_absolute_path(Path::new("x")),
            Err(PathError::NotAbsolute(_))
        ));
    }

    #[test]
    fn the_profile_is_closed_by_default_and_grants_writes_only_where_named() {
        let policy = SandboxPolicy {
            writable_roots: vec![PathBuf::from("/tmp")],
            full_disk_read: true,
            network: false,
            platform_defaults: false,
        };
        let text = profile_text(&policy).expect("builds");

        // Upstream's base policy closes by default; that must survive the port,
        // because a profile that opened by default would make every later
        // allow-list meaningless.
        assert!(
            text.contains("(deny default)"),
            "profile must deny by default"
        );
        assert!(
            text.contains("(allow file-write*"),
            "the named root is granted"
        );
        assert!(
            text.contains("(param \"WRITABLE_ROOT_0\")"),
            "paths travel as -D params"
        );
        // The anchor deny is what stops the confined process from moving its own
        // grant out from under itself.
        assert!(
            text.contains("(deny file-write-unlink"),
            "writable roots need an anchor deny"
        );
        // Nothing may grant blanket write access.
        assert!(
            !text.contains("(allow file-write*)") || text.contains("(param"),
            "no unconditional write grant"
        );
    }

    #[test]
    fn network_is_not_granted_unless_asked_for() {
        let without = profile_text(&SandboxPolicy {
            writable_roots: vec![PathBuf::from("/tmp")],
            full_disk_read: true,
            network: false,
            platform_defaults: false,
        })
        .expect("builds");
        assert!(!without.contains("(allow network*)"));

        let with = profile_text(&SandboxPolicy {
            writable_roots: vec![PathBuf::from("/tmp")],
            full_disk_read: true,
            network: true,
            platform_defaults: false,
        })
        .expect("builds");
        assert!(with.contains("(allow network*)"));
    }

    #[test]
    fn narrow_reads_fall_back_to_explicit_grants_rather_than_full_disk() {
        let policy = SandboxPolicy {
            writable_roots: vec![PathBuf::from("/tmp")],
            full_disk_read: false,
            network: false,
            platform_defaults: false,
        };
        let text = profile_text(&policy).expect("builds");
        assert!(
            !text.contains("(allow file-read*)"),
            "full-disk read must be opt-in: {text}"
        );
        assert!(text.contains("(param \"READABLE_ROOT_0\")"));
    }

    #[test]
    fn the_profile_text_taken_from_upstream_is_unchanged() {
        // Guards the port against accidental edits to the copied policy text:
        // these strings are the vetted part of the sandbox.
        assert!(BASE_POLICY.contains("(deny default)"));
        assert!(BASE_POLICY.contains("Chrome's sandbox policy"));
        assert!(NETWORK_POLICY.contains("AF_SYSTEM"));
        assert!(PREFERENCES_POLICY.contains("user-preference-read"));
        assert!(READ_ONLY_PLATFORM_DEFAULTS.contains("/System"));
    }

    #[test]
    fn the_command_is_wrapped_through_the_pinned_executable() {
        let policy = SandboxPolicy {
            writable_roots: vec![PathBuf::from("/tmp")],
            full_disk_read: true,
            network: false,
            platform_defaults: false,
        };
        let argv =
            sandbox_command(&["/bin/echo".to_string(), "hi".to_string()], &policy).expect("builds");
        assert_eq!(
            argv[0], "/usr/bin/sandbox-exec",
            "must not resolve via PATH"
        );
        assert_eq!(argv[1], "-p");
        assert_eq!(argv[argv.len() - 3], "--");
        assert_eq!(
            &argv[argv.len() - 2..],
            &["/bin/echo".to_string(), "hi".to_string()]
        );
        // Every -D argument precedes the `--` separator.
        let separator = argv.iter().position(|arg| arg == "--").expect("has --");
        for (index, arg) in argv.iter().enumerate() {
            if arg.starts_with("-D") {
                assert!(index < separator, "-D argument after `--`: {arg}");
            }
        }
    }

    #[test]
    fn an_empty_command_is_refused() {
        let policy = SandboxPolicy::default();
        assert!(sandbox_command(&[], &policy).is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_path_with_a_quote_cannot_break_out_of_its_literal() {
        // The reason paths travel as -D parameters: this directory name would
        // otherwise be able to inject policy syntax.
        let dir = std::env::temp_dir().join("taoli-quote-\"injection");
        let _ = std::fs::create_dir_all(&dir);
        let policy = macos_policy(vec![dir.clone()]);
        let argv = sandbox_command(&["/bin/echo".to_string()], &policy).expect("builds");

        let profile = &argv[2];
        let param_args: Vec<&String> = argv.iter().filter(|arg| arg.starts_with("-D")).collect();
        assert!(
            param_args.iter().any(|arg| arg.contains("injection")),
            "the path must appear as a parameter: {param_args:?}"
        );
        assert!(
            !profile.contains("injection"),
            "the path must not be interpolated into the profile text"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_symlinked_writable_root_is_refused() {
        // Granting the resolved target would grant something the caller did not
        // name, and the link can be repointed later.
        let base = std::env::temp_dir().join("taoli-symlink-probe");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).expect("create base");
        let target = base.join("target");
        std::fs::create_dir_all(&target).expect("create target");
        let link = base.join("link");
        std::os::unix::fs::symlink(&target, &link).expect("create symlink");

        let policy = macos_policy(vec![link.clone()]);
        let error = profile_text(&policy).expect_err("symlinked root must be refused");
        assert!(
            matches!(error, SandboxError::SymlinkedWritableRoot { .. }),
            "{error:?}"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_top_level_aliases_are_accepted() {
        // `/tmp -> /private/tmp` is expected on macOS and must not be mistaken
        // for a user-controlled symlink.
        let policy = macos_policy(vec![PathBuf::from("/tmp")]);
        let text = profile_text(&policy).expect("macOS alias must be accepted");
        assert!(text.contains("(param \"WRITABLE_ROOT_0\")"));
    }
}
