//! `agentz-core`
//!
//! Pure, IO-free domain model for [agentz](https://github.com/dot-agents). This crate contains
//! only typed data, transforms between them, and schema generation. It **does not** touch the
//! filesystem, spawn processes, or open sockets. That is the job of the `agentz` crate.
//!
//! ## Layers
//!
//! | Module | What |
//! |--------|------|
//! | [`id`] | Stable identifiers ([`WorkstreamId`](id::WorkstreamId), [`StepId`](id::StepId), etc.). |
//! | [`model`] | Per-agent types: [`AgentId`](model::AgentId), [`LinkKind`](model::LinkKind), [`PlannedLink`](model::PlannedLink), [`CursorRuleNaming`](model::CursorRuleNaming). |
//! | [`workstream`] | [`WorkstreamDescriptor`](workstream::WorkstreamDescriptor) + kind taxonomy. |
//! | [`tree`] | Recursive **AgentsTree** AST (the pure shape of `~/.agents`). |
//! | [`plan`] | Pure [`Plan`](plan::Plan) DAG of steps (objective + [`Step`](plan::Step) + edges). |
//! | [`compile`] | Fold `(AgentsTree, Context)` → [`CompiledPlan`](compile::CompiledPlan) of [`FsOp`](compile::FsOp)s. |
//! | [`schema`] | `schemars`-backed plugin schema types and registry. |
//! | [`vocabulary`] | schema.org JSON-LD builders. |
//! | [`plugins`] | [`ProjectLinker`](plugins::ProjectLinker) extension trait (pure; produces `FsOp`). |
//!
//! Everything is `Send + Sync` and serde-round-trippable. Wire this crate into an IO crate to
//! actually run installs, talk MCP, or route Zenoh traffic.

pub mod compile;
pub mod id;
pub mod model;
pub mod plan;
pub mod plugins;
pub mod schema;
pub mod tree;
pub mod vocabulary;
pub mod workstream;

pub use compile::{CompileContext, CompileError, CompiledPlan, FsOp};
pub use id::{StepId, WorkstreamId};
pub use model::{cursor_display_name, AgentId, CursorRuleNaming, LinkKind, PlannedLink};
pub use plan::{Dag, DagError, Objective, Plan, Step, StepKind, StepStatus};
pub use plugins::{InstallContext, ProjectLinker};
pub use schema::{
    PluginSchemaEntry, PluginSchemaRegistry, PluginsSection, SchemaError,
};
pub use tree::{AgentsTree, RuleNode, SettingsNode, SkillNode};
pub use vocabulary::{install_context, json_ld_install_report, SCHEMA_ORG};
pub use workstream::{WorkstreamDescriptor, WorkstreamKind};
