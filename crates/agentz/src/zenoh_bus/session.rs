//! Thin wrapper around [`zenoh::Session`] plus helpers to open in-process peers.

use std::sync::Arc;

use tokio::io::{AsyncRead, AsyncWrite};
use zenoh::Session;

use super::duplex::ZenohDuplex;

/// Open a local Zenoh peer that has no network listeners. Ideal for tests and in-process pub/sub.
pub async fn open_local_peer() -> zenoh::Result<Session> {
    let mut config = zenoh::Config::default();
    config.set_mode(Some(zenoh::config::WhatAmI::Peer)).unwrap();
    let _ = config
        .scouting
        .multicast
        .set_enabled(Some(false));
    let _ = config.listen.endpoints.set(vec![]);
    let _ = config.connect.endpoints.set(vec![]);
    zenoh::open(config).await
}

/// Open a session from a caller-provided [`zenoh::Config`].
pub async fn open_session_with_config(config: zenoh::Config) -> zenoh::Result<Session> {
    zenoh::open(config).await
}

/// Convenience bundle: a Zenoh session and a duplex factory.
#[derive(Clone)]
pub struct ZenohBus {
    pub session: Arc<Session>,
}

impl ZenohBus {
    pub fn new(session: Session) -> Self {
        Self { session: Arc::new(session) }
    }

    /// Build a duplex pair for a workstream — reads from `inbound_key`, writes to `outbound_key`.
    pub async fn duplex(
        &self,
        inbound_key: impl Into<String>,
        outbound_key: impl Into<String>,
    ) -> zenoh::Result<impl AsyncRead + AsyncWrite + Send + Unpin + 'static> {
        ZenohDuplex::open(self.session.clone(), inbound_key.into(), outbound_key.into()).await
    }
}
