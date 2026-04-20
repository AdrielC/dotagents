//! **Repos and workspaces.**
//!
//! A [`Repo`] is a named *catalogue* of [`AgentsTree`] fragments — the same shape Claude Code's
//! plugin marketplace (`.claude-plugin/marketplace.json`) serves, and the same shape a `.mdc`
//! rules directory, a `SKILL.md` folder tree, or an inline `include!(..)` bundle takes. Every
//! source resolves to an AST, so downstream code is IO-agnostic: `compile` doesn't care whether
//! a rule came from a file, a git repo, or a literal string.
//!
//! A [`Workspace`] pins one git checkout (or plain directory) as the *target* of a compile and
//! declares which [`Repo`]s are enabled for it. [`Workspace::materialize`] folds them into a
//! single [`AgentsTree`] the compiler can consume.
//!
//! ## Purity
//!
//! The types here are pure data: [`RepoSource`] describes **where** to fetch, not **how**, and
//! [`Workspace::materialize`] is a deterministic tree-assembly pass. Fetch logic for `Git` /
//! `Http` sources belongs in an IO crate (e.g. `agentz`), layered on top.

use std::collections::BTreeMap;
use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::id::ProjectKey;
use crate::tree::{AgentsTree, ScopeKind};

/// Stable identifier for a [`Repo`] — kebab-case by convention; used as a scope prefix when the
/// repo contributes prefixed rules.
#[derive(
    Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct RepoId(pub String);

impl RepoId {
    #[must_use]
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for RepoId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl From<String> for RepoId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl std::fmt::Display for RepoId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Where a [`Repo`] loads from. Pure descriptor — no IO. An IO layer maps each variant to bytes
/// (filesystem read, git clone, HTTP GET) and hands back the parsed [`AgentsTree`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RepoSource {
    /// A directory on disk containing rule/skill/plugin files. The IO layer walks the directory
    /// with a parser (e.g. [`crate::parser::cursor`]).
    Local { path: PathBuf },
    /// A git URL + optional revision. Cloned by the IO layer; content parsed like [`Self::Local`].
    Git {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rev: Option<String>,
        /// Subdirectory within the checkout that holds the repo content. Empty = repo root.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subdir: Option<PathBuf>,
    },
    /// Content embedded directly in the `Repo` value (tests, tiny bundled repos).
    Embedded,
}

/// A named bundle of [`AgentsTree`] content. Analogous to a Claude plugin (`plugin.json` + its
/// `skills/`, `agents/`, `commands/`, `hooks/`, `.mcp.json`) or an `awesome-cursorrules` directory.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Repo {
    pub id: RepoId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub source: RepoSource,
    /// The content this repo contributes. For [`RepoSource::Embedded`] this is the whole payload;
    /// for `Local` / `Git` it's the parsed result the IO layer materialised.
    pub tree: AgentsTree,
}

impl Repo {
    /// Build an embedded repo with an inline tree (most useful in tests).
    #[must_use]
    pub fn embedded(id: impl Into<RepoId>, tree: AgentsTree) -> Self {
        Self {
            id: id.into(),
            description: None,
            version: None,
            source: RepoSource::Embedded,
            tree,
        }
    }
}

/// The full compile target: one project + the set of [`Repo`]s enabled for it + the user's own
/// project-level tree.
///
/// Call [`Self::materialize`] to get a single [`AgentsTree`] ready for [`crate::compile::compile`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Workspace {
    /// Filesystem path to the project checkout (== `CompileContext::project_path`).
    pub root: PathBuf,
    /// Project key used for Cursor rule scope prefixes etc.
    pub project_key: ProjectKey,
    /// The user's own tree for this workspace — typically `AgentsTree::global([...])` plus a
    /// `ScopeKind::Project` scope with their repo-level overrides.
    pub defaults: AgentsTree,
    /// Repos enabled for this workspace, keyed by id. `BTreeMap` for deterministic merge order.
    #[serde(default)]
    pub repos: BTreeMap<RepoId, Repo>,
}

impl Workspace {
    /// Create an empty workspace whose `defaults` is an empty [`ScopeKind::Global`] scope.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>, project_key: impl Into<ProjectKey>) -> Self {
        Self {
            root: root.into(),
            project_key: project_key.into(),
            defaults: AgentsTree::Scope {
                kind: ScopeKind::Global,
                children: Vec::new(),
            },
            repos: BTreeMap::new(),
        }
    }

    /// Builder: add a repo.
    #[must_use]
    pub fn with_repo(mut self, repo: Repo) -> Self {
        self.repos.insert(repo.id.clone(), repo);
        self
    }

    /// Builder: replace the user's defaults tree.
    #[must_use]
    pub fn with_defaults(mut self, defaults: AgentsTree) -> Self {
        self.defaults = defaults;
        self
    }

    /// Fold repos + defaults into a single tree. Repo trees are emitted **before** the defaults
    /// so the workspace's own tree takes override precedence (later scopes win in the compiler's
    /// merge policy). Repo iteration order is stable via `BTreeMap<RepoId, _>`.
    #[must_use]
    pub fn materialize(&self) -> AgentsTree {
        let mut children: Vec<AgentsTree> = self.repos.values().map(|r| r.tree.clone()).collect();
        children.push(self.defaults.clone());

        // If every contribution is already a `Global` scope, collapse them into one so callers
        // don't end up with two sibling Global branches at the top level.
        let all_global = children.iter().all(|c| {
            matches!(
                c,
                AgentsTree::Scope {
                    kind: ScopeKind::Global,
                    ..
                }
            )
        });
        if all_global {
            let mut merged_children: Vec<AgentsTree> = Vec::new();
            for c in children {
                if let AgentsTree::Scope {
                    kind: ScopeKind::Global,
                    children,
                } = c
                {
                    merged_children.extend(children);
                }
            }
            return AgentsTree::Scope {
                kind: ScopeKind::Global,
                children: merged_children,
            };
        }

        AgentsTree::Scope {
            kind: ScopeKind::Global,
            children,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::{RuleBody, RuleNode};

    #[test]
    fn workspace_materialize_merges_global_scopes() {
        let repo = Repo::embedded(
            "extra",
            AgentsTree::global([AgentsTree::Rules(vec![RuleNode {
                name: "from-repo.md".into(),
                body: RuleBody::Inline("repo\n".into()),
            }])]),
        );
        let ws = Workspace::new("/tmp/demo", "demo")
            .with_repo(repo)
            .with_defaults(AgentsTree::global([AgentsTree::Rules(vec![RuleNode {
                name: "from-workspace.md".into(),
                body: RuleBody::Inline("workspace\n".into()),
            }])]));

        let tree = ws.materialize();
        let AgentsTree::Scope { kind, children } = tree else {
            panic!("expected Scope")
        };
        assert!(matches!(kind, ScopeKind::Global));
        // We should see both Rules bundles, repo first then workspace last (so workspace wins).
        assert_eq!(children.len(), 2);
    }
}
