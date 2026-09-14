//! The agent's own folder, on disk (guide §4.3; `harness::workspace`).
//!
//! Every conversation gets one workspace and the four `fs` tools over it. This module
//! is the IO half: the harness decides *whether* a path is allowed, this performs the
//! read or write once it has.
//!
//! # Why the resolver is not re-implemented here
//!
//! `Workspace::resolve` does the check lexically — `..` is consumed against a stack
//! and a path that pops past the root is refused **before** it touches the
//! filesystem. Resolving on disk first would follow a symlink the agent planted,
//! which is the exact escape being guarded. So every path in this file goes through
//! `resolve` and nothing here builds a `PathBuf` by concatenation.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use harness::workspace::{Area, Quota, Workspace};

/// Cap on a single file read, so a file the agent wrote in a loop cannot be used to
/// blow the context budget from inside the workspace.
const MAX_READ_BYTES: usize = 256 * 1024;

/// Where a conversation's workspace lives.
///
/// Per conversation, not per run: an agent that loses its notes between turns is an
/// agent with no memory of its own work.
#[must_use]
pub fn workspace_for(root: &Path, conversation_id: uuid::Uuid) -> Workspace {
    Workspace::new(
        root.join("agents").join(conversation_id.to_string()),
        Quota::default(),
    )
}

/// Creates the five areas. Idempotent.
pub async fn ensure(ws: &Workspace) -> std::io::Result<()> {
    for dir in ws.areas() {
        tokio::fs::create_dir_all(dir).await?;
    }
    Ok(())
}

