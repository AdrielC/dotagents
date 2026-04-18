//! `config.json` reader/writer with default-template merge.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

impl Default for AgentsConfig {
    fn default() -> Self {
        let v = default_config_json();
        serde_json::from_value(v).unwrap_or(Self {
            version: 1,
            projects: BTreeMap::new(),
            extra: Default::default(),
        })
    }
}

pub fn read_config(path: &Path) -> std::io::Result<AgentsConfig> {
    if !path.exists() {
        return Ok(AgentsConfig::default());
    }
    let text = std::fs::read_to_string(path)?;
    let mut base: Value = default_config_json();
    let overlay: Value = serde_json::from_str(&text)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    merge_json(&mut base, overlay);
    serde_json::from_value(base)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

pub fn write_config(path: &Path, config: &AgentsConfig) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut base = default_config_json();
    let overlay = serde_json::to_value(config)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    merge_json(&mut base, overlay);
    let pretty = serde_json::to_string_pretty(&base)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(path, pretty + "\n")
}

fn merge_json(base: &mut Value, overlay: Value) {
    match (base, overlay) {
        (Value::Object(a), Value::Object(b)) => {
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

/// Default `config.json` template. Kept here (IO crate) because it's a file format, not a domain
/// invariant — the domain view is in [`agentz_core::schema::PluginsSection`].
pub fn default_config_json() -> Value {
    serde_json::json!({
        "$schema": "https://agentz.dev/schemas/config.json",
        "version": 1,
        "defaults": { "agent": "cursor" },
        "projects": {},
        "agents": {
            "cursor": { "enabled": true, "version_detected": null },
            "claude-code": { "enabled": true, "version_detected": null },
            "codex": { "enabled": true, "version_detected": null },
            "opencode": { "enabled": false, "version_detected": null }
        },
        "features": { "tasks": false, "history": false, "sync": false },
        "plugins": { "schemas": [], "config": {} }
    })
}
