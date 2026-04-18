//! Extension trait for plugins. Pure: a plugin returns [`FsOp`](crate::compile::FsOp) values; the
//! IO crate decides how to apply them.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::compile::FsOp;
use crate::id::ProjectKey;
use crate::schema::{PluginSchemaRegistry, PluginsSection, SchemaError};

/// Non-IO context given to a plugin. Plugins are pure: they inspect this and return `FsOp` values.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstallContext {
    pub agents_home: std::path::PathBuf,
    pub project_key: ProjectKey,
    pub project_path: std::path::PathBuf,
    #[serde(default)]
    pub force: bool,
    #[serde(default)]
    pub dry_run: bool,
}

impl InstallContext {
    pub fn new(
        agents_home: impl AsRef<Path>,
        project_key: impl Into<ProjectKey>,
        project_path: impl AsRef<Path>,
    ) -> Self {
        Self {
            agents_home: agents_home.as_ref().to_path_buf(),
            project_key: project_key.into(),
            project_path: project_path.as_ref().to_path_buf(),
            force: false,
            dry_run: false,
        }
    }
}

/// A user-supplied plugin that adds operations to the compiled plan.
///
/// Plugins are pure: `plan` must not touch the filesystem or spawn processes. The IO crate will
/// apply the returned ops.
pub trait ProjectLinker: Send + Sync {
    fn id(&self) -> &'static str;

    fn plan(&self, ctx: &InstallContext) -> Vec<FsOp>;

    /// Called after the registry validates the `plugins.config.<id>` block.
    fn configure(&mut self, _config: serde_json::Value) {}
}

/// Tiny helper: validate `plugins.config.<id>` for each registered linker against the schema
/// registry, then return validated configs so a runtime can call [`ProjectLinker::configure`].
pub fn validate_plugin_configs(
    linkers: &[&dyn ProjectLinker],
    registry: &PluginSchemaRegistry,
    section: &PluginsSection,
) -> Result<Vec<(String, serde_json::Value)>, SchemaError> {
    let mut out = Vec::new();
    for l in linkers {
        let id = l.id().to_string();
        if let Some(v) = section.config.get(&id) {
            registry.validate_plugin_payload(&id, "linker", v)?;
            out.push((id, v.clone()));
        }
    }
    Ok(out)
}
