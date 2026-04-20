//! `agentz` — IO-bearing runtime on top of [`agentz_core`].
//!
//! `agentz-core` is pure data: typed `AgentsTree` AST, `Plan` DAG, `CompiledPlan` of `FsOp`,
//! schema.org vocabulary, and `schemars`-backed plugin schemas. This crate wires those into
//! actual filesystem operations and optional transports:
//!
//! | Feature | Module | What it does |
//! |---------|--------|--------------|
//! | always  | [`apply`]     | Execute a [`agentz_core::CompiledPlan`] by walking `FsOp` values. |
//! | always  | [`config`]    | Read/write `config.json` with default merge. |
//! | `mcp`   | [`mcp`]       | stdio MCP server via `rmcp`; tools for plugin-schema lifecycle. |
//! | `zenoh-bus` | [`zenoh_bus`] | In-process Zenoh session + `AsyncRead`/`AsyncWrite` duplex over two keys. |
//! | `acp`       | [`acp`]   | `agent-client-protocol` stubs that can ride the Zenoh duplex. |
//!
//! Prefer writing new domain code in `agentz-core` (pure) and surfacing it here only when it
//! needs real IO.

pub mod apply;
pub mod config;
pub mod env;
pub mod ingest;

#[cfg(feature = "mcp")]
pub mod mcp;

#[cfg(feature = "zenoh-bus")]
pub mod zenoh_bus;

#[cfg(feature = "acp")]
pub mod acp;

pub use agentz_core::*;
pub use apply::{apply_plan, ApplyError, ApplyOptions, ApplyReport};
pub use config::{read_config, write_config, AgentsConfig, ProjectEntry};
