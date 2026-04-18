//! **Workstream** descriptor (routing metadata). Pure data; used as a key for per-workstream
//! MCP/ACP servers and for schema.org emission.

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::id::WorkstreamId;

/// Category of a workstream (used in scope rule prefixes and routing metadata).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum WorkstreamKind {
    #[default]
    Feature,
    Spike,
    Bug,
    TechDebt,
}

impl WorkstreamKind {
    /// Short segment in compiled rule filenames: `ws--{segment}--{slug}--*`.
    pub fn as_rule_segment(self) -> &'static str {
        match self {
            WorkstreamKind::Feature => "feature",
            WorkstreamKind::Spike => "spike",
            WorkstreamKind::Bug => "bug",
            WorkstreamKind::TechDebt => "tech-debt",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WorkstreamDescriptor {
    /// Stable id (UUID; mint with [`WorkstreamId::new_v7`] or parse from storage).
    pub id: WorkstreamId,
    /// Human path / short label for filenames and UI (e.g. `feat-auth`). Distinct from `id`.
    pub slug: String,
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
    pub fn new(id: WorkstreamId, slug: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id,
            slug: slug.into(),
            title: title.into(),
            kind: WorkstreamKind::default(),
            objective: String::new(),
            root: None,
        }
    }

    /// Convenience: mint a new v7 id and fill slug/title.
    pub fn new_v7(slug: impl Into<String>, title: impl Into<String>) -> Self {
        Self::new(WorkstreamId::new_v7(), slug, title)
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
