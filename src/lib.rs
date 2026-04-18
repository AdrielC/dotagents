//! Unified AI agent configuration: common `~/.agents` format, conversions between
//! on-disk rule names and per-agent layouts, and filesystem installs (hard links
//! and symlinks) matching [dot-agents](https://github.com/dot-agents/dot-agents).
//!
//! The upstream shell implementation lives in the `dot-agents` git submodule for reference.

pub mod config;
pub mod install;
pub mod model;
pub mod plugins;

pub use config::{read_config, write_config, AgentsConfig};
pub use install::{init_agents_home, install_project, InitOptions, InstallOptions, InstallReport};
pub use model::{
    default_config_json, project_record, AgentId, CursorRuleNaming, LinkKind, PlannedLink,
};
pub use plugins::{InstallContext, PluginRegistry, ProjectLinker};
