//! Per-agent catalogue, link kinds, and the static [`AgentSpec`] table that replaces the per-arm
//! match policies scattered across the old `compile` module.
//!
//! The **only** thing an [`AgentId`] enum variant contributes on its own is the serde tag and a
//! compact wire value. Every policy — config directory, rules directory, skills layout, Cursor's
//! `.md → .mdc` rewrite, whether this agent even has a rules concept, etc. — lives as data on
//! [`AgentSpec`] entries in the [`SPECS`] slice. Adding a new agent is one row, not three `match`
//! arms in four files.

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Built-in agent ids. The identifier is a stable serde tag; all behaviour lives on [`AgentSpec`].
///
/// Mirrors the dot-agents platforms and adds upstream clients covered by
/// [iannuttall/dotagents](https://github.com/iannuttall/dotagents).
#[derive(
    Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize, JsonSchema,
)]
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
    /// Stable string tag used on the wire.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        self.spec().id
    }

    /// Project-relative dot-directory where this agent's config lives (e.g. `.cursor`, `.claude`).
    #[must_use]
    pub fn config_dir(self) -> &'static str {
        self.spec().config_dir
    }

    /// Every known agent, in enum-declaration order.
    #[must_use]
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

    /// The full [`AgentSpec`] for this agent.
    #[must_use]
    pub fn spec(self) -> &'static AgentSpec {
        // Small linear scan — the table is ~8 entries. A match would also work but this way we
        // assert the lookup is *data-driven*: if SPECS is missing an entry the panic tells us.
        SPECS
            .iter()
            .find(|s| s.agent == self)
            .expect("every AgentId variant must appear in SPECS")
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

/// How an agent lays out its **rules** on disk. `None` on [`AgentSpec::rules`] means "no rules
/// concept" (e.g. some agents only care about settings/hooks).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RulesLayout {
    /// Sub-directory under [`AgentSpec::config_dir`] that holds rule files (e.g. `"rules"`). If
    /// `None`, this agent writes a **single** file instead (see [`Self::single_file`]).
    pub dir: Option<&'static str>,
    /// Single-file rules layout: the one file name the agent reads (e.g. Codex's `"AGENTS.md"`).
    /// Mutually exclusive with [`Self::dir`].
    pub single_file: Option<&'static str>,
    /// How to materialise a rule pulled from an on-disk source. Cursor wants hard-links so its
    /// file watcher fires; Claude is happy with symlinks.
    pub link_kind: LinkKind,
    /// How rule filenames get rewritten on emit. See [`RuleNameRewrite`].
    pub name_rewrite: RuleNameRewrite,
    /// Scope prefix separator. Emitted filenames follow `{scope}{sep}{name}`.
    pub scope_sep: &'static str,
    /// For single-file layouts, a rule name must start with this stem to be selected (e.g. Codex
    /// picks the rule literally named `"agents.*"`). `None` means "no filter".
    pub single_file_rule_stem: Option<&'static str>,
}

/// Filename-rewriting policy for rules. Cursor rewrites `.md → .mdc`; Claude strips trailing
/// `.md`/`.mdc` and always appends `.md`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuleNameRewrite {
    /// Leave the filename alone.
    AsIs,
    /// Cursor's `.md → .mdc` rewrite. Anything already `.mdc` is kept.
    CursorMdc,
    /// Claude's normalize-extension rewrite. Any `.md`/`.mdc` trailing extension is stripped and
    /// `.md` is re-appended.
    ClaudeMd,
}

impl RuleNameRewrite {
    /// Apply the rewrite to a raw rule filename.
    #[must_use]
    pub fn apply(self, original: &str) -> String {
        match self {
            RuleNameRewrite::AsIs => original.to_string(),
            RuleNameRewrite::CursorMdc => {
                if original.ends_with(".mdc") {
                    original.to_string()
                } else if let Some(stem) = original.strip_suffix(".md") {
                    format!("{stem}.mdc")
                } else {
                    original.to_string()
                }
            }
            RuleNameRewrite::ClaudeMd => {
                let stem = original
                    .strip_suffix(".mdc")
                    .or_else(|| original.strip_suffix(".md"))
                    .unwrap_or(original);
                format!("{stem}.md")
            }
        }
    }
}

