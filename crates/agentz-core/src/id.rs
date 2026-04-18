//! Stable, string-backed newtypes used as routing and graph keys.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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

id_newtype!(/// Stable identifier for a workstream.
WorkstreamId);

id_newtype!(/// Stable identifier for a [`crate::plan::Step`] in a [`crate::plan::Plan`].
StepId);

id_newtype!(/// Stable identifier for a project key (the bucket under `rules/<project>/`, etc.).
ProjectKey);
