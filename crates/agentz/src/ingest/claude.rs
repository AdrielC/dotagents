//! Lift an on-disk `.claude/` directory into an [`AgentsTree`].
//!
//! Walks the documented Claude Code surfaces in one pass (see
//! <https://code.claude.com/docs/en/overview>):
//!
//! | File / dir on disk                                  | Resulting leaf                            |
//! |-----------------------------------------------------|-------------------------------------------|
//! | `.claude/rules/*.md`                                | [`AgentsTree::Rules`] (agent-agnostic)    |
//! | `.claude/skills/<name>/SKILL.md`                    | [`AgentsTree::Skills`] (agent-agnostic)   |
//! | `.claude/agents/<name>.md`                          | [`AgentsTree::Agents`] (Claude-only)      |
//! | `.claude/commands/<name>.md`                        | Merged into [`AgentsTree::Skills`]        |
//! | `.claude/settings.json`                             | [`AgentsTree::Settings`] project scope    |
//! | `.claude/settings.local.json`                       | [`AgentsTree::Settings`] local scope      |
//! | `.claude/managed-settings.json`                     | [`AgentsTree::Settings`] managed scope    |
//! | `.claude/CLAUDE.md` OR `./CLAUDE.md` (repo root)    | [`AgentsTree::TextFile`] at repo root     |
//! | `.claude/.mcp.json` OR `./.mcp.json` (repo root)    | [`AgentsTree::Mcp`]                       |
//! | `./.claudeignore`                                   | [`AgentsTree::Ignore`] for `ClaudeCode`   |
//!
//! The resulting tree is deliberately **agent-agnostic where possible** — rules and skills are
//! bare leaves that [`agentz_core::compile`] fans out to every target, so once you ingest
//! a `.claude/` you can compile the same tree into `.cursor/` or `.codex/` without touching the
//! data again.
//!
//! Unknown files are preserved as [`IngestReport::unknown_paths`] so callers can see what we
//! silently skipped.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;

use agentz_core::model::IgnoreKind;
use agentz_core::tree::{
    AgentBody, AgentNode, AgentsTree, RuleBody, RuleNode, ScopeKind, SettingsBody, SettingsNode,
    SkillBody, SkillNode,
};
use agentz_core::{AgentId, SettingsScope};

/// Per-ingest configuration. Sensible defaults; override for non-standard repos.
#[derive(Clone, Debug)]
pub struct IngestOptions {
    /// Also check the **parent** of the `.claude/` directory for files Claude conventionally puts
    /// at the repo root (`CLAUDE.md`, `.mcp.json`, `.claudeignore`). Default: true.
    pub include_repo_root: bool,
}

impl Default for IngestOptions {
    fn default() -> Self {
        Self {
            include_repo_root: true,
        }
    }
}

/// Everything the ingest pass surfaced — both the structured tree and a diagnostic log of files
/// we chose not to (or couldn't) classify.
#[derive(Clone, Debug)]
pub struct IngestReport {
    /// The ingested tree; pass it straight to [`agentz_core::compile::compile`].
    pub tree: AgentsTree,
    /// Paths we found but didn't know what to do with. Not an error — useful for debugging.
    pub unknown_paths: Vec<PathBuf>,
    /// Non-fatal warnings (e.g. couldn't decode a file as UTF-8).
    pub warnings: Vec<String>,
}

#[derive(Debug, Error)]
pub enum IngestError {
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("not a directory: {0}")]
    NotADirectory(PathBuf),
    #[error("path does not exist: {0}")]
    Missing(PathBuf),
}

/// Walk a `.claude/` directory and return an `AgentsTree` describing its content.
///
/// `root` must point at the `.claude/` directory itself (not its parent). See [`IngestOptions`]
/// for the repo-root files we optionally pick up alongside.
pub fn ingest(root: &Path, opts: &IngestOptions) -> Result<IngestReport, IngestError> {
    if !root.exists() {
        return Err(IngestError::Missing(root.to_path_buf()));
    }
    if !root.is_dir() {
        return Err(IngestError::NotADirectory(root.to_path_buf()));
    }

    let mut children: Vec<AgentsTree> = Vec::new();
    let mut unknown: Vec<PathBuf> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    ingest_rules(root, &mut children, &mut warnings)?;
    ingest_skills(root, &mut children, &mut warnings)?;
    ingest_agents_dir(root, &mut children, &mut warnings)?;
    ingest_commands(root, &mut children, &mut warnings)?;
    ingest_settings(root, &mut children, &mut warnings)?;

    // Claude's memory file lives either in .claude/CLAUDE.md or at the repo root.
    let claude_md_in_dir = root.join("CLAUDE.md");
    if claude_md_in_dir.is_file() {
        match fs::read_to_string(&claude_md_in_dir) {
            Ok(body) => children.push(AgentsTree::TextFile {
                name: "CLAUDE.md".into(),
                body,
            }),
            Err(e) => warnings.push(format!("read {}: {e}", claude_md_in_dir.display())),
        }
    }

    // Record any top-level file in .claude/ we didn't classify.
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file()
            && !KNOWN_TOP_LEVEL.contains(&file_name_str(&path).as_deref().unwrap_or(""))
        {
            unknown.push(path);
        }
    }

    if opts.include_repo_root {
        if let Some(parent) = root.parent() {
            ingest_repo_root(
                parent,
                &mut children,
                &mut warnings,
                &mut unknown,
                &claude_md_in_dir,
            )?;
        }
    }

    let tree = AgentsTree::Scope {
        kind: ScopeKind::Global,
        children,
    };

    Ok(IngestReport {
        tree,
        unknown_paths: unknown,
        warnings,
    })
}

