//! **Ingest — one function, every agent.**
//!
//! Before this module existed there was a hand-rolled `claude::ingest` walker. Now every
//! per-agent walker is auto-generated: [`ingest_dir`] asks a [`Dialect`] to pull each artifact
//! kind, stitches them into an [`AgentsTree`], and returns the bundle.
//!
//! Real IO goes through [`RealFileSource`]; tests use [`MemFileSource`]; anyone can stub the
//! [`FileSource`] trait if they want to ingest from a tar stream, a git blob, a web response,
//! whatever.

use std::path::{Path, PathBuf};

use agentz_core::dialect::{Dialect, FileSource, RealFileSource};
use agentz_core::dialects;
use agentz_core::tree::{AgentsTree, ScopeKind};
use agentz_core::AgentId;
use thiserror::Error;

/// Back-compat shim: Claude-specific convenience functions. Prefer [`ingest_dir`] for new code.
pub mod claude;

/// Per-ingest configuration shared across every agent.
#[derive(Clone, Debug)]
pub struct IngestOptions {
    /// Also consult the **repo root** (parent of the per-agent dir) for files conventionally
    /// stored there — `.mcp.json`, `.claudeignore`, `.cursorignore`, `CLAUDE.md`, etc.
    pub include_repo_root: bool,
    /// After ingest, duplicate every ignore node so listed agents also receive those patterns.
    /// Lets `.claude → .cursor` migrations produce a `.cursorignore` alongside `.claudeignore`.
    /// Empty (default) = no mirroring.
    pub mirror_ignores_to: Vec<AgentId>,
}

impl Default for IngestOptions {
    fn default() -> Self {
        Self {
            include_repo_root: true,
            mirror_ignores_to: Vec::new(),
        }
    }
}

/// Result of an ingest pass.
#[derive(Clone, Debug)]
pub struct IngestReport {
    pub tree: AgentsTree,
    pub unknown_paths: Vec<PathBuf>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Error)]
pub enum IngestError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("not a directory: {0}")]
    NotADirectory(PathBuf),
    #[error("path does not exist: {0}")]
    Missing(PathBuf),
}

/// Ingest `agent`'s configuration directory from the real filesystem.
pub fn ingest_dir(
    agent: AgentId,
    root: &Path,
    opts: &IngestOptions,
) -> Result<IngestReport, IngestError> {
    ingest_dir_with(&RealFileSource, dialects::get(agent), root, opts)
}

