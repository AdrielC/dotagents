//! Optional extension points: register extra [`ProjectLinker`] implementations
//! (for example a future “Cursor marketplace plugin” layout) without forking core logic.
//!
//! Pair with [`crate::schema::PluginSchemaRegistry`] so each plugin can ship a JSON Schema
//! for its `plugins.config.<id>` block in `config.json`.

use std::path::Path;

use crate::model::{AgentId, PlannedLink};
use crate::schema::{PluginSchemaEntry, PluginSchemaRegistry, PluginsSection, SchemaError};

/// Resolved paths for one install run.
#[derive(Clone, Debug)]
pub struct InstallContext<'a> {
    pub agents_home: &'a Path,
    pub project_key: &'a str,
    pub project_path: &'a Path,
    pub force: bool,
    pub dry_run: bool,
}

/// Plugin hook: append additional planned links for a project.
pub trait ProjectLinker: Send + Sync {
    fn id(&self) -> &'static str;

    /// Return extra links to create after built-in agents (order is preserved).
    fn plan(&self, ctx: &InstallContext) -> Vec<PlannedLink>;

    /// Optional JSON config for this linker (from `config.json` → `plugins.config.<id>`).
    /// Default: ignored. Implement to read `serde_json::Value` after schema validation.
    fn configure(&mut self, _config: serde_json::Value) {}
}

/// Holds optional [`ProjectLinker`] registrations and plugin JSON Schemas.
#[derive(Default)]
pub struct PluginRegistry {
    linkers: Vec<Box<dyn ProjectLinker>>,
    pub schemas: PluginSchemaRegistry,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, linker: Box<dyn ProjectLinker>) {
        self.linkers.push(linker);
    }

    /// Register a schema entry (typically from `config.json` `plugins.schemas` or code).
    pub fn register_schema(&mut self, entry: PluginSchemaEntry) {
        self.schemas.register(entry);
    }

    /// Load `plugins` from merged config extra, validate declared configs, apply configs to linkers.
    pub fn sync_from_agents_config(
        &mut self,
        plugins: &PluginsSection,
    ) -> Result<(), SchemaError> {
        self.schemas.merge_from_config(plugins);
        self.schemas.validate_all_configs(plugins)?;
        for linker in self.linkers.iter_mut() {
            let id = linker.id().to_string();
            if let Some(v) = plugins.config.get(&id) {
                self.schemas
                    .validate_plugin_payload(&id, "linker", v)?;
                linker.configure(v.clone());
            }
        }
        Ok(())
    }

    pub fn plan_all(&self, ctx: &InstallContext) -> Vec<PlannedLink> {
        let mut out = Vec::new();
        for l in &self.linkers {
            out.extend(l.plan(ctx));
        }
        out
    }

    /// Built-in agent identifiers present in this crate (for filtering or UI).
    pub fn builtin_agent_ids() -> &'static [AgentId] {
        AgentId::all()
    }
}
