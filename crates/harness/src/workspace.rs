//! The agent workspace contract (harness guide §10.1).
//!
//! ```text
//! /workspace/<agent_id>/<session_id>/
//!   inbox/     inputs handed to the agent   (read-only to the agent)
//!   work/      scratch area                 (full CRUD)
//!   memory/    scratchpad, notes, plan      (full CRUD)
//!   outputs/   final deliverables ONLY      (full CRUD; the only thing a caller sees)
//!   logs/      harness-written step logs    (read-only to the agent)
//! ```
//!
//! Two rules carry the weight.
//!
//! **Path validation is harness code, never a prompt.** Every path is canonicalised
//! and checked against the workspace root before use. The model's path string is an
//! input, not an instruction — `../../etc/passwd` and a symlink pointing out of the
//! tree are the same attack, and both are defeated by resolving before comparing
//! rather than by pattern-matching the string.
//!
//! **`outputs/` is a contract.** Only files placed there and announced through
//! `deliver` count as results. That is what makes "done" a checkable claim (§5.1)
//! instead of the model's opinion, and it keeps half-finished scratch work out of
//! the user's view.

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The five directories, and what the agent may do in each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Area {
    Inbox,
    Work,
    Memory,
    Outputs,
    Logs,
}

impl Area {
    pub const ALL: &'static [Area] = &[
        Area::Inbox,
        Area::Work,
        Area::Memory,
        Area::Outputs,
        Area::Logs,
    ];

    #[must_use]
    pub fn dir(self) -> &'static str {
        match self {
            Area::Inbox => "inbox",
            Area::Work => "work",
            Area::Memory => "memory",
            Area::Outputs => "outputs",
            Area::Logs => "logs",
        }
    }

    /// Whether the agent may create, modify or delete here.
    ///
    /// `inbox/` and `logs/` are read-only *structurally*: inputs are copies the user
    /// still owns, and logs are the harness's account of what happened. An agent
    /// that could rewrite its own logs would make the trace worthless exactly when
    /// it matters.
    #[must_use]
    pub fn agent_writable(self) -> bool {
        matches!(self, Area::Work | Area::Memory | Area::Outputs)
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Area::ALL.iter().copied().find(|a| a.dir() == s)
    }
}

/// Per-workspace ceilings (§10.1). Surfaced to the model as tool errors.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Quota {
    pub max_bytes: u64,
    pub max_files: usize,
    /// Cap on a single `read` returning into context. Distinct from the profile's
    /// tool-result cap because a file read is the most common way a large payload
    /// gets into a prompt by accident.
    pub max_read_bytes: usize,
}

