use std::fs;

use crate::model::{AgentId, LinkKind, PlannedLink};
use crate::plugins::InstallContext;

use super::push_unique;
use crate::install::types::InstallReport;

pub fn plan(ctx: &InstallContext<'_>, report: &mut InstallReport) -> Result<(), std::io::Error> {
    let g = ctx.agents_home.join("rules/global");
    let p = ctx.agents_home.join("rules").join(ctx.project_key);

    let mut src: Option<std::path::PathBuf> = None;
    if p.join("agents.md").is_file() {
        src = Some(p.join("agents.md"));
    } else if g.join("agents.md").is_file() {
        src = Some(g.join("agents.md"));
    } else if g.join("rules.md").is_file() {
        src = Some(g.join("rules.md"));
    }
    if let Some(s) = src {
        push_unique(
            &mut report.planned,
            PlannedLink {
                agent: AgentId::Codex,
                kind: LinkKind::Symlink,
                source: s,
                dest: ctx.project_path.join("AGENTS.md"),
            },
        );
    }

    let codex_dir = ctx.project_path.join(".codex");
    if !ctx.dry_run {
        fs::create_dir_all(&codex_dir)?;
    }
    let t_proj = ctx
        .agents_home
        .join("settings")
        .join(ctx.project_key)
        .join("codex.toml");
    let t_glob = ctx.agents_home.join("settings/global/codex.toml");
    let tpick = if t_proj.is_file() {
        Some(t_proj)
    } else if t_glob.is_file() {
        Some(t_glob)
    } else {
        None
    };
    if let Some(src) = tpick {
        push_unique(
            &mut report.planned,
            PlannedLink {
                agent: AgentId::Codex,
                kind: LinkKind::Symlink,
                source: src,
                dest: codex_dir.join("config.toml"),
            },
        );
    }

    Ok(())
}
