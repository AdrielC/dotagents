//! Planning phase: discover sources under `~/.agents` and append [`PlannedLink`](crate::model::PlannedLink).

mod claude;
mod codex;
mod cursor;
mod opencode;
mod skills;

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::PathBuf;

use crate::model::{AgentId, PlannedLink};
use crate::plugins::InstallContext;

use crate::install::types::InstallReport;

pub(crate) const SKILL_FILE: &str = "SKILL.md";

pub(crate) fn push_unique(plan: &mut Vec<PlannedLink>, link: PlannedLink) {
    if !plan.iter().any(|p| p.dest == link.dest && p.source == link.source) {
        plan.push(link);
    }
}

pub(crate) fn find_rule_base(dir: &std::path::Path, base: &str) -> Option<PathBuf> {
    for ext in ["md", "mdc", "txt"] {
        let p = dir.join(format!("{base}.{ext}"));
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

pub(crate) fn collect_skill_names(skills_root: &std::path::Path) -> io::Result<Vec<String>> {
    let mut names = Vec::new();
    if !skills_root.is_dir() {
        return Ok(names);
    }
    for entry in fs::read_dir(skills_root)? {
        let path = entry?.path();
        if path.is_dir() && path.join(SKILL_FILE).is_file() {
            if let Some(n) = path.file_name().and_then(|s| s.to_str()) {
                names.push(n.to_string());
            }
        }
    }
    Ok(names)
}

/// Append all planned links for built-in agents and cross-cutting skills.
pub fn plan_builtin(
    ctx: &InstallContext<'_>,
    enabled: &HashSet<AgentId>,
    report: &mut InstallReport,
) -> Result<(), io::Error> {
    if enabled.contains(&AgentId::Cursor) {
        cursor::plan(ctx, report)?;
    }
    if enabled.contains(&AgentId::ClaudeCode) {
        claude::plan(ctx, report)?;
    }
    if enabled.contains(&AgentId::Codex) {
        codex::plan(ctx, report)?;
    }
    if enabled.contains(&AgentId::OpenCode) {
        opencode::plan(ctx, report)?;
    }

    skills::plan_cursor(ctx, report)?;
    skills::plan_claude(ctx, report)?;
    skills::plan_codex(ctx, report)?;
    Ok(())
}
