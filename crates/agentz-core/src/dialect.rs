//! **`Dialect` — one trait per agent, covering both emit and ingest.**
//!
//! Before this module existed, per-agent behaviour was split:
//!
//! - Emission lived in `compile.rs` as a bag of `emit_rules` / `emit_skills` / … free functions
//!   that switch on [`AgentSpec`] table rows.
//! - Ingest lived in the `agentz` crate as a hand-rolled walker for Claude only.
//!
//! That's two parallel pipelines doing the same thing in opposite directions with no compiler
//! help keeping them in sync. [`Dialect`] collapses them into a single trait with paired
//! `emit_*` / `ingest_*` methods per artifact family (rules, skills, agents, settings, hooks,
//! ignores, mcp). Each agent has exactly **one** `impl Dialect for …` block; adding a new agent
//! or a new artifact is a trait method, not a cross-file refactor.
//!
//! ## Purity
//!
//! The crate invariant is **no IO in `agentz-core`**. Ingest needs to read files, so instead of
//! touching `std::fs` we thread a [`FileSource`] handle through every `ingest_*` call. Callers
//! (the `agentz` IO crate) supply [`RealFileSource`]; tests use [`MemFileSource`]; anyone else
//! can stub the trait however they like. The trait body is a tiny five-method surface — `read_dir`,
//! `read_to_string`, `is_dir`, `is_file`, `exists`.
//!
//! ## Defaults
//!
//! Every `emit_*` / `ingest_*` method defaults to "do nothing / find nothing" so an agent that
//! doesn't support a concept (e.g. Gemini has no rules, Cursor has no subagents) just doesn't
//! override that pair. Adding a new method later is a non-breaking change.

use std::io;
use std::path::{Path, PathBuf};

use crate::compile::{CompileContext, FsOp};
use crate::model::{AgentId, IgnoreKind};
use crate::tree::{AgentNode, HookBinding, RuleNode, SettingsNode, SkillNode};

/// Minimal filesystem capability the ingest side needs. Implementors decide whether this reads
/// from the real filesystem, from a `tar` stream, from a baked-in `include_dir!`, or from a
/// `HashMap` (tests). Nothing else in the core crate uses `std::fs`.
pub trait FileSource: Send + Sync {
    /// List the direct children of a directory. Order is unspecified — callers that need
    /// determinism must sort.
    fn read_dir(&self, path: &Path) -> io::Result<Vec<PathBuf>>;
    /// Read a file to a UTF-8 string.
    fn read_to_string(&self, path: &Path) -> io::Result<String>;
    /// `true` if `path` names a directory.
    fn is_dir(&self, path: &Path) -> bool;
    /// `true` if `path` names a file.
    fn is_file(&self, path: &Path) -> bool;
    /// `true` if `path` exists at all.
    fn exists(&self, path: &Path) -> bool {
        self.is_dir(path) || self.is_file(path)
    }
}

/// Real-filesystem [`FileSource`]. Lives here (in the pure crate) because it's a thin wrapper —
/// the crate still doesn't touch IO unless you hand one of these in.
#[derive(Copy, Clone, Debug, Default)]
pub struct RealFileSource;

impl FileSource for RealFileSource {
    fn read_dir(&self, path: &Path) -> io::Result<Vec<PathBuf>> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(path)? {
            out.push(entry?.path());
        }
        Ok(out)
    }
    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        std::fs::read_to_string(path)
    }
    fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }
    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }
}

/// In-memory [`FileSource`] for tests. Paths are stored as-is; `read_dir(p)` returns every entry
/// whose parent is `p`.
#[derive(Clone, Debug, Default)]
pub struct MemFileSource {
    files: std::collections::BTreeMap<PathBuf, String>,
}

impl MemFileSource {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a file. Parent directories are implicit.
    pub fn insert(&mut self, path: impl Into<PathBuf>, content: impl Into<String>) {
        self.files.insert(path.into(), content.into());
    }
}

