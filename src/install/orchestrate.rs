//! Install **pipeline**: load config → plan → plugins → apply → register → schema.org audit graph.

use std::path::{Path, PathBuf};

use crate::config::{read_config, write_config, ProjectEntry};
use crate::model::LinkKind;
use crate::plugins::{InstallContext, PluginRegistry};
use crate::schema::plugins_section_from_config;
use crate::vocabulary;

use super::apply::apply_planned;
use super::error::InstallError;
use super::plan;
use super::policy::agents_enabled_filter;
use super::types::{InstallOptions, InstallReport};
use super::workspace::create_project_dirs;

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

    plan::plan_builtin(&ctx, &enabled, &mut report)?;

    if let Some(reg) = plugins {
        let plugins_section = plugins_section_from_config(&cfg);
        reg.sync_from_agents_config(&plugins_section)?;
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
        let canon =
            std::fs::canonicalize(project_path).unwrap_or_else(|_| project_path.to_path_buf());
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

    attach_schema_org_graph(&mut report, project_key, project_path, opts);
    Ok(report)
}

fn attach_schema_org_graph(
    report: &mut InstallReport,
    project_key: &str,
    project_path: &Path,
    opts: &InstallOptions,
) {
    let applied_paths: Vec<PathBuf> = report.applied.iter().map(|l| l.dest.clone()).collect();
    let link_pairs: Vec<(PathBuf, PathBuf, &'static str)> = report
        .applied
        .iter()
        .map(|l| {
            let kind = match l.kind {
                LinkKind::HardLink => "HardLink",
                LinkKind::Symlink => "Symlink",
            };
            (l.source.clone(), l.dest.clone(), kind)
        })
        .collect();
    report.schema_org_json_ld = Some(vocabulary::json_ld_install_report(
        project_key,
        project_path,
        opts.dry_run,
        &applied_paths,
        &link_pairs,
    ));
}
