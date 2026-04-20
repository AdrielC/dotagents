//! **`agentz.toml` — the declarative config format users actually author.**
//!
//! Everything in [`crate::tree::AgentsTree`] can be built from Rust, but that's only useful to
//! people who write Rust. The shape below serialises one-to-one to a TOML file that lives at a
//! repo root and describes the whole `Workspace`: which repos to pull in, which targets to
//! compile for, which template variables to plumb through.
//!
//! The file is pure data, validated by serde + the schema this module derives. A future CLI
//! (`agentz validate`) can emit a JSON Schema for IDE auto-complete via [`crate::schema`].
//!
//! ```toml
//! # agentz.toml — a minimal example
//! [workspace]
//! project_key = "my-project"
//! target_agents = ["cursor", "claude-code", "codex"]
//!
//! [vars]
//! team = "infra"
//!
//! [[rules]]
//! name = "010-style.md"
//! body = "# Style for {{ vars.team }}\n- small functions\n"
//!
//! [[skills]]
//! name = "review"
//! body = "---\nname: review\n---\nWalk the diff.\n"
//!
//! [[hooks]]
//! event = "PreToolUse"
//! matcher = "Bash"
//! [hooks.handler]
//! type = "command"
//! command = "scripts/audit.sh"
//! ```

use std::collections::BTreeMap;
use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::id::ProjectKey;
use crate::model::{AgentId, IgnoreKind, SettingsScope};
use crate::render::RenderContext;
use crate::tree::{
    AgentBody, AgentNode, AgentsTree, HookBinding, HookEvent, HookHandler, RuleBody, RuleNode,
    ScopeKind, SettingsBody, SettingsNode, SkillBody, SkillNode,
};

