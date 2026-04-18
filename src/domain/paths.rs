//! Newtypes for filesystem roots so install code does not thread raw `PathBuf` without meaning.

use std::path::{Path, PathBuf};

/// Canonical configuration store (typically `~/.agents`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentsHome(pub PathBuf);

impl AgentsHome {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self(root.into())
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }

    pub fn rules_global(&self) -> PathBuf {
        self.0.join("rules/global")
    }

    pub fn rules_project(&self, project_key: &str) -> PathBuf {
        self.0.join("rules").join(project_key)
    }

    pub fn settings_global(&self) -> PathBuf {
        self.0.join("settings/global")
    }

    pub fn settings_project(&self, project_key: &str) -> PathBuf {
        self.0.join("settings").join(project_key)
    }

    pub fn mcp_global(&self) -> PathBuf {
        self.0.join("mcp/global")
    }

    pub fn mcp_project(&self, project_key: &str) -> PathBuf {
        self.0.join("mcp").join(project_key)
    }

    pub fn skills_global(&self) -> PathBuf {
        self.0.join("skills/global")
    }

    pub fn skills_project(&self, project_key: &str) -> PathBuf {
        self.0.join("skills").join(project_key)
    }

    pub fn config_json(&self) -> PathBuf {
        self.0.join("config.json")
    }
}

/// A project repository that receives per-agent symlinks and hard links.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectWorkspace(pub PathBuf);

impl ProjectWorkspace {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self(root.into())
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }
}
