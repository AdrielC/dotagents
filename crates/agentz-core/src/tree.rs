//! **AgentsTree** — the recursive, pure-data AST.
//!
//! A `compile` pass walks this tree and emits a [`CompiledPlan`] (see [`crate::compile`]) without
//! ever touching disk. A separate IO crate then walks that plan to produce real files.
//!
//! ```text
//! AgentsTree::Scope {
//!     kind: ScopeKind::Global,
//!     children: vec![
//!         AgentsTree::Rules(vec![RuleNode { name: "rules.mdc", body: RuleBody::Inline("..") }]),
//!         AgentsTree::Skills(vec![SkillNode { name: "test-writer", body: SkillBody::Inline("..") }]),
//!         AgentsTree::Settings(vec![SettingsNode::claude_local("settings.local.json", "{\"x\":1}")]),
//!         AgentsTree::Hooks(vec![HookBinding::pre_tool_use("Bash", HookHandler::command("check.sh"))]),
//!         AgentsTree::Ignore { agent: AgentId::Cursor, kind: IgnoreKind::Primary, patterns: vec![".env".into()] },
//!         AgentsTree::Mcp(serde_json::json!({ "mcpServers": { .. } })),
//!     ],
//! }
//! ```
//!
//! ## Scopes and composition
//!
//! [`ScopeKind`] classifies each branch: **global** (user-wide), **project**, **workstream** (slug +
//! [`WorkstreamKind`](crate::workstream::WorkstreamKind)), or **profile** (id + optional `extends`
//! via [`AgentsTree::ProfileDef`]). You **nest** scopes by placing [`AgentsTree::Scope`] nodes
//! inside `children` — e.g. global → project → workstream. **Profiles** merge inherited profile
//! definitions (see [`ScopeKind::Profile`]) then apply their own leaves on top (rules/skills/
//! settings/MCP override by key).
//!
//! ## Magic strings are banished
//!
//! Scope prefix literals (`global`, `profile--`, `ws--`) live as [`SCOPE_GLOBAL`], [`SCOPE_PROFILE_PREFIX`],
//! [`SCOPE_WORKSTREAM_PREFIX`], [`SCOPE_SEP`] constants — **not** scattered `format!` calls. Agent layout
//! details (config dir, rules subdir, ignore filename, …) live on [`crate::model::AgentSpec`].

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::id::{ProfileId, ProjectKey, WorkstreamId};
use crate::model::{AgentId, IgnoreKind, SettingsScope};
use crate::workstream::WorkstreamKind;

/// The canonical separator between scope prefix segments and between prefix and rule name.
/// Used in filenames like `global--foo.mdc` and `ws--feature--auth--010-rule.mdc`.
pub const SCOPE_SEP: &str = "--";

/// Scope prefix for user-wide / "global" defaults.
pub const SCOPE_GLOBAL: &str = "global";

/// Scope prefix head for workstream scopes — full segment is `ws--{kind}--{slug}`.
pub const SCOPE_WORKSTREAM_PREFIX: &str = "ws";

/// Scope prefix head for profile scopes — full segment is `profile--{id}`.
pub const SCOPE_PROFILE_PREFIX: &str = "profile";

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

/// A Claude-style **subagent** (`.claude/agents/<name>.md`): markdown with YAML frontmatter
/// (`name`, `description`, `tools`, `model`, …) and a body that's the system prompt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentNode {
    /// Agent id (becomes the filename stem). Kebab-case by convention.
    pub name: String,
    pub body: AgentBody,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum AgentBody {
    /// Full text of the subagent markdown (frontmatter + body).
    Inline(String),
    /// Path to an existing `.md` file on disk that the IO layer will link to.
    Source(PathBuf),
}

/// Per-agent settings file. The compiler resolves the actual filename from
/// [`crate::model::AgentSpec::settings_filename`] given the agent + [`SettingsScope`] so callers
/// don't hardcode names like `settings.local.json`.
///
/// `file_name` is an explicit override: if the agent's spec has no settings file for the selected
/// scope (or you want a non-default name), set this. Otherwise leave it `None`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SettingsNode {
    pub agent: AgentId,
    #[serde(default)]
    pub scope: SettingsScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    #[serde(default)]
    pub body: SettingsBody,
}