/// How an agent stores **skills / reusable commands**.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SkillsLayout {
    /// Cursor: one file per skill at `<config_dir>/commands/<name>.md`.
    FlatFile {
        dir: &'static str,
        extension: &'static str,
    },
    /// Claude/Codex: a directory per skill containing `SKILL.md` + optional scripts.
    Directory {
        dir: &'static str,
        manifest_file: &'static str,
    },
    /// This agent has no skills concept.
    None,
}

/// How an agent stores **settings** files. Claude has a real (managed > user > project > local)
/// precedence story; Cursor stores settings in a SQLite blob and has no file form worth modelling
/// here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SettingsLayout {
    /// Filename for the base settings file (e.g. `"settings.json"`, written to the config dir).
    /// `None` means the agent has no JSON settings file we care to emit.
    pub base: Option<&'static str>,
    /// Filename for the user-local override (gitignored, personal). Claude calls this
    /// `settings.local.json`; most agents have none.
    pub local: Option<&'static str>,
    /// Filename for org-wide managed settings, if the agent supports them. Claude does.
    pub managed: Option<&'static str>,
}

/// How an agent consumes **MCP server** definitions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct McpLayout {
    /// Path **relative to the project root** where this agent reads its MCP config
    /// (e.g. `".mcp.json"`, `".cursor/mcp.json"`). `None` disables MCP emission for this agent.
    pub project_file: Option<&'static str>,
}

/// How an agent consumes **hooks**.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HooksLayout {
    /// Hooks live inside the settings file, keyed under a top-level object (Claude's model).
    InSettings { key: &'static str },
    /// Hooks live in a standalone JSON file at `<config_dir>/<filename>` (Cursor's model).
    StandaloneFile { filename: &'static str },
    /// This agent has no hooks concept.
    None,
}

/// How an agent consumes **ignore** files (e.g. `.cursorignore`, `.claudeignore`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IgnoreLayout {
    /// Primary ignore file the agent reads (`None` = agent has no ignore file).
    pub primary: Option<&'static str>,
    /// Secondary ignore file — Cursor has `.cursorindexignore` alongside `.cursorignore`.
    pub secondary: Option<&'static str>,
}

/// How an agent stores **subagents** — distinct from skills; Claude-specific today.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentsLayout {
    /// Flat `<config_dir>/<dir>/<name>.<extension>` (Claude's `.claude/agents/<name>.md`).
    FlatFile {
        dir: &'static str,
        extension: &'static str,
    },
    /// This agent has no subagent concept.
    None,
}

/// Complete per-agent policy. One row per [`AgentId`] in [`SPECS`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AgentSpec {
    pub agent: AgentId,
    /// Serde/tag name (kebab-case): `"cursor"`, `"claude-code"`, etc.
    pub id: &'static str,
    /// Project-relative dot-directory, WITH the leading dot (e.g. `".cursor"`).
    pub config_dir: &'static str,
    pub rules: Option<RulesLayout>,
    pub skills: SkillsLayout,
    pub settings: SettingsLayout,
    pub mcp: McpLayout,
    pub hooks: HooksLayout,
    pub ignore: IgnoreLayout,
    pub agents: AgentsLayout,
}

/// Kinds of ignore file. Used both for serde on [`crate::tree::AgentsTree::Ignore`] and to pick
/// the right filename off the [`IgnoreLayout`].
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Eq,
    PartialEq,
    Hash,
    Ord,
    PartialOrd,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum IgnoreKind {
    /// Primary ignore file (`.cursorignore`, `.claudeignore`).
    #[default]
    Primary,
    /// Secondary ignore file (`.cursorindexignore`).
    Secondary,
}

impl AgentSpec {
    /// Filename for a settings scope on this agent, if the agent supports that scope.
    #[must_use]
    pub fn settings_filename(&self, scope: SettingsScope) -> Option<&'static str> {
        match scope {
            SettingsScope::Managed => self.settings.managed,
            SettingsScope::User | SettingsScope::Project => self.settings.base,
            SettingsScope::Local => self.settings.local,
        }
    }

    /// Filename for an ignore kind on this agent, if supported.
    #[must_use]
    pub fn ignore_filename(&self, kind: IgnoreKind) -> Option<&'static str> {
        match kind {
            IgnoreKind::Primary => self.ignore.primary,
            IgnoreKind::Secondary => self.ignore.secondary,
        }
    }
}