/// Runs one `fs` tool call. Returns `(text, is_error)`.
///
/// Errors come back as text the model can act on, carrying the `fix` the harness
/// already wrote — a path refusal the model cannot read just gets retried verbatim.
pub async fn execute(ws: &Workspace, name: &str, args: &Map<String, Value>) -> (String, bool) {
    let path_arg = args.get("path").and_then(Value::as_str);

    match name {
        "write_file" => {
            let (Some(path), Some(content)) =
                (path_arg, args.get("content").and_then(Value::as_str))
            else {
                return ("write_file needs `path` and `content`".into(), true);
            };
            let resolved = match ws.resolve(path, true) {
                Ok(p) => p,
                Err(e) => return (format!("{e}. {}", e.fix()), true),
            };
            if let Some(parent) = resolved.parent() {
                if let Err(e) = tokio::fs::create_dir_all(parent).await {
                    return (format!("could not create {}: {e}", parent.display()), true);
                }
            }
            match tokio::fs::write(&resolved, content).await {
                Ok(()) => (format!("wrote {} bytes to {path}", content.len()), false),
                Err(e) => (format!("could not write {path}: {e}"), true),
            }
        }

        "read_file" => {
            let Some(path) = path_arg else {
                return ("read_file needs `path`".into(), true);
            };
            // `write: false` — reading is allowed from every area, including the
            // read-only ones.
            let resolved = match ws.resolve(path, false) {
                Ok(p) => p,
                Err(e) => return (format!("{e}. {}", e.fix()), true),
            };
            match tokio::fs::read(&resolved).await {
                Ok(bytes) => {
                    let cut = bytes.len() > MAX_READ_BYTES;
                    let end = if cut { MAX_READ_BYTES } else { bytes.len() };
                    let text = String::from_utf8_lossy(&bytes[..end]).into_owned();
                    if cut {
                        (
                            format!(
                                "{text}\n\n[truncated: {path} is {} bytes, showing the first {MAX_READ_BYTES}]",
                                bytes.len()
                            ),
                            false,
                        )
                    } else {
                        (text, false)
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (
                    format!("{path} does not exist. Use list_files to see what does"),
                    true,
                ),
                Err(e) => (format!("could not read {path}: {e}"), true),
            }
        }

        "list_files" => {
            let base = match path_arg.filter(|p| !p.trim().is_empty()) {
                Some(p) => match ws.resolve(p, false) {
                    Ok(r) => r,
                    Err(e) => return (format!("{e}. {}", e.fix()), true),
                },
                None => ws.root().to_path_buf(),
            };
            let mut found: Vec<String> = Vec::new();
            collect(&base, ws.root(), &mut found).await;
            found.sort();
            if found.is_empty() {
                ("the workspace is empty".into(), false)
            } else {
                (found.join("\n"), false)
            }
        }

        "delete_file" => {
            let Some(path) = path_arg else {
                return ("delete_file needs `path`".into(), true);
            };
            let resolved = match ws.resolve(path, true) {
                Ok(p) => p,
                Err(e) => return (format!("{e}. {}", e.fix()), true),
            };
            match tokio::fs::remove_file(&resolved).await {
                Ok(()) => (format!("deleted {path}"), false),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    (format!("{path} does not exist"), true)
                }
                Err(e) => (format!("could not delete {path}: {e}"), true),
            }
        }

        other => (format!("{other} is not a workspace tool"), true),
    }
}

/// Walks a directory, reporting paths relative to the workspace root with sizes.
///
/// Iterative rather than recursive: a directory tree the agent built could be
/// arbitrarily deep, and an async recursion would need boxing to compile anyway.
async fn collect(base: &Path, root: &Path, out: &mut Vec<String>) {
    let mut stack: Vec<PathBuf> = vec![base.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(mut entries) = tokio::fs::read_dir(&dir).await else {
            continue;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            let Ok(meta) = entry.metadata().await else {
                continue;
            };
            if meta.is_dir() {
                stack.push(path);
            } else {
                let rel = path.strip_prefix(root).unwrap_or(&path);
                out.push(format!(
                    "{} ({} bytes)",
                    rel.to_string_lossy().replace('\\', "/"),
                    meta.len()
                ));
            }
        }
    }
}

/// Whether a tool name belongs to the workspace rather than the platform.
#[must_use]
pub fn is_workspace_tool(namespace: &str) -> bool {
    namespace == "fs"
}

/// The areas, for the charter. Telling the agent where it may write is cheaper than
/// letting it find out by rejection.
#[must_use]
pub fn areas_hint() -> String {
    let writable: Vec<&str> = Area::ALL
        .iter()
        .filter(|a| a.agent_writable())
        .map(|a| a.dir())
        .collect();
    format!(
        "You have your own folder. Write with `write_file` and read it back with \
         `read_file`; `list_files` shows what is there. Writable areas: {}. Notes you \
         want after a compaction belong in `memory/`; working data in `work/`; \
         anything you want a human to read in `outputs/`.",
        writable.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    /// A throwaway root that cleans itself up.
    ///
    /// Hand-rolled rather than pulling in `tempfile`: one new dependency to get a
    /// directory with a random name is not a trade worth making.
    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!("tbot-ws-{}", Uuid::new_v4()));
            std::fs::create_dir_all(&p).expect("temp root");
            Self(p)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn temp_ws() -> (TempRoot, Workspace) {
        let dir = TempRoot::new();
        let ws = workspace_for(dir.path(), Uuid::nil());
        (dir, ws)
    }

    fn args(pairs: &[(&str, &str)]) -> Map<String, Value> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), Value::String((*v).to_string())))
            .collect()
    }

    #[tokio::test]
    async fn the_agent_can_write_read_list_and_delete_its_own_files() {
        let (_d, ws) = temp_ws();
        ensure(&ws).await.unwrap();

        let (msg, err) = execute(
            &ws,
            "write_file",
            &args(&[
                ("path", "work/notes.md"),
                ("content", "momentum looks weak"),
            ]),
        )
        .await;
        assert!(!err, "{msg}");

        let (body, err) = execute(&ws, "read_file", &args(&[("path", "work/notes.md")])).await;
        assert!(!err);
        assert_eq!(body, "momentum looks weak");

        let (listing, err) = execute(&ws, "list_files", &Map::new()).await;
        assert!(!err);
        assert!(listing.contains("work/notes.md"), "{listing}");

        let (msg, err) = execute(&ws, "delete_file", &args(&[("path", "work/notes.md")])).await;
        assert!(!err, "{msg}");
        let (_, err) = execute(&ws, "read_file", &args(&[("path", "work/notes.md")])).await;
        assert!(err, "a deleted file should not read back");
    }

    /// The containment that makes "its own folder" mean something. The refusal is
    /// lexical, so it happens before anything touches the disk.
    #[tokio::test]
    async fn it_cannot_reach_outside_its_own_folder() {
        let (_d, ws) = temp_ws();
        ensure(&ws).await.unwrap();
        for escape in [
            "../../../etc/passwd",
            "work/../../other-agent/memory/notes.md",
            "/etc/passwd",
            "C:/Windows/System32/config",
        ] {
            let (msg, err) = execute(
                &ws,
                "write_file",
                &args(&[("path", escape), ("content", "x")]),
            )
            .await;
            assert!(err, "{escape} should be refused, got: {msg}");
        }
    }

    /// Every refusal carries the fix the harness already wrote. A path error the
    /// model cannot act on just gets retried verbatim.
    #[tokio::test]
    async fn a_refusal_tells_the_model_what_to_do_instead() {
        let (_d, ws) = temp_ws();
        ensure(&ws).await.unwrap();
        let (msg, err) = execute(
            &ws,
            "write_file",
            &args(&[("path", "notes.md"), ("content", "x")]),
        )
        .await;
        assert!(err);
        assert!(
            msg.contains("work/") || msg.contains("area"),
            "the refusal should name the areas: {msg}"
        );
    }

    #[tokio::test]
    async fn the_read_only_areas_refuse_writes_but_allow_reads() {
        let (_d, ws) = temp_ws();
        ensure(&ws).await.unwrap();
        let (_, err) = execute(
            &ws,
            "write_file",
            &args(&[("path", "inbox/task.md"), ("content", "x")]),
        )
        .await;
        assert!(err, "inbox is read-only to the agent");

        // Placed by the harness, read by the agent.
        tokio::fs::write(ws.root().join("inbox").join("task.md"), "the brief")
            .await
            .unwrap();
        let (body, err) = execute(&ws, "read_file", &args(&[("path", "inbox/task.md")])).await;
        assert!(!err, "{body}");
        assert_eq!(body, "the brief");
    }

    /// Two conversations are two folders. Sharing one would let a later agent read an
    /// earlier one's working notes.
    #[test]
    fn each_conversation_gets_its_own_folder() {
        let dir = TempRoot::new();
        let a = workspace_for(dir.path(), Uuid::new_v4());
        let b = workspace_for(dir.path(), Uuid::new_v4());
        assert_ne!(a.root(), b.root());
        assert!(!a.contains(b.root()));
    }
}