impl SettingsNode {
    /// Convenience: a Claude Code `settings.json` (project scope).
    #[must_use]
    pub fn claude_project(body: SettingsBody) -> Self {
        Self {
            agent: AgentId::ClaudeCode,
            scope: SettingsScope::Project,
            file_name: None,
            body,
        }
    }

    /// Convenience: a Claude Code `settings.local.json` (personal / gitignored).
    #[must_use]
    pub fn claude_local(body: SettingsBody) -> Self {
        Self {
            agent: AgentId::ClaudeCode,
            scope: SettingsScope::Local,
            file_name: None,
            body,
        }
    }

    /// Convenience: a Claude Code `managed-settings.json` (org-wide).
    #[must_use]
    pub fn claude_managed(body: SettingsBody) -> Self {
        Self {
            agent: AgentId::ClaudeCode,
            scope: SettingsScope::Managed,
            file_name: None,
            body,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum SettingsBody {
    #[default]
    Empty,
    Inline(String),
    Source(PathBuf),
}

/// A hook binding — one event + matcher + handler triple. Claude Code's hook config is a map of
/// `{event: [{matcher, hooks: [...] }]}`; we flatten it here so a `Vec<HookBinding>` can serialize
/// to either Claude's shape or Cursor's depending on the target agent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HookBinding {
    pub event: HookEvent,
    /// Matcher string (e.g. `"Bash"`, `"Edit|Write"`, `""` to match all). Interpretation depends
    /// on the event (see [`HookEvent`]).
    #[serde(default)]
    pub matcher: String,
    pub handler: HookHandler,
}

impl HookBinding {
    /// Convenience: a pre-tool-use hook running a shell command.
    #[must_use]
    pub fn pre_tool_use(matcher: impl Into<String>, handler: HookHandler) -> Self {
        Self {
            event: HookEvent::PreToolUse,
            matcher: matcher.into(),
            handler,
        }
    }

    /// Convenience: a post-tool-use hook.
    #[must_use]
    pub fn post_tool_use(matcher: impl Into<String>, handler: HookHandler) -> Self {
        Self {
            event: HookEvent::PostToolUse,
            matcher: matcher.into(),
            handler,
        }
    }
}

/// The catalogue of hook events. Covers Claude Code's full event list (session, tool, permission,
/// compaction, etc.) plus Cursor's Agent lifecycle events (which map onto a subset).
///
/// The [`HookEvent::serde`] tag uses Claude's exact casing (PreToolUse, SessionStart, …) — that's
/// the only stable name across our targets. The [`HookEvent::as_str`] helper returns the same.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize, JsonSchema)]
pub enum HookEvent {
    // Session lifecycle
    SessionStart,
    SessionEnd,
    // Tools
    PreToolUse,
    PostToolUse,
    PostToolUseFailure,
    // Permissions
    PermissionRequest,
    PermissionDenied,
    // Turns / stops
    Stop,
    StopFailure,
    SubagentStart,
    SubagentStop,
    // Prompts
    UserPromptSubmit,
    // Context / memory
    PreCompact,
    PostCompact,
    InstructionsLoaded,
    // Filesystem / cwd
    FileChanged,
    CwdChanged,
    WorktreeCreate,
    WorktreeRemove,
    // Notifications + MCP elicitation
    Notification,
    Elicitation,
    ElicitationResult,
    // Cursor-specific events
    BeforeShellExecution,
    AfterFileEdit,
    BeforeTabFileRead,
    AfterTabFileEdit,
}

impl HookEvent {
    /// Exact event name as Anthropic and Cursor document it. Stable; used on the wire.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            HookEvent::SessionStart => "SessionStart",
            HookEvent::SessionEnd => "SessionEnd",
            HookEvent::PreToolUse => "PreToolUse",
            HookEvent::PostToolUse => "PostToolUse",
            HookEvent::PostToolUseFailure => "PostToolUseFailure",
            HookEvent::PermissionRequest => "PermissionRequest",
            HookEvent::PermissionDenied => "PermissionDenied",
            HookEvent::Stop => "Stop",
            HookEvent::StopFailure => "StopFailure",
            HookEvent::SubagentStart => "SubagentStart",
            HookEvent::SubagentStop => "SubagentStop",
            HookEvent::UserPromptSubmit => "UserPromptSubmit",
            HookEvent::PreCompact => "PreCompact",
            HookEvent::PostCompact => "PostCompact",
            HookEvent::InstructionsLoaded => "InstructionsLoaded",
            HookEvent::FileChanged => "FileChanged",
            HookEvent::CwdChanged => "CwdChanged",
            HookEvent::WorktreeCreate => "WorktreeCreate",
            HookEvent::WorktreeRemove => "WorktreeRemove",
            HookEvent::Notification => "Notification",
            HookEvent::Elicitation => "Elicitation",
            HookEvent::ElicitationResult => "ElicitationResult",
            HookEvent::BeforeShellExecution => "beforeShellExecution",
            HookEvent::AfterFileEdit => "afterFileEdit",
            HookEvent::BeforeTabFileRead => "beforeTabFileRead",
            HookEvent::AfterTabFileEdit => "afterTabFileEdit",
        }
    }
}