const KNOWN_TOP_LEVEL: &[&str] = &[
    "settings.json",
    "settings.local.json",
    "managed-settings.json",
    "CLAUDE.md",
    ".mcp.json",
];

fn file_name_str(path: &Path) -> Option<String> {
    path.file_name().and_then(|s| s.to_str()).map(str::to_owned)
}

fn ingest_rules(
    root: &Path,
    out: &mut Vec<AgentsTree>,
    warnings: &mut Vec<String>,
) -> io::Result<()> {
    let dir = root.join("rules");
    if !dir.is_dir() {
        return Ok(());
    }
    let mut rules: Vec<RuleNode> = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let Some(name) = file_name_str(&p) else {
            continue;
        };
        match fs::read_to_string(&p) {
            Ok(body) => rules.push(RuleNode {
                name,
                body: RuleBody::Inline(body),
            }),
            Err(e) => warnings.push(format!("read {}: {e}", p.display())),
        }
    }
    rules.sort_by(|a, b| a.name.cmp(&b.name));
    if !rules.is_empty() {
        out.push(AgentsTree::Rules(rules));
    }
    Ok(())
}

fn ingest_skills(
    root: &Path,
    out: &mut Vec<AgentsTree>,
    warnings: &mut Vec<String>,
) -> io::Result<()> {
    let dir = root.join("skills");
    if !dir.is_dir() {
        return Ok(());
    }
    let mut skills: Vec<SkillNode> = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let p = entry.path();
        if !p.is_dir() {
            continue;
        }
        let Some(name) = file_name_str(&p) else {
            continue;
        };
        let manifest = p.join("SKILL.md");
        if manifest.is_file() {
            match fs::read_to_string(&manifest) {
                Ok(body) => skills.push(SkillNode {
                    name,
                    body: SkillBody::Inline(body),
                }),
                Err(e) => warnings.push(format!("read {}: {e}", manifest.display())),
            }
        } else {
            warnings.push(format!("skill dir without SKILL.md: {}", p.display()));
        }
    }
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    if !skills.is_empty() {
        out.push(AgentsTree::Skills(skills));
    }
    Ok(())
}

fn ingest_agents_dir(
    root: &Path,
    out: &mut Vec<AgentsTree>,
    warnings: &mut Vec<String>,
) -> io::Result<()> {
    let dir = root.join("agents");
    if !dir.is_dir() {
        return Ok(());
    }
    let mut agents: Vec<AgentNode> = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let Some(stem) = p.file_stem().and_then(|s| s.to_str()).map(str::to_owned) else {
            continue;
        };
        match fs::read_to_string(&p) {
            Ok(body) => agents.push(AgentNode {
                name: stem,
                body: AgentBody::Inline(body),
            }),
            Err(e) => warnings.push(format!("read {}: {e}", p.display())),
        }
    }
    agents.sort_by(|a, b| a.name.cmp(&b.name));
    if !agents.is_empty() {
        out.push(AgentsTree::Agents(agents));
    }
    Ok(())
}

fn ingest_commands(
    root: &Path,
    out: &mut Vec<AgentsTree>,
    warnings: &mut Vec<String>,
) -> io::Result<()> {
    // Legacy `.claude/commands/` maps onto SkillNodes since that's the current canonical shape.
    let dir = root.join("commands");
    if !dir.is_dir() {
        return Ok(());
    }
    let mut skills: Vec<SkillNode> = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let Some(stem) = p.file_stem().and_then(|s| s.to_str()).map(str::to_owned) else {
            continue;
        };
        match fs::read_to_string(&p) {
            Ok(body) => skills.push(SkillNode {
                name: stem,
                body: SkillBody::Inline(body),
            }),
            Err(e) => warnings.push(format!("read {}: {e}", p.display())),
        }
    }
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    if !skills.is_empty() {
        // Folded into Skills; merge_scope_children will dedupe if skills/ already produced one.
        out.push(AgentsTree::Skills(skills));
    }
    Ok(())
}

