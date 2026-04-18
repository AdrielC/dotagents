//! **Workstream** descriptor (routing metadata). Pure data; used as a key for per-workstream
//! MCP/ACP servers and for schema.org emission.

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::id::WorkstreamId;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum WorkstreamKind {
    #[default]
    Feature,
    Incident,
    Spike,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WorkstreamDescriptor {
    pub id: WorkstreamId,
    pub title: String,
    #[serde(default)]
    pub kind: WorkstreamKind,
    /// Free-form **objective** string that summarizes the goal of this workstream.
    /// Rendered into the `000-cyberdyne-context` rule and into schema.org `Action.description`.
    #[serde(default)]
    pub objective: String,
    /// Optional on-disk root for the workstream's state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<PathBuf>,
}

impl WorkstreamDescriptor {
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: WorkstreamId::new(id),
            title: title.into(),
            kind: WorkstreamKind::default(),
            objective: String::new(),
            root: None,
        }
    }

    /// Zenoh-friendly key prefix: `cyberdyne/ws/{id}`.
    pub fn key_prefix(&self) -> String {
        format!("cyberdyne/ws/{}", self.id)
    }

    pub fn key(&self, suffix: &str) -> String {
        if suffix.is_empty() {
            self.key_prefix()
        } else {
            format!("{}/{}", self.key_prefix(), suffix)
        }
    }
}
