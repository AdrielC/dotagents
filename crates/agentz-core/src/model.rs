//! Per-agent vocabulary: which agent, how a file is linked, and Cursor naming rules.

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Built-in agent ids. Mirrors the dot-agents platforms and adds upstream clients covered by
/// [iannuttall/dotagents](https://github.com/iannuttall/dotagents).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AgentId {
    Cursor,
    ClaudeCode,
    Codex,
    OpenCode,
    Gemini,
    Factory,
    Github,
    Ampcode,
}

impl AgentId {
    pub fn as_str(self) -> &'static str {
        match self {
            AgentId::Cursor => "cursor",
            AgentId::ClaudeCode => "claude-code",
            AgentId::Codex => "codex",
            AgentId::OpenCode => "opencode",
            AgentId::Gemini => "gemini",
            AgentId::Factory => "factory",
            AgentId::Github => "github",
            AgentId::Ampcode => "ampcode",
        }
    }

    pub fn all() -> &'static [AgentId] {
        &[
            AgentId::Cursor,
            AgentId::ClaudeCode,
            AgentId::Codex,
            AgentId::OpenCode,
            AgentId::Gemini,
            AgentId::Factory,
            AgentId::Github,
            AgentId::Ampcode,
        ]
    }
}

/// How a path is materialized in the project tree.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    /// Same inode as the source (required for Cursor `.cursor/rules` in dot-agents).
    HardLink,
    /// Symbolic link to an absolute or relative target.
    Symlink,
    /// Copy the file at apply time (used for `.md → .mdc` rewrites).
    Copy,
}

/// A planned link: "materialize `source` as `dest` using `kind`, owned by `agent`".
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PlannedLink {
    pub agent: AgentId,
    pub kind: LinkKind,
    pub source: PathBuf,
    pub dest: PathBuf,
}

/// Cursor rule filename layout in `.cursor/rules/`: `global--foo.mdc` or `{project}--foo.mdc`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CursorRuleNaming {
    pub scope: String,
    pub basename: String,
}

impl CursorRuleNaming {
    pub fn dest_filename(&self) -> String {
        format!("{}--{}", self.scope, self.basename)
    }
}

/// If the source is `.md` but not `.mdc`, Cursor receives `.mdc` (dot-agents behavior).
pub fn cursor_display_name(original: &str) -> String {
    if original.ends_with(".md") && !original.ends_with(".mdc") {
        let stem = original.strip_suffix(".md").unwrap_or(original);
        format!("{stem}.mdc")
    } else {
        original.to_string()
    }
}
