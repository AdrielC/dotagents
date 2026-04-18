//! **AgentsTree** — the recursive, pure-data AST for an `~/.agents` layout.
//!
//! The old `AgentsHome` type was a bag of `PathBuf` getters that mixed layout with IO. Here we
//! describe the layout itself. A `compile` pass walks this tree and emits a [`CompiledPlan`]
//! (see [`crate::compile`]) without ever touching disk. A separate IO crate then walks that
//! plan to produce real files.
//!
//! ```text
//! AgentsTree::Scope {
//!     kind: ScopeKind::Global,
//!     children: vec![
//!         AgentsTree::Rules(vec![RuleNode { name: "rules.mdc", body: RuleBody::Inline("..") }]),
//!         AgentsTree::Skills(vec![SkillNode { name: "test-writer", body: ".." }]),
//!         AgentsTree::Settings(vec![SettingsNode { agent: AgentId::Cursor, file_name: "cursor.json".into(), .. }]),
//!         AgentsTree::Mcp(serde_json::json!({ "servers": { .. } })),
//!     ],
//! }
//! ```
//!
//! ## Scopes and composition
//!
//! [`ScopeKind`] classifies each branch: **global** (user-wide), **project**, **workstream** (slug),
//! or **profile** (slug + optional `extends` chain). You **nest** scopes by placing
//! [`AgentsTree::Scope`] nodes inside `children` — e.g. global → project → workstream. **Profiles**
//! merge inherited profile definitions (see [`ScopeKind::Profile`]) then apply their own leaves on
//! top (rules/skills/settings/MCP override by key).

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::id::{ProfileId, ProjectKey, WorkstreamId};
use crate::model::AgentId;

/// A rule file. `body` is either inline content or an on-disk source the IO layer will hard-link
/// or symlink when it materializes the plan.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RuleNode {
    /// File name without the leading scope prefix (e.g. `"010-elixir-standards.md"`).
    pub name: String,
    pub body: RuleBody,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum RuleBody {
    /// Full text; IO layer writes the file atomically.
    Inline(String),
    /// Path to an on-disk source that the IO layer will link against.
    Source(PathBuf),
}

/// A skill folder. Body is a `SKILL.md` content or a source directory.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SkillNode {
    pub name: String,
    pub body: SkillBody,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum SkillBody {
    Inline(String),
    Source(PathBuf),
}

/// Per-agent settings file (e.g. `cursor.json`, `claude-code.json`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SettingsNode {
    pub agent: AgentId,
    pub file_name: String,
    #[serde(default)]
    pub body: SettingsBody,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum SettingsBody {
    #[default]
    Empty,
    Inline(String),
    Source(PathBuf),
}

/// What kind of branch this [`AgentsTree::Scope`] is. Drives filename prefixes and inheritance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ScopeKind {
    /// User-wide defaults (Cursor/Claude `global--*` prefixes).
    Global,
    /// A single project/repo bucket (`{project_key}--*` rule prefixes).
    Project { key: ProjectKey },
    /// Per workstream overlay (`ws--{slug}--*`).
    Workstream { slug: WorkstreamId },
    /// Named profile (`profile--{id}--*`). Inheritance is defined on [`AgentsTree::ProfileDef`]
    /// entries in the tree (see registry + [`ProfileRegistry`](crate::compile::ProfileRegistry) in `compile`).
    Profile { id: ProfileId },
}

impl ScopeKind {
    /// Cursor/Claude rule filename prefix segment (before `--{rule-name}`).
    pub fn rule_prefix(&self) -> String {
        match self {
            ScopeKind::Global => "global".into(),
            ScopeKind::Project { key } => key.as_str().to_string(),
            ScopeKind::Workstream { slug } => format!("ws--{}", slug.as_str()),
            ScopeKind::Profile { id, .. } => format!("profile--{}", id.as_str()),
        }
    }
}

