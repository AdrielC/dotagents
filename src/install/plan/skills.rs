use std::collections::HashSet;
use std::fs;

use crate::model::{AgentId, LinkKind, PlannedLink};
use crate::plugins::InstallContext;

use super::{collect_skill_names, push_unique, SKILL_FILE};
use crate::install::types::InstallReport;

pub fn plan_cursor(ctx: &InstallContext<'_>, report: &mut InstallReport) -> Result<(), std::io::Error> {
    let commands = ctx.project_path.join(".cursor/commands");
    if !ctx.dry_run {
        fs::create_dir_all(&commands)?;
    }
    let global = ctx.agents_home.join("skills/global");
    let project = ctx.agents_home.join("skills").join(ctx.project_key);
    let project_names: HashSet<String> = collect_skill_names(&project)?.into_iter().collect();

    for name in collect_skill_names(&global)? {
        if project_names.contains(&name) {
            report.warnings.push(format!(
                "skill '{name}' shadows global for Cursor commands (project wins)"
            ));
            continue;
        }
        let skill_md = global.join(&name).join(SKILL_FILE);
        if skill_md.is_file() {
            push_unique(
                &mut report.planned,
                PlannedLink {
                    agent: AgentId::Cursor,
                    kind: LinkKind::Symlink,
                    source: skill_md,
                    dest: commands.join(format!("{name}.md")),
                },
            );
        }
    }
    for name in &project_names {
        let skill_md = project.join(name).join(SKILL_FILE);
        if skill_md.is_file() {
            push_unique(
                &mut report.planned,
                PlannedLink {
                    agent: AgentId::Cursor,
                    kind: LinkKind::Symlink,
                    source: skill_md,
                    dest: commands.join(format!("{name}.md")),
                },
            );
        }
    }
    Ok(())
}

pub fn plan_claude(ctx: &InstallContext<'_>, report: &mut InstallReport) -> Result<(), std::io::Error> {
    let target = ctx.project_path.join(".claude/skills");
    if !ctx.dry_run {
        fs::create_dir_all(&target)?;
    }
    let global = ctx.agents_home.join("skills/global");
    let project = ctx.agents_home.join("skills").join(ctx.project_key);
    let project_names: HashSet<String> = collect_skill_names(&project)?.into_iter().collect();

    for name in collect_skill_names(&global)? {
        if project_names.contains(&name) {
            report.warnings.push(format!(
                "skill '{name}' shadows global for Claude skills (project wins)"
            ));
            continue;
        }
        let dir = global.join(&name);
        push_unique(
            &mut report.planned,
            PlannedLink {
                agent: AgentId::ClaudeCode,
                kind: LinkKind::Symlink,
                source: dir.clone(),
                dest: target.join(&name),
            },
        );
    }
    for name in &project_names {
        let dir = project.join(name);
        push_unique(
            &mut report.planned,
            PlannedLink {
                agent: AgentId::ClaudeCode,
                kind: LinkKind::Symlink,
                source: dir,
                dest: target.join(name),
            },
        );
    }
    Ok(())
}

pub fn plan_codex(ctx: &InstallContext<'_>, report: &mut InstallReport) -> Result<(), std::io::Error> {
    let target = ctx.project_path.join(".codex/skills");
    if !ctx.dry_run {
        fs::create_dir_all(&target)?;
    }
    let global = ctx.agents_home.join("skills/global");
    let project = ctx.agents_home.join("skills").join(ctx.project_key);
    let project_names: HashSet<String> = collect_skill_names(&project)?.into_iter().collect();

    for name in collect_skill_names(&global)? {
        if project_names.contains(&name) {
            report.warnings.push(format!(
                "skill '{name}' shadows global for Codex skills (project wins)"
            ));
            continue;
        }
        let dir = global.join(&name);
        push_unique(
            &mut report.planned,
            PlannedLink {
                agent: AgentId::Codex,
                kind: LinkKind::Symlink,
                source: dir.clone(),
                dest: target.join(&name),
            },
        );
    }
    for name in &project_names {
        let dir = project.join(name);
        push_unique(
            &mut report.planned,
            PlannedLink {
                agent: AgentId::Codex,
                kind: LinkKind::Symlink,
                source: dir,
                dest: target.join(name),
            },
        );
    }
    Ok(())
}