/// Where a settings file sits in the precedence chain. Follows Claude Code's published order:
/// managed (org policy) > user (shared across projects) > project (checked in) > local
/// (per-project, personal).
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Eq,
    PartialEq,
    Hash,
    Ord,
    PartialOrd,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum SettingsScope {
    /// Enterprise-deployed, read-only for the user.
    Managed,
    /// User-home scope (`~/.claude/settings.json`).
    User,
    /// Project scope, checked into the repo. This is the default.
    #[default]
    Project,
    /// Project scope, personal + gitignored (Claude's `settings.local.json`).
    Local,
}

/// Static table of every built-in agent's policy. **Edit this table, not scattered `match` arms,
/// when adding or tweaking an agent.**
pub static SPECS: &[AgentSpec] = &[
    AgentSpec {
        agent: AgentId::Cursor,
        id: "cursor",
        config_dir: ".cursor",
        rules: Some(RulesLayout {
            dir: Some("rules"),
            single_file: None,
            link_kind: LinkKind::HardLink,
            name_rewrite: RuleNameRewrite::CursorMdc,
            scope_sep: "--",
            single_file_rule_stem: None,
        }),
        skills: SkillsLayout::FlatFile {
            dir: "commands",
            extension: "md",
        },
        settings: SettingsLayout {
            // Cursor stores settings in a SQLite blob we don't manage; leave all scopes off.
            base: None,
            local: None,
            managed: None,
        },
        mcp: McpLayout {
            project_file: Some(".cursor/mcp.json"),
        },
        hooks: HooksLayout::StandaloneFile {
            filename: "hooks.json",
        },
        ignore: IgnoreLayout {
            primary: Some(".cursorignore"),
            secondary: Some(".cursorindexignore"),
        },
        agents: AgentsLayout::None,
    },
    AgentSpec {
        agent: AgentId::ClaudeCode,
        id: "claude-code",
        config_dir: ".claude",
        rules: Some(RulesLayout {
            dir: Some("rules"),
            single_file: None,
            link_kind: LinkKind::Symlink,
            name_rewrite: RuleNameRewrite::ClaudeMd,
            scope_sep: "--",
            single_file_rule_stem: None,
        }),
        skills: SkillsLayout::Directory {
            dir: "skills",
            manifest_file: "SKILL.md",
        },
        settings: SettingsLayout {
            base: Some("settings.json"),
            local: Some("settings.local.json"),
            managed: Some("managed-settings.json"),
        },
        mcp: McpLayout {
            // Claude reads .mcp.json at the repo root.
            project_file: Some(".mcp.json"),
        },
        hooks: HooksLayout::InSettings { key: "hooks" },
        ignore: IgnoreLayout {
            primary: Some(".claudeignore"),
            secondary: None,
        },
        agents: AgentsLayout::FlatFile {
            dir: "agents",
            extension: "md",
        },
    },
    AgentSpec {
        agent: AgentId::Codex,
        id: "codex",
        config_dir: ".codex",
        rules: Some(RulesLayout {
            dir: None,
            single_file: Some("AGENTS.md"),
            link_kind: LinkKind::Symlink,
            name_rewrite: RuleNameRewrite::AsIs,
            scope_sep: "--",
            single_file_rule_stem: Some("agents."),
        }),
        skills: SkillsLayout::Directory {
            dir: "skills",
            manifest_file: "SKILL.md",
        },
        settings: SettingsLayout {
            base: Some("config.json"),
            local: None,
            managed: None,
        },
        mcp: McpLayout { project_file: None },
        hooks: HooksLayout::None,
        ignore: IgnoreLayout {
            primary: None,
            secondary: None,
        },
        agents: AgentsLayout::None,
    },
    AgentSpec {
        agent: AgentId::OpenCode,
        id: "opencode",
        config_dir: ".opencode",
        rules: None,
        skills: SkillsLayout::None,
        settings: SettingsLayout {
            base: Some("config.json"),
            local: None,
            managed: None,
        },
        mcp: McpLayout { project_file: None },
        hooks: HooksLayout::None,
        ignore: IgnoreLayout {
            primary: None,
            secondary: None,
        },
        agents: AgentsLayout::None,
    },
    AgentSpec {
        agent: AgentId::Gemini,
        id: "gemini",
        config_dir: ".gemini",
        rules: None,
        skills: SkillsLayout::None,
        settings: SettingsLayout {
            base: Some("config.json"),
            local: None,
            managed: None,
        },
        mcp: McpLayout { project_file: None },
        hooks: HooksLayout::None,
        ignore: IgnoreLayout {
            primary: None,
            secondary: None,
        },
        agents: AgentsLayout::None,
    },
    AgentSpec {
        agent: AgentId::Factory,
        id: "factory",
        config_dir: ".factory",
        rules: None,
        skills: SkillsLayout::None,
        settings: SettingsLayout {
            base: Some("config.json"),
            local: None,
            managed: None,
        },
        mcp: McpLayout { project_file: None },
        hooks: HooksLayout::None,
        ignore: IgnoreLayout {
            primary: None,
            secondary: None,
        },
        agents: AgentsLayout::None,
    },
    AgentSpec {
        agent: AgentId::Github,
        id: "github",
        config_dir: ".github",
        rules: None,
        skills: SkillsLayout::None,
        settings: SettingsLayout {
            base: Some("copilot-instructions.md"),
            local: None,
            managed: None,
        },
        mcp: McpLayout { project_file: None },
        hooks: HooksLayout::None,
        ignore: IgnoreLayout {
            primary: None,
            secondary: None,
        },
        agents: AgentsLayout::None,
    },
    AgentSpec {
        agent: AgentId::Ampcode,
        id: "ampcode",
        config_dir: ".ampcode",
        rules: None,
        skills: SkillsLayout::None,
        settings: SettingsLayout {
            base: Some("config.json"),
            local: None,
            managed: None,
        },
        mcp: McpLayout { project_file: None },
        hooks: HooksLayout::None,
        ignore: IgnoreLayout {
            primary: None,
            secondary: None,
        },
        agents: AgentsLayout::None,
    },
];