/// A hook handler: what actually runs when the event fires. Claude Code supports four shapes
/// (command, HTTP, prompt, agent); Cursor currently ships only the command shape. We keep them
/// all here and the compiler refuses to emit incompatible shapes for targets that don't support
/// them (emitting a `plan.warnings` note).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum HookHandler {
    Command {
        command: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        args: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        shell: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_secs: Option<u64>,
    },
    Http {
        url: String,
        #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
        headers: std::collections::BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_secs: Option<u64>,
    },
    Prompt {
        prompt: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_secs: Option<u64>,
    },
    Agent {
        prompt: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_secs: Option<u64>,
    },
}

impl HookHandler {
    /// Convenience constructor for the common "run this script" case.
    #[must_use]
    pub fn command(cmd: impl Into<String>) -> Self {
        HookHandler::Command {
            command: cmd.into(),
            args: Vec::new(),
            shell: None,
            timeout_secs: None,
        }
    }
}

/// What kind of branch this [`AgentsTree::Scope`] is. Drives filename prefixes and inheritance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ScopeKind {
    /// User-wide defaults (Cursor/Claude `global--*` prefixes).
    Global,
    /// A single project/repo bucket (`{project_key}--*` rule prefixes).
    Project { key: ProjectKey },
    /// Per workstream overlay (`ws--{feature|spike|bug|tech-debt}--{slug}--*`).
    ///
    /// **`id`** is the stable UUID (v7 when minted); **`slug`** is the human path segment for
    /// filenames and display (not required to be globally unique).
    Workstream {
        id: WorkstreamId,
        /// Short label for rule paths and UI (e.g. `feat-auth`); distinct from [`WorkstreamId`].
        slug: String,
        /// Spike, Feature, Bug, or TechDebt — encoded in the rule filename prefix.
        #[serde(rename = "ws_kind")]
        ws_kind: WorkstreamKind,
    },
    /// Named profile (`profile--{id}--*`). Inheritance is defined on [`AgentsTree::ProfileDef`]
    /// entries in the tree (see registry + [`ProfileRegistry`](crate::compile::ProfileRegistry)).
    Profile { id: ProfileId },
}