impl Default for Quota {
    fn default() -> Self {
        Self {
            max_bytes: 8 * 1024 * 1024 * 1024,
            max_files: 50_000,
            max_read_bytes: 256 * 1024,
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PathError {
    #[error("path {given:?} escapes the workspace")]
    Escapes { given: String },
    #[error("path {given:?} names no area; expected one of inbox/ work/ memory/ outputs/ logs/")]
    NoArea { given: String },
    #[error("{area} is read-only to the agent")]
    ReadOnly { area: &'static str },
    #[error("path {given:?} is absolute; give a path relative to the workspace root")]
    Absolute { given: String },
}

impl PathError {
    /// Model-facing repair instruction (§2.3: errors are prompts).
    #[must_use]
    pub fn fix(&self) -> String {
        match self {
            PathError::Escapes { .. } => {
                "stay inside the workspace: use work/, memory/ or outputs/".into()
            }
            PathError::NoArea { .. } => {
                "prefix the path with an area, e.g. work/analysis.py".into()
            }
            PathError::ReadOnly { area } => format!(
                "{area} is read-only; write to work/ for scratch, memory/ for notes, outputs/ for deliverables"
            ),
            PathError::Absolute { .. } => {
                "give a path relative to the workspace root, e.g. work/out.parquet".into()
            }
        }
    }
}

/// One session's workspace.
#[derive(Debug, Clone)]
pub struct Workspace {
    root: PathBuf,
    pub quota: Quota,
}

impl Workspace {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>, quota: Quota) -> Self {
        Self {
            root: root.into(),
            quota,
        }
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The layout, for creation at session start.
    #[must_use]
    pub fn areas(&self) -> Vec<PathBuf> {
        Area::ALL.iter().map(|a| self.root.join(a.dir())).collect()
    }

    /// Resolves a model-supplied relative path, or refuses.
    ///
    /// Lexical resolution, deliberately: `..` components are consumed against the
    /// stack and a path that pops past the root is rejected *before* it touches the
    /// filesystem. Resolving on disk first would follow a symlink the agent planted,
    /// which is the escape this is guarding.
    ///
    /// `write` additionally refuses the read-only areas.
    pub fn resolve(&self, given: &str, write: bool) -> Result<PathBuf, PathError> {
        let raw = Path::new(given);
        if raw.is_absolute() || given.starts_with('/') || given.starts_with('\\') {
            return Err(PathError::Absolute {
                given: given.to_string(),
            });
        }
        // A Windows drive prefix (`C:foo`) is absolute in spirit even when `is_absolute`
        // disagrees, and a UNC-ish `\\host\share` is worse.
        if given.contains(':') {
            return Err(PathError::Absolute {
                given: given.to_string(),
            });
        }

        let mut stack: Vec<String> = Vec::new();
        for comp in raw.components() {
            match comp {
                Component::CurDir => {}
                Component::ParentDir => {
                    if stack.pop().is_none() {
                        return Err(PathError::Escapes {
                            given: given.to_string(),
                        });
                    }
                }
                Component::Normal(part) => {
                    stack.push(part.to_string_lossy().into_owned());
                }
                Component::RootDir | Component::Prefix(_) => {
                    return Err(PathError::Escapes {
                        given: given.to_string(),
                    })
                }
            }
        }

        let Some(area_name) = stack.first() else {
            return Err(PathError::NoArea {
                given: given.to_string(),
            });
        };
        let Some(area) = Area::parse(area_name) else {
            return Err(PathError::NoArea {
                given: given.to_string(),
            });
        };
        if write && !area.agent_writable() {
            return Err(PathError::ReadOnly { area: area.dir() });
        }

        let mut out = self.root.clone();
        for part in &stack {
            out.push(part);
        }
        Ok(out)
    }

    /// Whether a resolved path is inside this workspace.
    ///
    /// The second line of defence, for paths that reached the filesystem by another
    /// route (a shell command's cwd, a tool that took a `PathBuf`).
    #[must_use]
    pub fn contains(&self, path: &Path) -> bool {
        let canon_root = self
            .root
            .canonicalize()
            .unwrap_or_else(|_| self.root.clone());
        let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        canon.starts_with(&canon_root)
    }
}

/// What `deliver(paths, summary)` records (§10.1).
///
/// Every path must be under `outputs/`. A deliverable pointing at `work/` would make
/// the output contract advisory — and the point of the contract is that a caller can
/// read `outputs/` and know that is the whole answer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Delivery {
    pub paths: Vec<String>,
    pub summary: String,
}

#[derive(Debug, thiserror::Error)]
pub enum DeliveryError {
    #[error("{path:?} is not under outputs/; only files in outputs/ can be delivered")]
    NotAnOutput { path: String },
    #[error("a delivery needs a summary saying what was produced")]
    NoSummary,
    #[error("a delivery needs at least one file")]
    Empty,
}

impl Delivery {
    /// Checks the contract.
    pub fn validate(&self, ws: &Workspace) -> Result<(), DeliveryError> {
        if self.paths.is_empty() {
            return Err(DeliveryError::Empty);
        }
        if self.summary.trim().is_empty() {
            return Err(DeliveryError::NoSummary);
        }
        for p in &self.paths {
            let resolved = ws
                .resolve(p, false)
                .map_err(|_| DeliveryError::NotAnOutput { path: p.clone() })?;
            let rel = resolved
                .strip_prefix(ws.root())
                .map_err(|_| DeliveryError::NotAnOutput { path: p.clone() })?;
            let first = rel
                .components()
                .next()
                .and_then(|c| c.as_os_str().to_str())
                .unwrap_or("");
            if first != Area::Outputs.dir() {
                return Err(DeliveryError::NotAnOutput { path: p.clone() });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws() -> Workspace {
        Workspace::new("/workspace/proj/sess", Quota::default())
    }

    #[test]
    fn the_layout_is_the_five_named_areas() {
        let dirs: Vec<&str> = Area::ALL.iter().map(|a| a.dir()).collect();
        assert_eq!(dirs, ["inbox", "work", "memory", "outputs", "logs"]);
    }

    #[test]
    fn a_normal_path_resolves_under_the_root() {
        let p = ws().resolve("work/analysis.py", true).unwrap();
        assert!(p.ends_with("work/analysis.py") || p.ends_with("work\\analysis.py"));
    }

    /// The attack the module exists for, in every spelling that reaches the string.
    #[test]
    fn traversal_out_of_the_workspace_is_refused() {
        for bad in [
            "../etc/passwd",
            "work/../../etc/passwd",
            "work/../../../../../../etc/passwd",
            "./work/../..",
        ] {
            assert!(
                matches!(ws().resolve(bad, true), Err(PathError::Escapes { .. })),
                "{bad} was not refused"
            );
        }
    }

    #[test]
    fn traversal_that_stays_inside_is_allowed() {
        // `work/a/../b` never leaves the tree, and refusing it would make ordinary
        // path arithmetic fail for no gain.
        let p = ws().resolve("work/a/../b.txt", true).unwrap();
        assert!(p.ends_with("work/b.txt") || p.ends_with("work\\b.txt"));
    }

    #[test]
    fn absolute_paths_are_refused_including_windows_drive_forms() {
        for bad in ["/etc/passwd", "C:/Windows", "C:Windows", "\\\\host\\share"] {
            assert!(
                matches!(
                    ws().resolve(bad, true),
                    Err(PathError::Absolute { .. }) | Err(PathError::Escapes { .. })
                ),
                "{bad} was not refused"
            );
        }
    }

    #[test]
    fn a_path_with_no_area_is_refused_with_a_fix_naming_the_areas() {
        let err = ws().resolve("analysis.py", true).unwrap_err();
        assert!(matches!(err, PathError::NoArea { .. }));
        assert!(err.fix().contains("work/"));
    }

    #[test]
    fn inbox_and_logs_are_read_only_to_the_agent() {
        assert!(
            ws().resolve("inbox/data.csv", false).is_ok(),
            "reading is fine"
        );
        assert!(matches!(
            ws().resolve("inbox/data.csv", true),
            Err(PathError::ReadOnly { .. })
        ));
        assert!(matches!(
            ws().resolve("logs/step.jsonl", true),
            Err(PathError::ReadOnly { .. }),
        ));
        for a in Area::ALL {
            assert_eq!(
                a.agent_writable(),
                matches!(a, Area::Work | Area::Memory | Area::Outputs)
            );
        }
    }

    #[test]
    fn every_path_error_carries_a_fix() {
        for bad in ["../x", "x.py", "inbox/x", "/abs"] {
            if let Err(e) = ws().resolve(bad, true) {
                assert!(!e.fix().is_empty(), "{bad} has no fix");
            }
        }
    }

    // ── The output contract ─────────────────────────────────────────────────

    #[test]
    fn a_delivery_of_outputs_is_accepted() {
        let d = Delivery {
            paths: vec!["outputs/report.md".into(), "outputs/figure.png".into()],
            summary: "the report and its figure".into(),
        };
        d.validate(&ws()).unwrap();
    }

    /// Without this the output contract is advisory, and a caller reading `outputs/`
    /// no longer knows that is the whole answer.
    #[test]
    fn delivering_a_scratch_file_is_refused() {
        let d = Delivery {
            paths: vec!["work/draft.md".into()],
            summary: "a draft".into(),
        };
        assert!(matches!(
            d.validate(&ws()),
            Err(DeliveryError::NotAnOutput { .. })
        ));
    }

    #[test]
    fn a_delivery_needs_a_summary_and_a_file() {
        let no_summary = Delivery {
            paths: vec!["outputs/a.md".into()],
            summary: "   ".into(),
        };
        assert!(matches!(
            no_summary.validate(&ws()),
            Err(DeliveryError::NoSummary)
        ));

        let empty = Delivery {
            paths: vec![],
            summary: "nothing".into(),
        };
        assert!(matches!(empty.validate(&ws()), Err(DeliveryError::Empty)));
    }

    #[test]
    fn a_traversal_dressed_as_an_output_is_refused() {
        let d = Delivery {
            paths: vec!["outputs/../../../etc/passwd".into()],
            summary: "x".into(),
        };
        assert!(d.validate(&ws()).is_err());
    }
}
