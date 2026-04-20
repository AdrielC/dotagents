//! **Pure-data diff model.** Given a [`CompiledPlan`] and an observed filesystem state (via a
//! [`FileSource`]), classify each op as create / update / no-change / conflict. The result is a
//! typed [`Diff`] that can be serialised, pretty-printed as a Terraform-style plan, or fed into
//! a CLI for `--dry-run` previews.
//!
//! The classifier lives in the pure crate — it reads files through a [`FileSource`] so it works
//! identically against the real FS ([`crate::RealFileSource`]) or an in-memory mock
//! ([`crate::MemFileSource`]).

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::compile::{CompiledPlan, FsOp};
use crate::dialect::FileSource;

/// One classified change against observed state.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum DiffEntry {
    /// Path doesn't exist yet; the plan would create it.
    Create {
        path: PathBuf,
        kind: EntryKind,
        /// Byte count of the content to be written (files only).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bytes: Option<usize>,
    },
    /// Path exists and matches the planned content byte-for-byte — apply would skip.
    NoChange { path: PathBuf, kind: EntryKind },
    /// Path exists but content (for files) or target (for links) differs from the plan.
    Update {
        path: PathBuf,
        kind: EntryKind,
        /// Human-readable summary of what would change.
        summary: String,
    },
    /// Path exists and is of a different kind than the plan expects (e.g. directory vs file).
    Conflict { path: PathBuf, reason: String },
}

impl DiffEntry {
    #[must_use]
    pub fn path(&self) -> &PathBuf {
        match self {
            DiffEntry::Create { path, .. }
            | DiffEntry::NoChange { path, .. }
            | DiffEntry::Update { path, .. }
            | DiffEntry::Conflict { path, .. } => path,
        }
    }
}

/// What kind of filesystem entry the entry concerns.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    Directory,
    File,
    Link,
}

/// A plan's worth of diff entries.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Diff {
    pub entries: Vec<DiffEntry>,
}

impl Diff {
    #[must_use]
    pub fn is_empty_change_set(&self) -> bool {
        self.entries
            .iter()
            .all(|e| matches!(e, DiffEntry::NoChange { .. }))
    }

    /// Terraform-style: `+ create / ~ update / = no-change / ! conflict`. Paths are relative to
    /// `strip_prefix` when supplied.
    #[must_use]
    pub fn render_tf_style(&self, strip_prefix: Option<&std::path::Path>) -> String {
        let mut out = String::new();
        let mut counts = [0usize; 4]; // create, update, nochange, conflict
        for e in &self.entries {
            let raw = e.path();
            let rel = match strip_prefix {
                Some(p) => raw.strip_prefix(p).unwrap_or(raw).display().to_string(),
                None => raw.display().to_string(),
            };
            let (sigil, bucket) = match e {
                DiffEntry::Create { .. } => ("+", 0),
                DiffEntry::Update { summary, .. } => {
                    out.push_str(&format!("~ {rel}  ({summary})\n"));
                    counts[1] += 1;
                    continue;
                }
                DiffEntry::NoChange { .. } => ("=", 2),
                DiffEntry::Conflict { reason, .. } => {
                    out.push_str(&format!("! {rel}  ({reason})\n"));
                    counts[3] += 1;
                    continue;
                }
            };
            out.push_str(&format!("{sigil} {rel}\n"));
            counts[bucket] += 1;
        }
        out.push_str(&format!(
            "\nPlan: {} to create, {} to update, {} unchanged, {} conflict.\n",
            counts[0], counts[1], counts[2], counts[3]
        ));
        out
    }
}

/// Compute a [`Diff`] by comparing each op in `plan` against what `fs` reports.
pub fn compute(plan: &CompiledPlan, fs: &dyn FileSource) -> Diff {
    let mut entries = Vec::with_capacity(plan.ops.len());
    for op in &plan.ops {
        entries.push(classify(op, fs));
    }
    Diff { entries }
}

