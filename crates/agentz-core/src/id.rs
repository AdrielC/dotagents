//! Stable identifiers: mostly string-backed newtypes; [`WorkstreamId`] wraps a UUID (v7 at creation time).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! id_newtype {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize, JsonSchema)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            pub fn new(s: impl Into<String>) -> Self { Self(s.into()) }
            pub fn as_str(&self) -> &str { &self.0 }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<&str> for $name { fn from(s: &str) -> Self { Self(s.to_string()) } }
        impl From<String> for $name { fn from(s: String) -> Self { Self(s) } }
    };
}

/// Stable **machine id** for a workstream (UUID; use [`uuid::Uuid::now_v7`] when minting).
///
/// Separate from the human **slug** on [`crate::tree::ScopeKind::Workstream`] used in rule
/// filename prefixes and display — the slug is not guaranteed unique.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
#[schemars(transparent)]
pub struct WorkstreamId(pub Uuid);

impl WorkstreamId {
    #[must_use]
    pub fn new(uuid: Uuid) -> Self {
        Self(uuid)
    }

    /// Mint a new time-ordered id (UUID v7).
    #[must_use]
    pub fn new_v7() -> Self {
        Self(Uuid::now_v7())
    }

    #[must_use]
    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl std::fmt::Display for WorkstreamId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0.as_hyphenated(), f)
    }
}

impl From<Uuid> for WorkstreamId {
    fn from(value: Uuid) -> Self {
        Self(value)
    }
}

id_newtype!(/// Stable identifier for a [`crate::plan::Step`] in a [`crate::plan::Plan`].
StepId);

id_newtype!(/// Stable identifier for a project key (the bucket under `rules/<project>/`, etc.).
ProjectKey);

id_newtype!(/// Stable identifier for a reusable **profile** bundle (rules/skills/settings merged with [`crate::tree::ScopeKind::Profile`] inheritance).
ProfileId);
