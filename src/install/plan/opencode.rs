use std::collections::HashSet;
use std::fs;

use crate::model::{AgentId, LinkKind, PlannedLink};
use crate::plugins::InstallContext;

use super::push_unique;
use crate::install::types::InstallReport;

pub fn plan(ctx: &InstallContext<'_>, report: &mut InstallReport) -> Result<(), std::io::Error> {
    let s_proj = ctx
        .agents_home
        .join("settings")
        .join(ctx.project_key)
        .join("opencode.json");
    let s_glob = ctx.agents_home.join("settings/global/opencode.json");
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
                agent: AgentId::OpenCode,
                kind: LinkKind::Symlink,
                source: src,
                dest: ctx.project_path.join("opencode.json"),
            },
        );
    }

    let agent_dir = ctx.project_path.join(".opencode/agent");
    if !ctx.dry_run {
        fs::create_dir_all(&agent_dir)?;
    }

    let proj_rules = ctx.agents_home.join("rules").join(ctx.project_key);
    let glob_rules = ctx.agents_home.join("rules/global");
    let mut opencode_dest: HashSet<std::path::PathBuf> = HashSet::new();
    for dir in [proj_rules, glob_rules] {
        if !dir.is_dir() {
            continue;
        }
        for entry in fs::read_dir(&dir)? {
            let path = entry?.path();
            let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if !name.starts_with("opencode-") || !path.is_file() {
                continue;
            }
            let target_name = name.trim_start_matches("opencode-");
            let dest = agent_dir.join(target_name);
            if !opencode_dest.insert(dest.clone()) {
                continue;
            }
            push_unique(
                &mut report.planned,
                PlannedLink {
                    agent: AgentId::OpenCode,
                    kind: LinkKind::Symlink,
                    source: path,
                    dest,
                },
            );
        }
    }

    Ok(())
}
