use std::fs;

use crate::model::{AgentId, LinkKind, PlannedLink};
use crate::plugins::InstallContext;

use super::{find_rule_base, push_unique};
use crate::install::types::InstallReport;

pub fn plan(ctx: &InstallContext<'_>, report: &mut InstallReport) -> Result<(), std::io::Error> {
    let rules_dir = ctx.project_path.join(".claude/rules");
    if !ctx.dry_run {
        fs::create_dir_all(&rules_dir)?;
    }
    let g = ctx.agents_home.join("rules/global");
    let p = ctx.agents_home.join("rules").join(ctx.project_key);

    if let Some(src) = find_rule_base(&g, "rules") {
        push_unique(
            &mut report.planned,
            PlannedLink {
                agent: AgentId::ClaudeCode,
                kind: LinkKind::Symlink,
                source: src.clone(),
                dest: rules_dir.join("global--rules.md"),
            },
        );
    }
    let claude_global = find_rule_base(&g, "claude-code").or_else(|| find_rule_base(&g, "claude"));
    if let Some(src) = claude_global {
        push_unique(
            &mut report.planned,
            PlannedLink {
                agent: AgentId::ClaudeCode,
                kind: LinkKind::Symlink,
                source: src,
                dest: rules_dir.join("global--claude-code.md"),
            },
        );
    }
    if let Some(src) = find_rule_base(&p, "rules") {
        push_unique(
            &mut report.planned,
            PlannedLink {
                agent: AgentId::ClaudeCode,
                kind: LinkKind::Symlink,
                source: src,
                dest: rules_dir.join("project--rules.md"),
            },
        );
    }
    let claude_proj = find_rule_base(&p, "claude-code").or_else(|| find_rule_base(&p, "claude"));
    if let Some(src) = claude_proj {
        push_unique(
            &mut report.planned,
            PlannedLink {
                agent: AgentId::ClaudeCode,
                kind: LinkKind::Symlink,
                source: src,
                dest: rules_dir.join("project--claude-code.md"),
            },
        );
    }

    let settings_proj = ctx
        .agents_home
        .join("settings")
        .join(ctx.project_key)
        .join("claude-code.json");
    if settings_proj.is_file() {
        push_unique(
            &mut report.planned,
            PlannedLink {
                agent: AgentId::ClaudeCode,
                kind: LinkKind::Symlink,
                source: settings_proj,
                dest: ctx.project_path.join(".claude/settings.local.json"),
            },
        );
    }

    let m_proj = ctx.agents_home.join("mcp").join(ctx.project_key).join("claude.json");
    let m_glob = ctx.agents_home.join("mcp/global/claude.json");
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
                agent: AgentId::ClaudeCode,
                kind: LinkKind::Symlink,
                source: src,
                dest: ctx.project_path.join(".mcp.json"),
            },
        );
    }

    Ok(())
}
