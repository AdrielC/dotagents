//! **Agent Client Protocol** adapter.
//!
//! This module re-exports the public API of [`agent_client_protocol`] so downstream code does not
//! need to add the crate itself, and provides helpers to run the ACP JSON-RPC framing over a
//! [`crate::zenoh_bus::ZenohDuplex`] (or any other `AsyncRead`/`AsyncWrite` transport). Wire this
//! into a per-workstream agent to get a dedicated ACP channel keyed by
//! `cyberdyne/ws/{id}/acp/{a2b|b2a}`.
//!
//! The heavy lifting lives in `agent-client-protocol`: it ships its own async connection types
//! that accept any read/write transport. We surface them here under stable names so a caller can
//! swap transports without touching the agent or client implementation.

pub use agent_client_protocol::{
    Agent, AgentCapabilities, AgentNotification, AgentRequest, AgentResponse, AgentSideConnection,
    Client, ClientCapabilities, ClientNotification, ClientRequest, ClientResponse,
    ClientSideConnection, ContentBlock, InitializeRequest, InitializeResponse, NewSessionRequest,
    NewSessionResponse, PromptRequest, PromptResponse, ProtocolVersion, SessionId,
    SessionNotification, SessionUpdate, StopReason,
};

/// Compose a per-workstream ACP key pair that pairs cleanly with a [`crate::zenoh_bus::ZenohDuplex`].
///
/// Returns `(inbound_key, outbound_key)` for the given side (`"agent"` or `"client"`). Use the
/// returned pair on one side and the swap on the other.
pub fn workstream_acp_keys(workstream_id: &str, side: AcpSide) -> (String, String) {
    let agent = format!("cyberdyne/ws/{workstream_id}/acp/agent");
    let client = format!("cyberdyne/ws/{workstream_id}/acp/client");
    match side {
        AcpSide::Agent => (client.clone(), agent.clone()),
        AcpSide::Client => (agent, client),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AcpSide {
    Agent,
    Client,
}
