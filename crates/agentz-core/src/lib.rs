//! `agentz-core`
//!
//! Pure, IO-free domain model for [agentz](https://github.com/dot-agents). This crate contains
//! only typed data, transforms between them, parsers, and schema generation. It **does not**
//! touch the filesystem, spawn processes, or open sockets. That is the job of the `agentz` crate.
//!
//! ## Layers
//!
//! | Module | What |
//! |--------|------|
//! | [`id`] | Stable identifiers ([`WorkstreamId`](id::WorkstreamId) = UUID v7-capable, [`StepId`](id::StepId), etc.). |
//! | [`model`] | [`AgentId`](model::AgentId) + the single [`AgentSpec`](model::AgentSpec) table that replaces per-agent `match` arms. |
//! | [`workstream`] | [`WorkstreamDescriptor`](workstream::WorkstreamDescriptor) + Zenoh key prefix constants. |
//! | [`tree`] | Recursive **AgentsTree** AST + [`ScopeKind`](tree::ScopeKind), hooks, ignores, settings scope. |
//! | [`plan`] | Pure [`Plan`](plan::Plan) DAG of steps. |
//! | [`compile`] | Fold `(AgentsTree, Context)` → [`CompiledPlan`](compile::CompiledPlan) of [`FsOp`](compile::FsOp)s, all data-driven from [`SPECS`](model::SPECS). |
//! | [`schema`] | `schemars`-backed plugin schema types and registry. |
//! | [`vocabulary`] | schema.org JSON-LD builders + typed [`SchemaType`](vocabulary::SchemaType) / [`ActionStatus`](vocabulary::ActionStatus) constants. |
//! | [`plugins`] | [`ProjectLinker`](plugins::ProjectLinker) extension trait (pure; produces `FsOp`). |
//! | [`repo`] | [`Repo`](repo::Repo), [`RepoSource`](repo::RepoSource), [`Workspace`](repo::Workspace) — named catalogues of [`AgentsTree`] content. |
//! | [`parser`] | Frontmatter-aware parsers for Cursor `.mdc` and Claude `SKILL.md`. |
//!
//! Everything is `Send + Sync` and serde-round-trippable. Wire this crate into an IO crate to
//! actually run installs, talk MCP, or route Zenoh traffic.

pub mod compile;
pub mod id;
pub mod model;
pub mod parser;
pub mod plan;
pub mod plugins;
pub mod repo;
pub mod schema;
pub mod tree;
pub mod vocabulary;
pub mod workstream;

pub use compile::{CompileContext, CompileError, CompiledPlan, FsOp, ProfileRegistry};
pub use id::{ProfileId, ProjectKey, StepId, WorkstreamId};
pub use model::{
    cursor_display_name, AgentId, AgentSpec, AgentsLayout, CursorRuleNaming, HooksLayout,
    IgnoreKind, IgnoreLayout, LinkKind, McpLayout, PlannedLink, RuleNameRewrite, RulesLayout,
    SettingsLayout, SettingsScope, SkillsLayout, SPECS,
};
pub use plan::{Dag, DagError, Objective, Plan, Step, StepKind, StepStatus};
pub use plugins::{InstallContext, ProjectLinker};
pub use repo::{Repo, RepoId, RepoSource, Workspace};
pub use schema::{PluginSchemaEntry, PluginSchemaRegistry, PluginsSection, SchemaError};
pub use tree::{
    AgentBody, AgentNode, AgentsTree, HookBinding, HookEvent, HookHandler, RuleBody, RuleNode,
    ScopeKind, SettingsBody, SettingsNode, SkillBody, SkillNode, SCOPE_GLOBAL,
    SCOPE_PROFILE_PREFIX, SCOPE_SEP, SCOPE_WORKSTREAM_PREFIX,
};
pub use vocabulary::{
    install_context, json_ld_install_report, ActionStatus, SchemaType, AGENTZ_APP_ID,
    AGENTZ_NAMESPACE, SCHEMA_ORG,
};
pub use workstream::{WorkstreamDescriptor, WorkstreamKind, ZENOH_KEY_NAMESPACE, ZENOH_WS_SEGMENT};