/// Cursor rule filename layout in `.cursor/rules/`: `global--foo.mdc` or `{project}--foo.mdc`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CursorRuleNaming {
    pub scope: String,
    pub basename: String,
}

impl CursorRuleNaming {
    #[must_use]
    pub fn dest_filename(&self) -> String {
        format!("{}--{}", self.scope, self.basename)
    }
}

/// If the source is `.md` but not `.mdc`, Cursor receives `.mdc` (dot-agents behavior).
///
/// Kept for backwards-compat; prefer [`RuleNameRewrite::CursorMdc::apply`] via
/// [`AgentSpec::rules`].
#[must_use]
pub fn cursor_display_name(original: &str) -> String {
    RuleNameRewrite::CursorMdc.apply(original)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_agent_variant_has_a_spec() {
        for a in AgentId::all() {
            let s = a.spec();
            assert_eq!(s.agent, *a);
            assert!(s.config_dir.starts_with('.'), "{} config dir", s.id);
            assert!(!s.id.is_empty());
        }
    }

    #[test]
    fn cursor_mdc_rewrite_cases() {
        let r = RuleNameRewrite::CursorMdc;
        assert_eq!(r.apply("010-style.md"), "010-style.mdc");
        assert_eq!(r.apply("020-strict.mdc"), "020-strict.mdc");
        assert_eq!(r.apply("README"), "README");
        assert_eq!(r.apply("notes.markdown"), "notes.markdown");
    }

    #[test]
    fn claude_md_rewrite_strips_and_reappends() {
        let r = RuleNameRewrite::ClaudeMd;
        assert_eq!(r.apply("010-style.md"), "010-style.md");
        assert_eq!(r.apply("020-strict.mdc"), "020-strict.md");
        assert_eq!(r.apply("plain"), "plain.md");
    }

    #[test]
    fn settings_scope_filenames_for_claude_and_cursor() {
        let claude = AgentId::ClaudeCode.spec();
        assert_eq!(
            claude.settings_filename(SettingsScope::Project),
            Some("settings.json")
        );
        assert_eq!(
            claude.settings_filename(SettingsScope::Local),
            Some("settings.local.json")
        );
        assert_eq!(
            claude.settings_filename(SettingsScope::Managed),
            Some("managed-settings.json")
        );
        let cursor = AgentId::Cursor.spec();
        assert_eq!(cursor.settings_filename(SettingsScope::Project), None);
    }
}
