//! **AgentsTree** — the recursive, pure-data AST for an `~/.agents` layout.
//!
//! The old `AgentsHome` type was a bag of `PathBuf` getters that mixed layout with IO. Here we
//! describe the layout itself. A `compile` pass walks this tree and emits a [`CompiledPlan`]
//! (see [`crate::compile`]) without ever touching disk. A separate IO crate then walks that
//! plan to produce real files.
//!
//! ```text
//! AgentsTree::Scope { name: "global", children: [
//!     AgentsTree::Rules(vec![RuleNode { name: "rules.mdc", body: RuleBody::Inline(".."). }]),
//!     AgentsTree::Skills(vec![SkillNode { name: "test-writer", body: ".." }]),
//!     AgentsTree::Settings(vec![SettingsNode { agent: AgentId::Cursor, path: "cursor.json" }]),
//!     AgentsTree::Mcp(serde_json::json!({ "servers": { .. } })),
//! ]}
//! ```

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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

/// Recursive `~/.agents` AST.
///
/// `Scope` nodes are the only branch in the tree. Everything else is a leaf that carries typed
/// content.  A `Scope` is either `"global"` or a project key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum AgentsTree {
    /// A named scope (`"global"` or a project key) grouping child nodes.
    Scope {
        name: String,
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
    /// Build a `global` scope. Handy entry point for fluent construction.
    pub fn global(children: impl IntoIterator<Item = AgentsTree>) -> Self {
        AgentsTree::Scope {
            name: "global".into(),
            children: children.into_iter().collect(),
        }
    }

    /// Build a named project scope.
    pub fn project(name: impl Into<String>, children: impl IntoIterator<Item = AgentsTree>) -> Self {
        AgentsTree::Scope {
            name: name.into(),
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
        if let AgentsTree::Scope { children, .. } = node {
            // Push right-to-left so we visit left-to-right.
            for child in children.iter().rev() {
                self.stack.push(child);
            }
        }
        Some(node)
    }
}
