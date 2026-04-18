use serde::{Deserialize, Serialize};

use crate::model::{AgentId, PlannedLink};

#[derive(Clone, Debug, Default)]
pub struct InitOptions {
    pub force: bool,
}

#[derive(Clone, Debug, Default)]
pub struct InstallOptions {
    pub force: bool,
    pub dry_run: bool,
    /// When true, merge `projects.<name>` into `config.json`.
    pub register_project: bool,
    /// When set, only these agents are installed; when unset, all built-ins run.
    pub agents: Option<Vec<AgentId>>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct InstallReport {
    pub planned: Vec<PlannedLink>,
    pub applied: Vec<PlannedLink>,
    pub skipped: Vec<String>,
    pub warnings: Vec<String>,
    /// Optional [schema.org](https://schema.org) JSON-LD graph for tooling and audits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_org_json_ld: Option<serde_json::Value>,
}
