//! Stdio **MCP server** (via [`rmcp`]) so agents can create, register, validate, and delete plugin
//! schemas at runtime, and audit installs over schema.org JSON-LD.
//!
//! The server is intentionally small: one handler with tools named after the plugin lifecycle.
//! It is transport-agnostic — use [`run_stdio`] for the conventional stdio transport, or pipe
//! any [`tokio::io::AsyncRead`] + [`tokio::io::AsyncWrite`] pair via [`PluginMcpServer::serve`]
//! to reach the same handler from a [Zenoh duplex](crate::zenoh::ZenohDuplex) or a test pipe.

mod handler;
mod tools;

pub use handler::{run_stdio, PluginMcpServer};
pub use tools::{
    DeleteSchemaRequest, ListSchemasResponse, RegisterSchemaRequest, SchemaOrgAuditResponse,
    ValidateConfigRequest, ValidateConfigResponse,
};