/// Recursive `~/.agents` AST.
///
/// `Scope` nodes are the only branch in the tree. Everything else is a leaf that carries typed
/// content. Nest scopes under `children` to compose global → project → workstream (or attach
/// profiles).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum AgentsTree {
    /// A typed scope grouping child nodes.
    Scope {
        kind: ScopeKind,
        children: Vec<AgentsTree>,
    },
    /// Registered profile bundle (`extends` + children). Placed alongside scopes (often under global).
    /// Not emitted directly; merged when a [`ScopeKind::Profile`] scope references `id`.
    ProfileDef {
        id: ProfileId,
        #[serde(default)]
        extends: Vec<ProfileId>,
        children: Vec<AgentsTree>,
    },
    /// A bundle of rule files for this scope.
    Rules(Vec<RuleNode>),
    /// A bundle of skill directories for this scope.
    Skills(Vec<SkillNode>),
    /// Per-agent settings files.
    Settings(Vec<SettingsNode>),
    /// A unified MCP server list (the Cyberdyne-style unified mcp.toml, as JSON).
    Mcp(serde_json::Value),
    /// A free-form text artifact the IO layer will write verbatim (e.g. a `.cursorignore`).
    TextFile { name: String, body: String },
}

impl AgentsTree {
    /// Build a [`ScopeKind::Global`] scope (user-wide).
    pub fn global(children: impl IntoIterator<Item = AgentsTree>) -> Self {
        AgentsTree::Scope {
            kind: ScopeKind::Global,
            children: children.into_iter().collect(),
        }
    }

    /// Build a [`ScopeKind::Project`] scope.
    pub fn project(key: impl Into<ProjectKey>, children: impl IntoIterator<Item = AgentsTree>) -> Self {
        AgentsTree::Scope {
            kind: ScopeKind::Project { key: key.into() },
            children: children.into_iter().collect(),
        }
    }

    /// Build a [`ScopeKind::Workstream`] scope (`ws--{slug}` prefixes).
    pub fn workstream(slug: impl Into<WorkstreamId>, children: impl IntoIterator<Item = AgentsTree>) -> Self {
        AgentsTree::Scope {
            kind: ScopeKind::Workstream {
                slug: slug.into(),
            },
            children: children.into_iter().collect(),
        }
    }

    /// Build a [`ScopeKind::Profile`] scope. Merge `extends` and default content via
    /// [`AgentsTree::profile_def`] nodes registered in the same tree.
    pub fn profile(id: impl Into<ProfileId>, children: impl IntoIterator<Item = AgentsTree>) -> Self {
        AgentsTree::Scope {
            kind: ScopeKind::Profile { id: id.into() },
            children: children.into_iter().collect(),
        }
    }

    /// Register a reusable profile body (for [`ScopeKind::Profile`] and `extends` chains).
    /// Emits nothing by itself; [`crate::compile::compile`] merges this when resolving profiles.
    pub fn profile_def(
        id: impl Into<ProfileId>,
        extends: impl IntoIterator<Item = ProfileId>,
        children: impl IntoIterator<Item = AgentsTree>,
    ) -> Self {
        AgentsTree::ProfileDef {
            id: id.into(),
            extends: extends.into_iter().collect(),
            children: children.into_iter().collect(),
        }
    }

    /// Generic scope constructor.
    pub fn scope(kind: ScopeKind, children: impl IntoIterator<Item = AgentsTree>) -> Self {
        AgentsTree::Scope {
            kind,
            children: children.into_iter().collect(),
        }
    }

    /// Depth-first iterator over every node.
    pub fn walk(&self) -> TreeWalk<'_> {
        TreeWalk { stack: vec![self] }
    }
}

pub struct TreeWalk<'a> {
    stack: Vec<&'a AgentsTree>,
}

impl<'a> Iterator for TreeWalk<'a> {
    type Item = &'a AgentsTree;

    fn next(&mut self) -> Option<Self::Item> {
        let node = self.stack.pop()?;
        match node {
            AgentsTree::Scope { children, .. } | AgentsTree::ProfileDef { children, .. } => {
                for child in children.iter().rev() {
                    self.stack.push(child);
                }
            }
            _ => {}
        }
        Some(node)
    }
}
