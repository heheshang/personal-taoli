//! Domain instructions injected into the agent runtime.
//!
//! The text lives in the repository (`docs/agent/instructions.md`) rather than
//! in this crate, so it is reviewed and versioned like any other contract.
//! Nothing here paraphrases it: the loader reads the file and hands the bytes
//! on, so what the agent receives is exactly what the repository contains.
//!
//! Missing or empty instructions are a **failure**, not a default. An agent
//! running without its domain constraints still looks like it is working, and
//! the constraint most worth keeping — "numbers come from tools, never from
//! memory" — is exactly the one its absence would silently drop.
//!
//! Measured against the live runtime: `thread/start.developerInstructions` is
//! accepted without any experimental capability, and the model honours it. The
//! field appends a fragment to the prompt rather than replacing the runtime's
//! own base instructions, which is why this is the right field for extra
//! project rules; `baseInstructions` would replace them wholesale.

use std::path::{Path, PathBuf};

/// Repository-relative location of the instruction document.
pub const DEFAULT_PATH: &str = "docs/agent/instructions.md";

/// Why the instructions could not be used.
#[derive(Debug)]
pub enum InstructionsError {
    /// The file was not found, including after searching parent directories.
    NotFound { searched: Vec<PathBuf> },
    /// The file exists but could not be read.
    Unreadable {
        path: PathBuf,
        source: std::io::Error,
    },
    /// The file exists but carries no instruction text.
    Empty { path: PathBuf },
}

impl std::fmt::Display for InstructionsError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound { searched } => write!(
                formatter,
                "找不到领域指令文件 {DEFAULT_PATH}（已查找：{}）",
                searched
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Unreadable { path, source } => write!(
                formatter,
                "无法读取领域指令文件 {}：{source}",
                path.display()
            ),
            Self::Empty { path } => write!(
                formatter,
                "领域指令文件 {} 为空；拒绝在无领域约束的情况下启动",
                path.display()
            ),
        }
    }
}

impl std::error::Error for InstructionsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unreadable { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Resolves [`DEFAULT_PATH`] starting from `base`, then each of its ancestors.
///
/// The same search shape the observer config uses: the app may be launched from
/// the project root or from a build directory several levels down.
pub fn resolve_from(base: &Path) -> Option<PathBuf> {
    if base.is_file() {
        return Some(base.to_path_buf());
    }
    let relative = Path::new(DEFAULT_PATH);
    let direct = base.join(relative);
    if direct.is_file() {
        return Some(direct);
    }
    base.ancestors()
        .map(|ancestor| ancestor.join(relative))
        .find(|candidate| candidate.is_file())
}

/// Resolves [`DEFAULT_PATH`] from the working directory, then from the
/// executable's ancestors.
pub fn resolve_default() -> Option<PathBuf> {
    if let Ok(cwd) = std::env::current_dir()
        && let Some(found) = resolve_from(&cwd)
    {
        return Some(found);
    }
    let executable = std::env::current_exe().ok()?;
    resolve_from(executable.parent()?)
}

/// The project root, derived from where [`DEFAULT_PATH`] was found.
///
/// The document lives at `<root>/docs/agent/instructions.md`, so finding it
/// also locates the root. This is what callers should anchor repo-relative
/// paths to: a path like `data/agent-traces` is only meaningful relative to the
/// project, and a *sandbox* root must additionally be absolute — the macOS
/// profile builder refuses relative roots outright, because normalising one
/// against some process's working directory would grant a directory nobody
/// named.
pub fn project_root() -> Option<PathBuf> {
    project_root_from(&std::env::current_dir().ok()?)
}

/// [`project_root`] anchored at `base`, for callers that know where they are.
///
/// Split out so the ancestor walk can be tested against a synthetic tree rather
/// than whatever directory the test happens to run in.
pub fn project_root_from(base: &Path) -> Option<PathBuf> {
    let document = resolve_from(base)?;
    // <root>/docs/agent/instructions.md -> <root>
    document.parent()?.parent()?.parent().map(Path::to_path_buf)
}

/// Reads the instruction text from `path`.
///
/// Returns the file's contents trimmed of surrounding whitespace. The document
/// is passed through unchanged otherwise: any rewriting here would put the
/// crate's copy and the repository's copy out of step.
pub fn load(path: &Path) -> Result<String, InstructionsError> {
    let contents = std::fs::read_to_string(path).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            InstructionsError::NotFound {
                searched: vec![path.to_path_buf()],
            }
        } else {
            InstructionsError::Unreadable {
                path: path.to_path_buf(),
                source,
            }
        }
    })?;
    let trimmed = contents.trim();
    if trimmed.is_empty() {
        return Err(InstructionsError::Empty {
            path: path.to_path_buf(),
        });
    }
    Ok(trimmed.to_string())
}

