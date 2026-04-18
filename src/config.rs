//! Read/write `config.json` in the common agents home layout.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::model::default_config_json;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectEntry {
    pub path: PathBuf,
    pub added: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentsConfig {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub projects: BTreeMap<String, ProjectEntry>,
    /// Extra keys (schema, agents, features, …) are preserved when round-tripping via JSON value merge in callers — see `read_config` / `write_config`.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Default for AgentsConfig {
    fn default() -> Self {
        let v = default_config_json();
        serde_json::from_value(v).unwrap_or_else(|_| Self {
            version: 1,
            projects: BTreeMap::new(),
            extra: serde_json::Map::new(),
        })
    }
}

/// Load `config.json`, merging with defaults for missing top-level keys.
pub fn read_config(path: &Path) -> std::io::Result<AgentsConfig> {
    if !path.exists() {
        return Ok(AgentsConfig::default());
    }
    let text = std::fs::read_to_string(path)?;
    let mut base: serde_json::Value = default_config_json();
    let overlay: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    merge_json(&mut base, overlay);
    serde_json::from_value(base).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

fn merge_json(base: &mut serde_json::Value, overlay: serde_json::Value) {
    match (base, overlay) {
        (serde_json::Value::Object(a), serde_json::Value::Object(b)) => {
            for (k, v) in b {
                if let Some(existing) = a.get_mut(&k) {
                    merge_json(existing, v);
                } else {
                    a.insert(k, v);
                }
            }
        }
        (slot, new) => *slot = new,
    }
}

/// Serialize config with stable key ordering where possible.
pub fn write_config(path: &Path, config: &AgentsConfig) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut base = default_config_json();
    let overlay =
        serde_json::to_value(config).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    merge_json(&mut base, overlay);
    let pretty = serde_json::to_string_pretty(&base)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(path, pretty + "\n")
}
