//! **Zenoh bus** — in-process Zenoh session + an [`AsyncRead`] + [`AsyncWrite`] **duplex**
//! over two key expressions.
//!
//! The design doc calls for RPC (both MCP and ACP) over Zenoh so each workstream can host a
//! dedicated agent. The primitive for that is:
//!
//! ```text
//!     peer A                                  peer B
//!  ┌──────────────┐                       ┌──────────────┐
//!  │ inbound key ◄┼───── cyberdyne/.../b2a │ publisher ►──┤
//!  │              │                       │              │
//!  │ publisher   ►┼───── cyberdyne/.../a2b │◄ inbound key │
//!  └──────────────┘                       └──────────────┘
//! ```
//!
//! Each peer writes bytes to its outbound publisher and reads bytes delivered to the subscriber
//! attached to its inbound key. Any protocol that works on [`tokio::io::AsyncRead`]/[`tokio::io::AsyncWrite`]
//! (e.g. the rmcp MCP server or an ACP JSON-RPC runner) can ride the duplex unchanged.

mod duplex;
mod session;

pub use duplex::{ZenohDuplex, ZenohDuplexReader, ZenohDuplexWriter};
pub use session::{open_local_peer, open_session_with_config, ZenohBus};
