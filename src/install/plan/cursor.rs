use std::fs;

use crate::model::{cursor_display_name, AgentId, LinkKind, PlannedLink};
use crate::plugins::InstallContext;

use super::push_unique;
use crate::install::types::InstallReport;

pub fn plan(ctx: &InstallContext<'_>, report: &mut InstallReport) -> Result<(), std::io::Error> {
    let global_rules = ctx.agents_home.join("rules/global");
    let proj_rules = ctx.agents_home.join("rules").join(ctx.project_key);
    let dest_dir = ctx.project_path.join(".cursor/rules");
    if !ctx.dry_run {
        fs::create_dir_all(&dest_dir)?;
    }

    for (scope, dir) in [("global", global_rules.clone()), (ctx.project_key, proj_rules.clone())] {
        if !dir.is_dir() {
            continue;
        }
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if !name.ends_with(".md") && !name.ends_with(".mdc") {
                continue;
            }
            let display = cursor_display_name(name);
            let dest = dest_dir.join(format!("{scope}--{display}"));
            push_unique(
                &mut report.planned,
                PlannedLink {
                    agent: AgentId::Cursor,
                    kind: LinkKind::HardLink,
                    source: path,
                    dest,
                },
            );
        }
    }

    let dest_base = ctx.project_path.join(".cursor");
    if !ctx.dry_run {
        fs::create_dir_all(&dest_base)?;
    }

    let s_proj = ctx
        .agents_home
        .join("settings")
        .join(ctx.project_key)
        .join("cursor.json");
    let s_glob = ctx.agents_home.join("settings/global/cursor.json");
    let pick = if s_proj.is_file() {
        Some(s_proj)
    } else if s_glob.is_file() {
        Some(s_glob)
    } else {
        None
    };
    if let Some(src) = pick {
        push_unique(
            &mut report.planned,
            PlannedLink {
                agent: AgentId::Cursor,
                kind: LinkKind::HardLink,
                source: src,
                dest: dest_base.join("settings.json"),
            },
        );
    }

    let m_proj = ctx.agents_home.join("mcp").join(ctx.project_key).join("cursor.json");
    let m_glob = ctx.agents_home.join("mcp/global/cursor.json");
    let mpick = if m_proj.is_file() {
        Some(m_proj)
    } else if m_glob.is_file() {
        Some(m_glob)
    } else {
        None
    };
    if let Some(src) = mpick {
        push_unique(
            &mut report.planned,
            PlannedLink {
                agent: AgentId::Cursor,
                kind: LinkKind::HardLink,
                source: src,
                dest: dest_base.join("mcp.json"),
            },
        );
    }

    let i_proj = ctx
        .agents_home
        .join("settings")
        .join(ctx.project_key)
        .join("cursorignore");
    let i_glob = ctx.agents_home.join("settings/global/cursorignore");
    let ipick = if i_proj.is_file() {
        Some(i_proj)
    } else if i_glob.is_file() {
        Some(i_glob)
    } else {
        None
    };
    if let Some(src) = ipick {
        push_unique(
            &mut report.planned,
            PlannedLink {
                agent: AgentId::Cursor,
                kind: LinkKind::HardLink,
                source: src,
                dest: ctx.project_path.join(".cursorignore"),
            },
        );
    }

    Ok(())
}