fn ingest_settings(
    root: &Path,
    out: &mut Vec<AgentsTree>,
    warnings: &mut Vec<String>,
) -> io::Result<()> {
    let files = [
        (SettingsScope::Project, "settings.json"),
        (SettingsScope::Local, "settings.local.json"),
        (SettingsScope::Managed, "managed-settings.json"),
    ];
    let mut nodes: Vec<SettingsNode> = Vec::new();
    for (scope, name) in files {
        let path = root.join(name);
        if !path.is_file() {
            continue;
        }
        match fs::read_to_string(&path) {
            Ok(body) => nodes.push(SettingsNode {
                agent: AgentId::ClaudeCode,
                scope,
                file_name: None,
                body: SettingsBody::Inline(body),
            }),
            Err(e) => warnings.push(format!("read {}: {e}", path.display())),
        }
    }
    if !nodes.is_empty() {
        out.push(AgentsTree::Settings(nodes));
    }
    Ok(())
}

fn ingest_repo_root(
    parent: &Path,
    out: &mut Vec<AgentsTree>,
    warnings: &mut Vec<String>,
    unknown: &mut Vec<PathBuf>,
    claude_md_already_found: &Path,
) -> io::Result<()> {
    // CLAUDE.md at repo root (if not already picked up inside .claude/).
    let repo_md = parent.join("CLAUDE.md");
    if repo_md.is_file() && !claude_md_already_found.is_file() {
        match fs::read_to_string(&repo_md) {
            Ok(body) => out.push(AgentsTree::TextFile {
                name: "CLAUDE.md".into(),
                body,
            }),
            Err(e) => warnings.push(format!("read {}: {e}", repo_md.display())),
        }
    }

    // .mcp.json at repo root.
    let mcp = parent.join(".mcp.json");
    if mcp.is_file() {
        match fs::read_to_string(&mcp) {
            Ok(text) => match serde_json::from_str::<serde_json::Value>(&text) {
                Ok(v) => out.push(AgentsTree::Mcp(v)),
                Err(e) => warnings.push(format!("parse {}: {e}", mcp.display())),
            },
            Err(e) => warnings.push(format!("read {}: {e}", mcp.display())),
        }
    }

    // .claudeignore at repo root.
    let ignore = parent.join(".claudeignore");
    if ignore.is_file() {
        match fs::read_to_string(&ignore) {
            Ok(text) => {
                let patterns: Vec<String> = text
                    .lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty() && !l.starts_with('#'))
                    .map(str::to_owned)
                    .collect();
                if !patterns.is_empty() {
                    out.push(AgentsTree::Ignore {
                        agent: AgentId::ClaudeCode,
                        kind: IgnoreKind::Primary,
                        patterns,
                    });
                }
            }
            Err(e) => warnings.push(format!("read {}: {e}", ignore.display())),
        }
    }

    // Any other dotfiles at the repo root are noise (node_modules, .gitignore, etc.) — don't
    // scan. We only record unclassified files below .claude/ in `unknown`; this is by design.
    let _ = unknown;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write(path: &Path, content: &str) {
        if let Some(p) = path.parent() {
            fs::create_dir_all(p).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    #[test]
    fn ingest_minimal_claude_dir() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join(".claude");
        write(
            &root.join("rules/010-style.md"),
            "---\nalwaysApply: true\n---\n# Style\n",
        );
        write(
            &root.join("skills/review/SKILL.md"),
            "---\nname: review\ndescription: review diffs\n---\nbody\n",
        );
        write(
            &root.join("agents/planner.md"),
            "---\nname: planner\ndescription: plan the work\n---\nprompt\n",
        );
        write(&root.join("settings.json"), "{\"ok\":true}\n");
        write(&root.join("settings.local.json"), "{\"me\":true}\n");

        let report = ingest(&root, &IngestOptions::default()).unwrap();
        let AgentsTree::Scope { children, .. } = &report.tree else {
            panic!("expected Scope");
        };
        // We expect: Rules, Skills, Agents, Settings.
        assert!(children.iter().any(|c| matches!(c, AgentsTree::Rules(_))));
        assert!(children.iter().any(|c| matches!(c, AgentsTree::Skills(_))));
        assert!(children.iter().any(|c| matches!(c, AgentsTree::Agents(_))));
        assert!(children
            .iter()
            .any(|c| matches!(c, AgentsTree::Settings(_))));
    }

    #[test]
    fn ingest_picks_up_repo_root_mcp_and_ignore() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join(".claude");
        fs::create_dir_all(&root).unwrap();
        write(
            &tmp.path().join(".mcp.json"),
            "{\"mcpServers\": {\"x\": {\"command\": \"echo\"}}}\n",
        );
        write(
            &tmp.path().join(".claudeignore"),
            "node_modules/\n.env\n# comment\n",
        );

        let report = ingest(&root, &IngestOptions::default()).unwrap();
        let AgentsTree::Scope { children, .. } = &report.tree else {
            panic!("expected Scope");
        };
        assert!(children.iter().any(|c| matches!(c, AgentsTree::Mcp(_))));
        let ignore = children
            .iter()
            .find_map(|c| match c {
                AgentsTree::Ignore {
                    agent,
                    kind,
                    patterns,
                } if *agent == AgentId::ClaudeCode && *kind == IgnoreKind::Primary => {
                    Some(patterns.clone())
                }
                _ => None,
            })
            .expect("ignore leaf");
        assert_eq!(ignore, vec!["node_modules/", ".env"]);
    }
}