/// Ingest via an arbitrary [`FileSource`] — tests, tarballs, MCP streams, etc.
pub fn ingest_dir_with(
    fs: &dyn FileSource,
    dialect: &dyn Dialect,
    root: &Path,
    opts: &IngestOptions,
) -> Result<IngestReport, IngestError> {
    if !fs.exists(root) {
        return Err(IngestError::Missing(root.to_path_buf()));
    }
    if !fs.is_dir(root) {
        return Err(IngestError::NotADirectory(root.to_path_buf()));
    }

    let mut children: Vec<AgentsTree> = Vec::new();
    let mut unknown: Vec<PathBuf> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    let rules = dialect.ingest_rules(fs, root)?;
    if !rules.is_empty() {
        children.push(AgentsTree::Rules(rules));
    }

    let skills = dialect.ingest_skills(fs, root)?;
    if !skills.is_empty() {
        children.push(AgentsTree::Skills(skills));
    }

    let agents = dialect.ingest_agents(fs, root)?;
    if !agents.is_empty() {
        children.push(AgentsTree::Agents(agents));
    }

    let settings = dialect.ingest_settings(fs, root)?;
    if !settings.is_empty() {
        children.push(AgentsTree::Settings(settings));
    }

    let hooks = dialect.ingest_hooks(fs, root)?;
    if !hooks.is_empty() {
        children.push(AgentsTree::Hooks(hooks));
    }

    // The repo root (parent of `.claude/`, `.cursor/`, …) is where ignore files and the MCP
    // config usually live. Defer that lookup unless the caller opts in.
    if opts.include_repo_root {
        if let Some(repo_root) = root.parent() {
            // Check both repo root and the config dir itself for ignore files — Claude in
            // particular has `.claudeignore` at the repo root, not inside `.claude/`.
            for dir in [repo_root, root] {
                for (kind, patterns) in dialect.ingest_ignore(fs, dir)? {
                    if !patterns.is_empty() {
                        children.push(AgentsTree::Ignore {
                            agent: dialect.agent(),
                            kind,
                            patterns,
                        });
                    }
                }
            }
            if let Some(mcp) = dialect.ingest_mcp(fs, repo_root)? {
                children.push(AgentsTree::Mcp(mcp));
            }

            // Claude CLAUDE.md — stored at repo root or inside `.claude/`. Not yet a Dialect
            // method; do it here directly for Claude Code only.
            if dialect.agent() == AgentId::ClaudeCode {
                for candidate in [repo_root.join("CLAUDE.md"), root.join("CLAUDE.md")] {
                    if fs.is_file(&candidate) {
                        match fs.read_to_string(&candidate) {
                            Ok(body) => children.push(AgentsTree::TextFile {
                                name: "CLAUDE.md".into(),
                                body,
                            }),
                            Err(e) => warnings.push(format!("read {}: {e}", candidate.display())),
                        }
                        break;
                    }
                }
            }
        }
    } else {
        // Config-dir-scoped ignore files only.
        for (kind, patterns) in dialect.ingest_ignore(fs, root)? {
            if !patterns.is_empty() {
                children.push(AgentsTree::Ignore {
                    agent: dialect.agent(),
                    kind,
                    patterns,
                });
            }
        }
    }

    // Record unknown top-level entries under the config dir so callers can audit.
    if let Ok(entries) = fs.read_dir(root) {
        for path in entries {
            if fs.is_file(&path) {
                let file = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default()
                    .to_string();
                if !is_known_top_level(dialect.agent(), &file) {
                    unknown.push(path);
                }
            }
        }
    }

    if !opts.mirror_ignores_to.is_empty() {
        mirror_ignore_nodes(&mut children, &opts.mirror_ignores_to);
    }

    Ok(IngestReport {
        tree: AgentsTree::Scope {
            kind: ScopeKind::Global,
            children,
        },
        unknown_paths: unknown,
        warnings,
    })
}

/// For every existing `Ignore` node, append equivalents for each mirror-target that can
/// represent the same `IgnoreKind`. Source-agent nodes are left untouched.
fn mirror_ignore_nodes(children: &mut Vec<AgentsTree>, targets: &[AgentId]) {
    let originals: Vec<(AgentId, agentz_core::model::IgnoreKind, Vec<String>)> = children
        .iter()
        .filter_map(|c| match c {
            AgentsTree::Ignore {
                agent,
                kind,
                patterns,
            } => Some((*agent, *kind, patterns.clone())),
            _ => None,
        })
        .collect();
    for (src_agent, kind, patterns) in originals {
        for &t in targets {
            if t == src_agent {
                continue;
            }
            if t.spec().ignore_filename(kind).is_none() {
                continue;
            }
            let already_present = children.iter().any(|c| {
                matches!(
                    c,
                    AgentsTree::Ignore { agent, kind: k, .. }
                        if *agent == t && *k == kind
                )
            });
            if !already_present {
                children.push(AgentsTree::Ignore {
                    agent: t,
                    kind,
                    patterns: patterns.clone(),
                });
            }
        }
    }
}

/// Top-level files the dialect already consumes through `AgentSpec.settings`, memory, etc. Keeps
/// the "unknown files" list honest without us having to enumerate by hand in every dialect.
fn is_known_top_level(agent: AgentId, name: &str) -> bool {
    let spec = agent.spec();
    for n in [
        spec.settings.base,
        spec.settings.local,
        spec.settings.managed,
    ]
    .into_iter()
    .flatten()
    {
        if n == name {
            return true;
        }
    }
    // Common memory + mcp manifest.
    matches!(name, "CLAUDE.md" | ".mcp.json")
}