impl ScopeKind {
    /// Cursor/Claude rule filename prefix segment (before `{SCOPE_SEP}{rule-name}`). The actual
    /// separators and prefixes come from [`SCOPE_GLOBAL`], [`SCOPE_SEP`], etc., so callers can't
    /// accidentally drift the format.
    #[must_use]
    pub fn rule_prefix(&self) -> String {
        match self {
            ScopeKind::Global => SCOPE_GLOBAL.into(),
            ScopeKind::Project { key } => key.as_str().to_string(),
            ScopeKind::Workstream { slug, ws_kind, .. } => {
                format!(
                    "{head}{sep}{kind}{sep}{slug}",
                    head = SCOPE_WORKSTREAM_PREFIX,
                    sep = SCOPE_SEP,
                    kind = ws_kind.as_rule_segment(),
                )
            }
            ScopeKind::Profile { id, .. } => {
                format!("{SCOPE_PROFILE_PREFIX}{SCOPE_SEP}{}", id.as_str())
            }
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
    /// Registered profile bundle (`extends` + children). Placed alongside scopes (often under
    /// global). Not emitted directly; merged when a [`ScopeKind::Profile`] scope references `id`.
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
    /// A bundle of **subagent** definitions (Claude's `.claude/agents/<name>.md`). Data-driven
    /// per-agent emission — agents without a subagent layout (Cursor, Codex, …) ignore these.
    Agents(Vec<AgentNode>),
    /// Per-agent settings files (scope-aware: `settings.json` vs. `settings.local.json`).
    Settings(Vec<SettingsNode>),
    /// Hooks bundled as one node. The compiler routes each [`HookBinding`] into the correct
    /// per-agent surface (Claude: inside `settings.json`; Cursor: `hooks.json`).
    Hooks(Vec<HookBinding>),
    /// An ignore file (`.cursorignore`, `.cursorindexignore`, `.claudeignore`). `patterns` are
    /// written one per line; the IO layer may overwrite or append depending on policy.
    Ignore {
        agent: AgentId,
        #[serde(default)]
        kind: IgnoreKind,
        patterns: Vec<String>,
    },
    /// A unified MCP server list (Cyberdyne-style, emitted to every agent that supports MCP).
    Mcp(serde_json::Value),
    /// A free-form text artifact the IO layer will write verbatim.
    TextFile { name: String, body: String },
}

impl AgentsTree {
    /// Build a [`ScopeKind::Global`] scope (user-wide).
    #[must_use]
    pub fn global(children: impl IntoIterator<Item = AgentsTree>) -> Self {
        AgentsTree::Scope {
            kind: ScopeKind::Global,
            children: children.into_iter().collect(),
        }
    }

    /// Build a [`ScopeKind::Project`] scope.
    #[must_use]
    pub fn project(
        key: impl Into<ProjectKey>,
        children: impl IntoIterator<Item = AgentsTree>,
    ) -> Self {
        AgentsTree::Scope {
            kind: ScopeKind::Project { key: key.into() },
            children: children.into_iter().collect(),
        }
    }

    /// Build a [`ScopeKind::Workstream`] scope (`ws--{kind}--{slug}` rule prefixes).
    #[must_use]
    pub fn workstream(
        id: WorkstreamId,
        slug: impl Into<String>,
        ws_kind: WorkstreamKind,
        children: impl IntoIterator<Item = AgentsTree>,
    ) -> Self {
        AgentsTree::Scope {
            kind: ScopeKind::Workstream {
                id,
                slug: slug.into(),
                ws_kind,
            },
            children: children.into_iter().collect(),
        }
    }

    /// Build a [`ScopeKind::Profile`] scope.
    #[must_use]
    pub fn profile(
        id: impl Into<ProfileId>,
        children: impl IntoIterator<Item = AgentsTree>,
    ) -> Self {
        AgentsTree::Scope {
            kind: ScopeKind::Profile { id: id.into() },
            children: children.into_iter().collect(),
        }
    }

    /// Register a reusable profile body (for [`ScopeKind::Profile`] and `extends` chains).
    #[must_use]
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
    #[must_use]
    pub fn scope(kind: ScopeKind, children: impl IntoIterator<Item = AgentsTree>) -> Self {
        AgentsTree::Scope {
            kind,
            children: children.into_iter().collect(),
        }
    }

    /// Depth-first iterator over every node.
    #[must_use]
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