/// Top-level `agentz.toml` document.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct AgentzConfig {
    #[serde(default)]
    pub workspace: WorkspaceConfig,
    /// User-supplied template variables; surfaced to minijinja under `{{ vars.* }}`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub vars: BTreeMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<RuleConfig>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<SkillConfig>,
    #[serde(default, rename = "agent", skip_serializing_if = "Vec::is_empty")]
    pub agents: Vec<AgentConfig>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub settings: Vec<SettingsConfig>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hooks: Vec<HookConfig>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ignore: Vec<IgnoreConfig>,
    /// Unified MCP server block (`.mcp.json` shape).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceConfig {
    #[serde(default)]
    pub project_key: String,
    /// When empty, every built-in dialect is enabled.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub target_agents: Vec<AgentId>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct RuleConfig {
    pub name: String,
    /// Inline rule content. One of `body` or `source` must be set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// On-disk source the IO layer will link against.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct SkillConfig {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct AgentConfig {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct SettingsConfig {
    pub agent: AgentId,
    #[serde(default)]
    pub scope: SettingsScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct HookConfig {
    pub event: HookEvent,
    #[serde(default)]
    pub matcher: String,
    pub handler: HookHandler,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct IgnoreConfig {
    pub agent: AgentId,
    #[serde(default)]
    pub kind: IgnoreKind,
    pub patterns: Vec<String>,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("toml parse: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("rule `{0}` needs either `body` or `source`")]
    RuleBodyMissing(String),
    #[error("skill `{0}` needs either `body` or `source`")]
    SkillBodyMissing(String),
    #[error("agent `{0}` needs either `body` or `source`")]
    AgentBodyMissing(String),
}

impl AgentzConfig {
    /// Parse from TOML text.
    pub fn from_toml(text: &str) -> Result<Self, ConfigError> {
        Ok(toml::from_str(text)?)
    }

    /// Serialise back to a canonical TOML string.
    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string_pretty(self)
    }

    /// Translate the config into a single [`AgentsTree`] (no scopes; everything is global).
    /// Pair with [`crate::render::render_tree`] + [`crate::compile::compile`] to actually emit.
    pub fn to_tree(&self) -> Result<AgentsTree, ConfigError> {
        let mut children: Vec<AgentsTree> = Vec::new();

        if !self.rules.is_empty() {
            let mut rules = Vec::with_capacity(self.rules.len());
            for r in &self.rules {
                rules.push(RuleNode {
                    name: r.name.clone(),
                    body: match (&r.body, &r.source) {
                        (Some(b), _) => RuleBody::Inline(b.clone()),
                        (None, Some(s)) => RuleBody::Source(s.clone()),
                        (None, None) => {
                            return Err(ConfigError::RuleBodyMissing(r.name.clone()));
                        }
                    },
                });
            }
            children.push(AgentsTree::Rules(rules));
        }

        if !self.skills.is_empty() {
            let mut skills = Vec::with_capacity(self.skills.len());
            for s in &self.skills {
                skills.push(SkillNode {
                    name: s.name.clone(),
                    body: match (&s.body, &s.source) {
                        (Some(b), _) => SkillBody::Inline(b.clone()),
                        (None, Some(src)) => SkillBody::Source(src.clone()),
                        (None, None) => {
                            return Err(ConfigError::SkillBodyMissing(s.name.clone()));
                        }
                    },
                });
            }
            children.push(AgentsTree::Skills(skills));
        }

        if !self.agents.is_empty() {
            let mut agents = Vec::with_capacity(self.agents.len());
            for a in &self.agents {
                agents.push(AgentNode {
                    name: a.name.clone(),
                    body: match (&a.body, &a.source) {
                        (Some(b), _) => AgentBody::Inline(b.clone()),
                        (None, Some(s)) => AgentBody::Source(s.clone()),
                        (None, None) => {
                            return Err(ConfigError::AgentBodyMissing(a.name.clone()));
                        }
                    },
                });
            }
            children.push(AgentsTree::Agents(agents));
        }

        if !self.settings.is_empty() {
            let mut settings = Vec::with_capacity(self.settings.len());
            for s in &self.settings {
                settings.push(SettingsNode {
                    agent: s.agent,
                    scope: s.scope,
                    file_name: s.file_name.clone(),
                    body: match (&s.body, &s.source) {
                        (Some(b), _) => SettingsBody::Inline(b.clone()),
                        (None, Some(src)) => SettingsBody::Source(src.clone()),
                        (None, None) => SettingsBody::Empty,
                    },
                });
            }
            children.push(AgentsTree::Settings(settings));
        }

        if !self.hooks.is_empty() {
            let hooks = self
                .hooks
                .iter()
                .map(|h| HookBinding {
                    event: h.event,
                    matcher: h.matcher.clone(),
                    handler: h.handler.clone(),
                })
                .collect();
            children.push(AgentsTree::Hooks(hooks));
        }

        for ig in &self.ignore {
            children.push(AgentsTree::Ignore {
                agent: ig.agent,
                kind: ig.kind,
                patterns: ig.patterns.clone(),
            });
        }

        if let Some(mcp) = &self.mcp {
            children.push(AgentsTree::Mcp(mcp.clone()));
        }

        Ok(AgentsTree::Scope {
            kind: ScopeKind::Global,
            children,
        })
    }

    /// Project key to pair with this tree. Empty string if not set.
    #[must_use]
    pub fn project_key(&self) -> ProjectKey {
        ProjectKey::from(self.workspace.project_key.clone())
    }

    /// Build a [`RenderContext`] from this config's `vars` bag plus caller-supplied extras.
    #[must_use]
    pub fn render_context(&self, project_path: &str) -> RenderContext {
        RenderContext {
            vars: self.vars.clone(),
            project: crate::render::ProjectVars {
                key: self.workspace.project_key.clone(),
                path: project_path.to_string(),
            },
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_config_and_produces_tree() {
        // Use `r##""##` because the TOML content contains `"#` which would close `r#""#`.
        let toml = r##"
[workspace]
project_key = "demo"

[[rules]]
name = "010-style.md"
body = "# Style\n"

[[skills]]
name = "review"
body = "---\nname: review\n---\nbody\n"

[[hooks]]
event = "PreToolUse"
matcher = "Bash"
handler = { type = "command", command = "audit.sh" }

[[ignore]]
agent = "cursor"
patterns = ["node_modules/"]

[mcp]
mcpServers = { gh = { command = "mcp-gh" } }
"##;
        let cfg: AgentzConfig = AgentzConfig::from_toml(toml).unwrap();
        assert_eq!(cfg.workspace.project_key, "demo");
        let tree = cfg.to_tree().unwrap();
        let AgentsTree::Scope { children, .. } = &tree else {
            panic!()
        };
        assert!(children.iter().any(|c| matches!(c, AgentsTree::Rules(_))));
        assert!(children.iter().any(|c| matches!(c, AgentsTree::Skills(_))));
        assert!(children.iter().any(|c| matches!(c, AgentsTree::Hooks(_))));
        assert!(children
            .iter()
            .any(|c| matches!(c, AgentsTree::Ignore { .. })));
        assert!(children.iter().any(|c| matches!(c, AgentsTree::Mcp(_))));
    }

    #[test]
    fn rule_without_body_or_source_errors() {
        let cfg = AgentzConfig {
            rules: vec![RuleConfig {
                name: "incomplete.md".into(),
                body: None,
                source: None,
            }],
            ..Default::default()
        };
        assert!(matches!(
            cfg.to_tree().unwrap_err(),
            ConfigError::RuleBodyMissing(_)
        ));
    }
}