fn classify(op: &FsOp, fs: &dyn FileSource) -> DiffEntry {
    match op {
        FsOp::MkdirP { path } => {
            if fs.is_dir(path) {
                DiffEntry::NoChange {
                    path: path.clone(),
                    kind: EntryKind::Directory,
                }
            } else if fs.is_file(path) {
                DiffEntry::Conflict {
                    path: path.clone(),
                    reason: "file exists where a directory is planned".into(),
                }
            } else {
                DiffEntry::Create {
                    path: path.clone(),
                    kind: EntryKind::Directory,
                    bytes: None,
                }
            }
        }
        FsOp::WriteFile { path, content, .. } => {
            if fs.is_dir(path) {
                DiffEntry::Conflict {
                    path: path.clone(),
                    reason: "directory exists where a file is planned".into(),
                }
            } else if fs.is_file(path) {
                match fs.read_to_string(path) {
                    Ok(existing) if existing == *content => DiffEntry::NoChange {
                        path: path.clone(),
                        kind: EntryKind::File,
                    },
                    Ok(existing) => DiffEntry::Update {
                        path: path.clone(),
                        kind: EntryKind::File,
                        summary: format!("{} → {} bytes", existing.len(), content.len()),
                    },
                    Err(_) => DiffEntry::Update {
                        path: path.clone(),
                        kind: EntryKind::File,
                        summary: "unreadable; would overwrite".into(),
                    },
                }
            } else {
                DiffEntry::Create {
                    path: path.clone(),
                    kind: EntryKind::File,
                    bytes: Some(content.len()),
                }
            }
        }
        FsOp::Link(link) => {
            if fs.exists(&link.dest) {
                // We don't try to compare link target via the abstract FileSource; defer to the
                // IO layer if it wants to be more precise.
                DiffEntry::Update {
                    path: link.dest.clone(),
                    kind: EntryKind::Link,
                    summary: format!("relink → {}", link.source.display()),
                }
            } else {
                DiffEntry::Create {
                    path: link.dest.clone(),
                    kind: EntryKind::Link,
                    bytes: None,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dialect::MemFileSource;

    #[test]
    fn create_entries_when_target_is_empty() {
        let plan = CompiledPlan {
            ops: vec![
                FsOp::MkdirP {
                    path: "/p/.cursor/rules".into(),
                },
                FsOp::WriteFile {
                    path: "/p/.cursor/rules/global--foo.mdc".into(),
                    overwrite: false,
                    content: "body".into(),
                },
            ],
            warnings: vec![],
        };
        let fs = MemFileSource::new();
        let diff = compute(&plan, &fs);
        assert_eq!(diff.entries.len(), 2);
        assert!(matches!(diff.entries[0], DiffEntry::Create { .. }));
        assert!(matches!(
            diff.entries[1],
            DiffEntry::Create {
                kind: EntryKind::File,
                bytes: Some(4),
                ..
            }
        ));
    }

    #[test]
    fn nochange_when_content_matches() {
        let plan = CompiledPlan {
            ops: vec![FsOp::WriteFile {
                path: "/p/.foo/bar".into(),
                overwrite: false,
                content: "same".into(),
            }],
            warnings: vec![],
        };
        let mut fs = MemFileSource::new();
        fs.insert("/p/.foo/bar", "same");
        let diff = compute(&plan, &fs);
        assert!(matches!(diff.entries[0], DiffEntry::NoChange { .. }));
        assert!(diff.is_empty_change_set());
    }

    #[test]
    fn update_when_content_differs() {
        let plan = CompiledPlan {
            ops: vec![FsOp::WriteFile {
                path: "/p/.foo/bar".into(),
                overwrite: false,
                content: "new content".into(),
            }],
            warnings: vec![],
        };
        let mut fs = MemFileSource::new();
        fs.insert("/p/.foo/bar", "old");
        let diff = compute(&plan, &fs);
        assert!(matches!(diff.entries[0], DiffEntry::Update { .. }));
    }
}
