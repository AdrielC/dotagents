//! Common vocabulary: which agent, how a file is linked, and naming rules for Cursor.

use serde::{Deserialize, Serialize};

/// Identifiers for built-in link targets (mirrors dot-agents platforms).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentId {
    Cursor,
    ClaudeCode,
    Codex,
    OpenCode,
}

impl AgentId {
    pub fn as_str(self) -> &'static str {
        match self {
            AgentId::Cursor => "cursor",
            AgentId::ClaudeCode => "claude-code",
            AgentId::Codex => "codex",
            AgentId::OpenCode => "opencode",
        }
    }

    /// All built-in agents, in a stable order.
    pub fn all() -> &'static [AgentId] {
        &[
            AgentId::Cursor,
            AgentId::ClaudeCode,
            AgentId::Codex,
            AgentId::OpenCode,
        ]
    }
}

/// How a path is materialized in the project tree.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum LinkKind {
    /// Same inode as the source (required for Cursor `.cursor/rules` in dot-agents).
    HardLink,
    /// Symbolic link to an absolute or relative target.
    Symlink,
}

/// One filesystem operation the installer may perform.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PlannedLink {
    pub agent: AgentId,
    pub kind: LinkKind,
    pub source: std::path::PathBuf,
    pub dest: std::path::PathBuf,
}

/// Cursor rule filename in `.cursor/rules/`: `global--foo.mdc` or `{project}--foo.mdc`.
#[derive(Clone, Debug, Eq, PartialEq)]
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

/// `serde_json::Value` matching the submodule template at `dot-agents/.../config.json`.
pub fn default_config_json() -> serde_json::Value {
    serde_json::json!({
        "$schema": "https://dot-agents.dev/schemas/config.json",
        "version": 1,
        "defaults": { "agent": "cursor" },
        "projects": {},
        "agents": {
            "cursor": { "enabled": true, "version_detected": null },
            "claude-code": { "enabled": true, "version_detected": null },
            "codex": { "enabled": true, "version_detected": null },
            "opencode": { "enabled": false, "version_detected": null }
        },
        "features": {
            "tasks": false,
            "history": false,
            "sync": false
        },
        "history": {
            "format": "jsonl",
            "retention_days": 90
        },
        "notifications": {
            "on_migration": true,
            "on_conflict": true
        }
    })
}

/// New project entry under `projects.<name>`.
pub fn project_record(path: &std::path::Path) -> serde_json::Value {
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    serde_json::json!({
        "path": path.to_string_lossy(),
        "added": now
    })
}