/// Resolves and loads [`DEFAULT_PATH`], or explains why it could not.
pub fn load_default() -> Result<String, InstructionsError> {
    let path = resolve_default().ok_or_else(|| InstructionsError::NotFound {
        searched: vec![PathBuf::from(DEFAULT_PATH)],
    })?;
    load(&path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "taoli-instructions-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch");
        dir
    }

    /// The repository's own document, used as the real fixture.
    fn repo_document() -> Option<PathBuf> {
        resolve_from(Path::new(env!("CARGO_MANIFEST_DIR")))
    }

    #[test]
    fn the_document_is_found_from_inside_the_workspace() {
        // `CARGO_MANIFEST_DIR` is `crates/core`; the document is two levels up,
        // so this also exercises the ancestor walk.
        let Some(path) = repo_document() else {
            eprintln!("skipping: {DEFAULT_PATH} is absent");
            return;
        };
        assert!(path.ends_with(DEFAULT_PATH), "unexpected path: {path:?}");
    }

    #[test]
    fn the_document_still_states_the_rules_that_matter() {
        // Guards against the document being gutted: the constraints below are
        // the ones whose silent removal would be hardest to notice, because an
        // agent without them still behaves plausibly.
        let Ok(text) = load_default() else {
            eprintln!("skipping: instructions are absent from this checkout");
            return;
        };
        for rule in [
            "只读",
            "工具返回值是唯一权威",
            "不得",
            "显式标注",
            "单轮单闭环",
            "会被记录",
        ] {
            assert!(text.contains(rule), "instruction document lost: {rule}");
        }
        // The boundary that keeps this out of the trading path.
        assert!(
            text.contains("没有") && text.contains("下单"),
            "read-only boundary missing"
        );
    }

    #[test]
    fn the_loaded_text_is_the_file_verbatim() {
        // No rewriting between the repository and the runtime: otherwise the
        // reviewed text and the injected text could differ.
        let dir = scratch("verbatim");
        let path = dir.join("instructions.md");
        let body = "# Rules\n\n- exact text, including  spacing\n";
        std::fs::write(&path, body).expect("write");

        let loaded = load(&path).expect("loads");
        assert_eq!(loaded, body.trim());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_empty_document_is_a_failure_not_a_default() {
        let dir = scratch("empty");
        let path = dir.join("instructions.md");
        std::fs::write(&path, "   \n\n\t\n").expect("write");

        let error = load(&path).expect_err("whitespace-only must be refused");
        assert!(
            matches!(error, InstructionsError::Empty { .. }),
            "{error:?}"
        );
        assert!(error.to_string().contains("拒绝在无领域约束的情况下启动"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_document_names_what_was_searched() {
        let error = load(Path::new("/nonexistent/instructions.md")).expect_err("must fail");
        assert!(
            matches!(error, InstructionsError::NotFound { .. }),
            "{error:?}"
        );
        // The message must name the expected location, or nobody can fix it.
        assert!(error.to_string().contains(DEFAULT_PATH), "{error}");
    }

    #[test]
    fn resolution_walks_up_to_the_workspace_root() {
        let dir = scratch("ancestors");
        let nested = dir.join("a").join("b").join("c");
        std::fs::create_dir_all(&nested).expect("mkdir");
        std::fs::create_dir_all(dir.join("docs").join("agent")).expect("mkdir docs");
        let document = dir.join(DEFAULT_PATH);
        std::fs::write(&document, "# from the root\n").expect("write");

        let found = resolve_from(&nested).expect("found from nested directory");
        assert_eq!(found, document);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolution_prefers_an_explicit_file_over_the_search() {
        let dir = scratch("explicit");
        let direct = dir.join("explicit.md");
        std::fs::write(&direct, "# explicit\n").expect("write");

        assert_eq!(resolve_from(&direct), Some(direct.clone()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolution_reports_absence_rather_than_inventing_a_path() {
        let dir = scratch("absent");
        assert_eq!(resolve_from(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_project_root_is_where_the_document_was_found() {
        let Some(root) = project_root() else {
            eprintln!("skipping: {DEFAULT_PATH} is absent");
            return;
        };
        assert!(root.is_absolute(), "the root must be absolute: {root:?}");
        assert!(
            root.join(DEFAULT_PATH).is_file(),
            "the root must actually contain the document: {root:?}"
        );
        // The anchor this exists for: repo-relative paths become absolute.
        let trace = root.join("data").join("agent-traces");
        assert!(trace.is_absolute(), "{trace:?}");
        assert!(!trace.starts_with(".."), "{trace:?}");
    }

    #[test]
    fn a_nested_working_directory_resolves_to_the_trees_root() {
        let dir = scratch("root-nested");
        let nested = dir.join("a").join("b");
        std::fs::create_dir_all(&nested).expect("mkdir");
        std::fs::create_dir_all(dir.join("docs").join("agent")).expect("mkdir docs");
        std::fs::write(dir.join(DEFAULT_PATH), "# rules\n").expect("write");

        // From three levels down, the walk must land on the tree's own root.
        // (Absence is covered by `resolution_reports_absence_rather_than_inventing_a_path`:
        // the walk necessarily keeps ascending, so it cannot stop at an
        // intermediate directory that happens to lack the document.)
        assert_eq!(project_root_from(&nested), Some(dir.clone()));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
