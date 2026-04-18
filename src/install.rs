//! Initialize `~/.agents/` and install per-project links (Cursor hard links, other agents symlinks).

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::symlink;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::config::{read_config, write_config, AgentsConfig, ProjectEntry};
use crate::model::{cursor_display_name, AgentId, LinkKind, PlannedLink};
use crate::plugins::{InstallContext, PluginRegistry};
use crate::schema::plugins_section_from_config;

const SKILL_FILE: &str = "SKILL.md";

#[derive(Debug, Error)]
pub enum InstallError {
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("plugin schema: {0}")]
    PluginSchema(#[from] crate::schema::SchemaError),
    #[error("symlinks are only supported on unix targets in this build")]
    SymlinkNotSupported,
    #[error("refusing to replace existing path without force: {0}")]
    Exists(PathBuf),
    #[error("source missing: {0}")]
    MissingSource(PathBuf),
}

#[derive(Clone, Debug, Default)]
pub struct InitOptions {
    pub force: bool,
}

#[derive(Clone, Debug, Default)]
pub struct InstallOptions {
    pub force: bool,
    pub dry_run: bool,
    /// When true, merge `projects.<name>` into `config.json`.
    pub register_project: bool,
    /// When set, only these agents are installed; when unset, all built-ins run.
    pub agents: Option<Vec<AgentId>>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct InstallReport {
    pub planned: Vec<PlannedLink>,
    pub applied: Vec<PlannedLink>,
    pub skipped: Vec<String>,
    pub warnings: Vec<String>,
}

fn agents_enabled_filter(cfg: &AgentsConfig, opt: &InstallOptions) -> HashSet<AgentId> {
    if let Some(ref list) = opt.agents {
        return list.iter().copied().collect();
    }
    let mut set = HashSet::new();
    for a in AgentId::all() {
        if agent_enabled_in_config(cfg, *a) {
            set.insert(*a);
        }
    }
    if set.is_empty() {
        AgentId::all().iter().copied().collect()
    } else {
        set
    }
}

fn agent_enabled_in_config(cfg: &AgentsConfig, agent: AgentId) -> bool {
    let key = format!("agents.{}.enabled", agent.as_str());
    if let Some(v) = cfg.extra.get("agents") {
        if let Some(obj) = v.as_object() {
            if let Some(agent_obj) = obj.get(agent.as_str()) {
                if let Some(en) = agent_obj.get("enabled") {
                    return en.as_bool().unwrap_or(true);
                }
            }
        }
    }
    // Fallback: parse dotted path not used; opencode default false in template
    let _ = key;
    match agent {
        AgentId::OpenCode => cfg
            .extra
            .get("agents")
            .and_then(|v| v.get("opencode"))
            .and_then(|o| o.get("enabled"))
            .and_then(|b| b.as_bool())
            .unwrap_or(false),
        _ => true,
    }
}

/// Create the standard `~/.agents/` tree when missing (safe to call repeatedly).
pub fn init_agents_home(agents_home: &Path, opts: &InitOptions) -> io::Result<()> {
    let dirs = [
        agents_home.join("rules/global"),
        agents_home.join("rules/_example"),
        agents_home.join("settings/global"),
        agents_home.join("mcp/global"),
        agents_home.join("skills/global"),
        agents_home.join("scripts"),
        agents_home.join("local"),
    ];
    for d in &dirs {
        fs::create_dir_all(d)?;
    }

    let config_path = agents_home.join("config.json");
    if !config_path.exists() || opts.force {
        let cfg = if config_path.exists() && opts.force {
            read_config(&config_path).unwrap_or_default()
        } else {
            AgentsConfig::default()
        };
        write_config(&config_path, &cfg)?;
    }

    let starter = agents_home.join("rules/global/rules.mdc");
    if !starter.exists() {
        fs::write(
            starter,
            b"---\ndescription: Starter rules managed by agents-unified\nglobs: []\nalwaysApply: true\n---\n\n# Rules\n\nEdit shared rules under `~/.agents/rules/global/`.\n",
        )?;
    }

    Ok(())
}

/// Plan and apply links for one repo (built-in agents + optional plugins).
pub fn install_project(
    agents_home: &Path,
    project_key: &str,
    project_path: &Path,
    opts: &InstallOptions,
    plugins: Option<&mut PluginRegistry>,
) -> Result<InstallReport, InstallError> {
    let mut report = InstallReport::default();
    let cfg_path = agents_home.join("config.json");
    let cfg = read_config(&cfg_path)?;
    let enabled = agents_enabled_filter(&cfg, opts);

    create_project_dirs(agents_home, project_key, opts.dry_run)?;

    let ctx = InstallContext {
        agents_home,
        project_key,
        project_path,
        force: opts.force,
        dry_run: opts.dry_run,
    };

    if enabled.contains(&AgentId::Cursor) {
        plan_cursor(&ctx, &mut report)?;
    }
    if enabled.contains(&AgentId::ClaudeCode) {
        plan_claude(&ctx, &mut report)?;
    }
    if enabled.contains(&AgentId::Codex) {
        plan_codex(&ctx, &mut report)?;
    }
    if enabled.contains(&AgentId::OpenCode) {
        plan_opencode(&ctx, &mut report)?;
    }

    plan_skills_cursor(&ctx, &mut report)?;
    plan_skills_claude(&ctx, &mut report)?;
    plan_skills_codex(&ctx, &mut report)?;

    if let Some(reg) = plugins {
        let plugins_section = plugins_section_from_config(&cfg);
        if let Err(e) = reg.sync_from_agents_config(&plugins_section) {
            return Err(InstallError::PluginSchema(e));
        }
        let pctx = InstallContext {
            agents_home,
            project_key,
            project_path,
            force: opts.force,
            dry_run: opts.dry_run,
        };
        for link in reg.plan_all(&pctx) {
            report.planned.push(link);
        }
    }

    apply_planned(&mut report, opts)?;

    if opts.register_project && !opts.dry_run {
        let mut cfg = read_config(&cfg_path)?;
        let canon = fs::canonicalize(project_path).unwrap_or_else(|_| project_path.to_path_buf());
        cfg.projects.insert(
            project_key.to_string(),
            ProjectEntry {
                path: canon,
                added: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            },
        );
        write_config(&cfg_path, &cfg)?;
    } else if opts.register_project && opts.dry_run {
        report
            .warnings
            .push("dry-run: skipped writing config.json".into());
    }

    Ok(report)
}

fn create_project_dirs(agents_home: &Path, project_key: &str, dry_run: bool) -> io::Result<()> {
    if dry_run {
        return Ok(());
    }
    for sub in ["rules", "settings", "mcp", "skills"] {
        fs::create_dir_all(agents_home.join(sub).join(project_key))?;
    }
    Ok(())
}

fn push_unique(plan: &mut Vec<PlannedLink>, link: PlannedLink) {
    if !plan.iter().any(|p| p.dest == link.dest && p.source == link.source) {
        plan.push(link);
    }
}

fn plan_cursor(ctx: &InstallContext, report: &mut InstallReport) -> Result<(), InstallError> {
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

    // settings.json
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

    // mcp.json
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

    // .cursorignore
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

fn find_rule_base(dir: &Path, base: &str) -> Option<PathBuf> {
    for ext in ["md", "mdc", "txt"] {
        let p = dir.join(format!("{base}.{ext}"));
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

fn plan_claude(ctx: &InstallContext, report: &mut InstallReport) -> Result<(), InstallError> {
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
    let claude_global = find_rule_base(&g, "claude-code")
        .or_else(|| find_rule_base(&g, "claude"));
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

fn plan_codex(ctx: &InstallContext, report: &mut InstallReport) -> Result<(), InstallError> {
    let g = ctx.agents_home.join("rules/global");
    let p = ctx.agents_home.join("rules").join(ctx.project_key);

    let mut src: Option<PathBuf> = None;
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

fn plan_opencode(ctx: &InstallContext, report: &mut InstallReport) -> Result<(), InstallError> {
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
    let mut opencode_dest: HashSet<PathBuf> = HashSet::new();
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

fn collect_skill_names(skills_root: &Path) -> io::Result<Vec<String>> {
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

fn plan_skills_cursor(ctx: &InstallContext, report: &mut InstallReport) -> Result<(), InstallError> {
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

fn plan_skills_claude(ctx: &InstallContext, report: &mut InstallReport) -> Result<(), InstallError> {
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

fn plan_skills_codex(ctx: &InstallContext, report: &mut InstallReport) -> Result<(), InstallError> {
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

fn apply_planned(report: &mut InstallReport, opts: &InstallOptions) -> Result<(), InstallError> {
    let planned: Vec<_> = report.planned.clone();
    for link in planned {
        if opts.dry_run {
            report.skipped.push(format!("dry-run: {:?}", link.dest));
            continue;
        }
        match link.kind {
            LinkKind::HardLink => apply_hardlink(&link, opts.force, report)?,
            LinkKind::Symlink => apply_symlink(&link, opts.force, report)?,
        }
    }
    Ok(())
}

fn apply_hardlink(link: &PlannedLink, force: bool, report: &mut InstallReport) -> Result<(), InstallError> {
    if !link.source.is_file() {
        return Err(InstallError::MissingSource(link.source.clone()));
    }
    if let Some(parent) = link.dest.parent() {
        fs::create_dir_all(parent)?;
    }

    if let Ok(meta) = fs::symlink_metadata(&link.dest) {
        if meta.file_type().is_symlink() {
            if !force {
                return Err(InstallError::Exists(link.dest.clone()));
            }
            fs::remove_file(&link.dest)?;
        } else if link.dest.is_file() {
            if same_file(&link.source, &link.dest)? {
                report.applied.push(link.clone());
                return Ok(());
            }
            if !force {
                return Err(InstallError::Exists(link.dest.clone()));
            }
            fs::remove_file(&link.dest)?;
        } else if !force {
            return Err(InstallError::Exists(link.dest.clone()));
        } else {
            fs::remove_dir_all(&link.dest)?;
        }
    }

    fs::hard_link(&link.source, &link.dest)?;
    report.applied.push(link.clone());
    Ok(())
}

fn same_file(a: &Path, b: &Path) -> io::Result<bool> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let ma = fs::metadata(a)?;
        let mb = fs::metadata(b)?;
        Ok(ma.dev() == mb.dev() && ma.ino() == mb.ino())
    }
    #[cfg(not(unix))]
    {
        let _ = (a, b);
        Ok(false)
    }
}

fn apply_symlink(link: &PlannedLink, force: bool, report: &mut InstallReport) -> Result<(), InstallError> {
    #[cfg(not(unix))]
    {
        let _ = (link, force, report);
        return Err(InstallError::SymlinkNotSupported);
    }
    #[cfg(unix)]
    {
        if !link.source.exists() {
            return Err(InstallError::MissingSource(link.source.clone()));
        }
        if let Some(parent) = link.dest.parent() {
            fs::create_dir_all(parent)?;
        }

        let target = compute_symlink_target(&link.source, &link.dest)?;
        if link.dest.symlink_metadata().is_ok() {
            if link.dest.is_symlink() {
                if let Ok(cur) = fs::read_link(&link.dest) {
                    if cur == target {
                        report.applied.push(link.clone());
                        return Ok(());
                    }
                }
            }
            if !force {
                return Err(InstallError::Exists(link.dest.clone()));
            }
            if link.dest.is_dir() {
                fs::remove_dir_all(&link.dest)?;
            } else {
                fs::remove_file(&link.dest)?;
            }
        }

        symlink(&target, &link.dest)?;
        report.applied.push(link.clone());
        Ok(())
    }
}

fn compute_symlink_target(source: &Path, dest: &Path) -> io::Result<PathBuf> {
    let dest_dir = dest.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "symlink dest has no parent")
    })?;
    pathdiff::diff_paths(source, dest_dir).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "could not compute relative symlink target",
        )
    })
}