impl FileSource for MemFileSource {
    fn read_dir(&self, path: &Path) -> io::Result<Vec<PathBuf>> {
        let mut out = std::collections::BTreeSet::new();
        for p in self.files.keys() {
            if let Some(parent) = p.parent() {
                if parent == path {
                    out.insert(p.clone());
                }
            }
            // Emit synthetic directory entries for deeper paths.
            let mut cur = p.parent();
            while let Some(c) = cur {
                if let Some(gp) = c.parent() {
                    if gp == path {
                        out.insert(c.to_path_buf());
                        break;
                    }
                }
                cur = c.parent();
            }
        }
        Ok(out.into_iter().collect())
    }
    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, path.display().to_string()))
    }
    fn is_dir(&self, path: &Path) -> bool {
        // Any key whose ancestor equals `path` counts as a dir.
        self.files.keys().any(|p| {
            let mut cur = p.parent();
            while let Some(c) = cur {
                if c == path {
                    return true;
                }
                cur = c.parent();
            }
            false
        })
    }
    fn is_file(&self, path: &Path) -> bool {
        self.files.contains_key(path)
    }
}

/// The per-agent dialect. One concrete impl per built-in agent; see [`crate::dialects`].
pub trait Dialect: Send + Sync {
    /// Which [`AgentId`] this dialect is for.
    fn agent(&self) -> AgentId;

    /// Project-relative config directory **including** the leading dot (e.g. `".cursor"`).
    /// Defaults to `self.agent().config_dir()` — override only if you really need to.
    fn config_dir(&self) -> &'static str {
        self.agent().config_dir()
    }

    // ── Emission (pure: input → Vec<FsOp>, no IO) ─────────────────────────────

    /// Emit ops for a bundle of rules under a scope prefix (`"global"`, `"my-project"`, …).
    fn emit_rules(&self, _ctx: &CompileContext, _scope: &str, _rules: &[RuleNode]) -> Vec<FsOp> {
        Vec::new()
    }

    /// Emit ops for a bundle of skills.
    fn emit_skills(&self, _ctx: &CompileContext, _skills: &[SkillNode]) -> Vec<FsOp> {
        Vec::new()
    }

    /// Emit ops for a bundle of subagents.
    fn emit_agents(&self, _ctx: &CompileContext, _agents: &[AgentNode]) -> Vec<FsOp> {
        Vec::new()
    }

    /// Emit ops for per-scope settings files.
    fn emit_settings(&self, _ctx: &CompileContext, _settings: &[SettingsNode]) -> Vec<FsOp> {
        Vec::new()
    }

    /// Emit ops for hooks.
    fn emit_hooks(&self, _ctx: &CompileContext, _hooks: &[HookBinding]) -> Vec<FsOp> {
        Vec::new()
    }

    /// Emit ops for an ignore file of the given kind.
    fn emit_ignore(
        &self,
        _ctx: &CompileContext,
        _kind: IgnoreKind,
        _patterns: &[String],
    ) -> Vec<FsOp> {
        Vec::new()
    }

    /// Emit ops for an MCP server block.
    fn emit_mcp(&self, _ctx: &CompileContext, _mcp: &serde_json::Value) -> Vec<FsOp> {
        Vec::new()
    }

    // ── Ingest (IO via FileSource; pure wrt the real FS) ──────────────────────

    /// Pull rules out of `<root>/<subdir?>`.
    fn ingest_rules(&self, _fs: &dyn FileSource, _root: &Path) -> io::Result<Vec<RuleNode>> {
        Ok(Vec::new())
    }

    /// Pull skills.
    fn ingest_skills(&self, _fs: &dyn FileSource, _root: &Path) -> io::Result<Vec<SkillNode>> {
        Ok(Vec::new())
    }

    /// Pull subagents.
    fn ingest_agents(&self, _fs: &dyn FileSource, _root: &Path) -> io::Result<Vec<AgentNode>> {
        Ok(Vec::new())
    }

    /// Pull per-scope settings files.
    fn ingest_settings(&self, _fs: &dyn FileSource, _root: &Path) -> io::Result<Vec<SettingsNode>> {
        Ok(Vec::new())
    }

    /// Pull hooks (Cursor's `hooks.json`; Claude embeds in `settings.json` so typically empty here).
    fn ingest_hooks(&self, _fs: &dyn FileSource, _root: &Path) -> io::Result<Vec<HookBinding>> {
        Ok(Vec::new())
    }

    /// Pull (kind, patterns) pairs for each ignore file.
    fn ingest_ignore(
        &self,
        _fs: &dyn FileSource,
        _root: &Path,
    ) -> io::Result<Vec<(IgnoreKind, Vec<String>)>> {
        Ok(Vec::new())
    }

    /// Pull the MCP servers block if present.
    fn ingest_mcp(
        &self,
        _fs: &dyn FileSource,
        _root: &Path,
    ) -> io::Result<Option<serde_json::Value>> {
        Ok(None)
    }
}
