//! Unified AI agent configuration: common `~/.agents` format, conversions between
//! on-disk rule names and per-agent layouts, and filesystem installs (hard links
//! and symlinks) matching [dot-agents](https://github.com/dot-agents/dot-agents).
//!
//! ## Domain layout
//!
//! - [`domain`] — bounded-context types (`AgentsHome`, `ProjectWorkspace`).
//! - [`install`] — split into **bootstrap**, **plan** (per agent), **apply**, and **orchestrate** pipeline.
//! - [`vocabulary`] — [schema.org](https://schema.org) JSON-LD for install audits (`InstallReport.schema_org_json_ld`).
//!
//! The upstream shell implementation lives in the `dot-agents` git submodule for reference.
//! The [iannuttall/dotagents](https://github.com/iannuttall/dotagents) CLI (Bun) lives in
//! `dotagents-iannuttall/` for mapping and client coverage reference.

pub mod config;
pub mod domain;
pub mod install;
pub mod model;
pub mod plugins;
pub mod schema;
pub mod vocabulary;

pub use config::{read_config, write_config, AgentsConfig};
pub use install::{init_agents_home, install_project, InitOptions, InstallOptions, InstallReport};
pub use model::{
    default_config_json, project_record, AgentId, CursorRuleNaming, LinkKind, PlannedLink,
};
/// When using [`install::install_project`], pass `plugins: Option<&mut PluginRegistry>` so
/// `plugins.sync_from_agents_config` can merge schemas from `config.json` and call [`ProjectLinker::configure`].
pub use plugins::{InstallContext, PluginRegistry, ProjectLinker};
pub use schema::{
    plugins_section_from_config, PluginSchemaEntry, PluginSchemaRegistry, PluginsSection,
    SchemaError,
};
pub use vocabulary::{install_context, json_ld_install_report, SCHEMA_ORG};
pub use domain::{AgentsHome, ProjectWorkspace};
