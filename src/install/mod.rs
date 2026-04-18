//! Installation **bounded context**: bootstrap home, plan links per agent, apply filesystem operations.
//!
//! Layout:
//! - [`bootstrap`] — first-run `~/.agents` tree
//! - [`plan`] — discover sources and build [`crate::model::PlannedLink`] lists (by agent)
//! - [`apply`] — hard links and symlinks
//! - [`orchestrate`] — end-to-end pipeline and [schema.org](https://schema.org) JSON-LD on [`InstallReport`]
//! - [`policy`] — which agents run from config
//! - [`workspace`] — per-project dirs under the home

mod apply;
mod bootstrap;
mod error;
mod orchestrate;
pub mod plan;
mod policy;
mod types;
mod workspace;

pub use bootstrap::init_agents_home;
pub use error::InstallError;
pub use orchestrate::install_project;
pub use types::{InitOptions, InstallOptions, InstallReport};
