//! Optional extension points: register extra [`ProjectLinker`] implementations
//! (for example a future “Cursor marketplace plugin” layout) without forking core logic.

use std::path::Path;

use crate::model::{AgentId, PlannedLink};

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
}

/// Holds optional [`ProjectLinker`] registrations.
#[derive(Default)]
pub struct PluginRegistry {
    linkers: Vec<Box<dyn ProjectLinker>>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, linker: Box<dyn ProjectLinker>) {
        self.linkers.push(linker);
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
