//! Pure compilation pass: `(AgentsTree, CompileContext) → CompiledPlan`.
//!
//! `CompiledPlan` is a list of [`FsOp`] values describing the exact filesystem operations an IO
//! layer should perform. This module itself never touches disk.

use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::model::{cursor_display_name, AgentId, LinkKind, PlannedLink};
use crate::tree::{AgentsTree, RuleBody, SettingsBody, SkillBody};

/// Context the compiler uses to resolve destinations relative to a project root.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CompileContext {
    pub project_path: PathBuf,
    /// Project key used for Cursor rule scope prefixes.
    pub project_key: String,
    /// When true, the compiler will emit copies instead of hard links for the rule rewrite.
    /// Useful for dry-runs and for filesystems that don't support hard links.
    #[serde(default)]
    pub force_copy_for_rules: bool,
}

impl CompileContext {
    pub fn new(project_path: impl Into<PathBuf>, project_key: impl Into<String>) -> Self {
        Self {
            project_path: project_path.into(),
            project_key: project_key.into(),
            force_copy_for_rules: false,
        }
    }
}

/// A single filesystem operation the IO layer must perform to apply the plan.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum FsOp {
    /// Ensure a directory exists.
    MkdirP { path: PathBuf },
    /// Write a file atomically. Used for inline rules/text artifacts.
    WriteFile {
        path: PathBuf,
        #[serde(default = "default_overwrite")]
        overwrite: bool,
        content: String,
    },
    /// Link `source` into `dest` using `kind`. Owned by `agent` for attribution.
    Link(PlannedLink),
}

fn default_overwrite() -> bool {
    false
}

#[derive(Debug, Error)]
pub enum CompileError {
    #[error("duplicate destination in plan: {0}")]
    DuplicateDest(PathBuf),
}

/// Output of [`compile`]. Ordering is deterministic within each scope.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CompiledPlan {
    pub ops: Vec<FsOp>,
    pub warnings: Vec<String>,
}

impl CompiledPlan {
    pub fn push(&mut self, op: FsOp) {
        self.ops.push(op);
    }

    pub fn extend(&mut self, ops: impl IntoIterator<Item = FsOp>) {
        self.ops.extend(ops);
    }

    /// Return only the [`PlannedLink`]s in emission order.
    pub fn links(&self) -> impl Iterator<Item = &PlannedLink> {
        self.ops.iter().filter_map(|op| match op {
            FsOp::Link(l) => Some(l),
            _ => None,
        })
    }
}

/// Compile a tree into a plan. Pure.
pub fn compile(tree: &AgentsTree, ctx: &CompileContext) -> Result<CompiledPlan, CompileError> {
    let mut plan = CompiledPlan::default();
    compile_node(tree, ctx, "global", &mut plan)?;
    // Deduplicate by (source, dest) so two scopes producing the same op collapse.
    let mut seen: Vec<(PathBuf, PathBuf)> = Vec::new();
    plan.ops.retain(|op| match op {
        FsOp::Link(l) => {
            let key = (l.source.clone(), l.dest.clone());
            if seen.iter().any(|k| k == &key) {
                false
            } else {
                seen.push(key);
                true
            }
        }
        _ => true,
    });
    Ok(plan)
}

fn compile_node(
    node: &AgentsTree,
    ctx: &CompileContext,
    current_scope: &str,
    plan: &mut CompiledPlan,
) -> Result<(), CompileError> {
    match node {
        AgentsTree::Scope { name, children } => {
            for c in children {
                compile_node(c, ctx, name, plan)?;
            }
        }
        AgentsTree::Rules(rules) => {
            for backend in rule_backends() {
                backend.emit(rules, ctx, current_scope, plan);
            }
        }
        AgentsTree::Skills(skills) => {
            emit_skill_commands(skills, ctx, plan);
        }
        AgentsTree::Settings(settings) => {
            emit_settings(settings, ctx, plan);
        }
        AgentsTree::Mcp(mcp_json) => {
            emit_mcp(mcp_json, ctx, plan);
        }
        AgentsTree::TextFile { name, body } => {
            plan.push(FsOp::WriteFile {
                path: ctx.project_path.join(name),
                overwrite: false,
                content: body.clone(),
            });
        }
    }
    Ok(())
}

/// Emits [`FsOp`]s for one agent’s rule layout (Cursor rules dir, Claude rules dir, Codex `AGENTS.md`, …).
trait RuleBackend {
    fn emit(
        &self,
        rules: &[crate::tree::RuleNode],
        ctx: &CompileContext,
        scope: &str,
        plan: &mut CompiledPlan,
    );
}

fn rule_backends() -> [&'static dyn RuleBackend; 3] {
    [&CursorRulesBackend, &ClaudeRulesBackend, &CodexRulesBackend]
}

struct CursorRulesBackend;
struct ClaudeRulesBackend;
struct CodexRulesBackend;

impl RuleBackend for CursorRulesBackend {
    fn emit(
        &self,
        rules: &[crate::tree::RuleNode],
        ctx: &CompileContext,
        scope: &str,
        plan: &mut CompiledPlan,
    ) {
        let dest_dir = ctx.project_path.join(".cursor").join("rules");
        plan.push(FsOp::MkdirP {
            path: dest_dir.clone(),
        });

        for r in rules {
            let display = cursor_display_name(&r.name);
            let dest = dest_dir.join(format!("{scope}--{display}"));
            let link_kind = if ctx.force_copy_for_rules {
                LinkKind::Copy
            } else {
                LinkKind::HardLink
            };
            emit_rule_body(plan, AgentId::Cursor, link_kind, dest, &r.body);
        }
    }
}

impl RuleBackend for ClaudeRulesBackend {
    fn emit(
        &self,
        rules: &[crate::tree::RuleNode],
        ctx: &CompileContext,
        scope: &str,
        plan: &mut CompiledPlan,
    ) {
        let dest_dir = ctx.project_path.join(".claude").join("rules");
        plan.push(FsOp::MkdirP {
            path: dest_dir.clone(),
        });

        for r in rules {
            let base = r.name.trim_end_matches(".md").trim_end_matches(".mdc");
            let dest = dest_dir.join(format!("{scope}--{base}.md"));
            emit_rule_body(plan, AgentId::ClaudeCode, LinkKind::Symlink, dest, &r.body);
        }
    }
}

impl RuleBackend for CodexRulesBackend {
    fn emit(
        &self,
        rules: &[crate::tree::RuleNode],
        ctx: &CompileContext,
        scope: &str,
        plan: &mut CompiledPlan,
    ) {
        // Codex binds a single AGENTS.md at the repo root. Prefer a rule literally named
        // `agents` in the project scope, then in the global scope.
        if scope != "global" && scope != ctx.project_key {
            return;
        }
        let Some(r) = rules.iter().find(|r| r.name.starts_with("agents.")) else {
            return;
        };
        let dest = ctx.project_path.join("AGENTS.md");
        emit_rule_body(plan, AgentId::Codex, LinkKind::Symlink, dest, &r.body);
    }
}

/// Shared emission for inline vs linked rule bodies.
fn emit_rule_body(
    plan: &mut CompiledPlan,
    agent: AgentId,
    link_kind: LinkKind,
    dest: PathBuf,
    body: &RuleBody,
) {
    match body {
        RuleBody::Inline(text) => plan.push(FsOp::WriteFile {
            path: dest,
            overwrite: false,
            content: text.clone(),
        }),
        RuleBody::Source(src) => plan.push(FsOp::Link(PlannedLink {
            agent,
            kind: link_kind,
            source: src.clone(),
            dest,
        })),
    }
}

fn emit_skill_commands(skills: &[crate::tree::SkillNode], ctx: &CompileContext, plan: &mut CompiledPlan) {
    let cursor_dir = ctx.project_path.join(".cursor").join("commands");
    let claude_dir = ctx.project_path.join(".claude").join("skills");
    let codex_dir = ctx.project_path.join(".codex").join("skills");
    plan.push(FsOp::MkdirP { path: cursor_dir.clone() });
    plan.push(FsOp::MkdirP { path: claude_dir.clone() });
    plan.push(FsOp::MkdirP { path: codex_dir.clone() });

    for s in skills {
        match &s.body {
            SkillBody::Inline(md) => {
                plan.push(FsOp::WriteFile {
                    path: cursor_dir.join(format!("{}.md", s.name)),
                    overwrite: false,
                    content: md.clone(),
                });
                plan.push(FsOp::WriteFile {
                    path: claude_dir.join(&s.name).join("SKILL.md"),
                    overwrite: false,
                    content: md.clone(),
                });
                plan.push(FsOp::WriteFile {
                    path: codex_dir.join(&s.name).join("SKILL.md"),
                    overwrite: false,
                    content: md.clone(),
                });
            }
            SkillBody::Source(dir) => {
                let skill_md = dir.join("SKILL.md");
                plan.push(FsOp::Link(PlannedLink {
                    agent: AgentId::Cursor,
                    kind: LinkKind::Symlink,
                    source: skill_md.clone(),
                    dest: cursor_dir.join(format!("{}.md", s.name)),
                }));
                plan.push(FsOp::Link(PlannedLink {
                    agent: AgentId::ClaudeCode,
                    kind: LinkKind::Symlink,
                    source: dir.clone(),
                    dest: claude_dir.join(&s.name),
                }));
                plan.push(FsOp::Link(PlannedLink {
                    agent: AgentId::Codex,
                    kind: LinkKind::Symlink,
                    source: dir.clone(),
                    dest: codex_dir.join(&s.name),
                }));
            }
        }
    }
}

fn emit_settings(settings: &[crate::tree::SettingsNode], ctx: &CompileContext, plan: &mut CompiledPlan) {
    for s in settings {
        let dest = match s.agent {
            AgentId::Cursor => ctx.project_path.join(".cursor").join(&s.file_name),
            AgentId::ClaudeCode => ctx.project_path.join(".claude").join(&s.file_name),
            AgentId::Codex => ctx.project_path.join(".codex").join(&s.file_name),
            AgentId::OpenCode => ctx.project_path.join(".opencode").join(&s.file_name),
            AgentId::Gemini => ctx.project_path.join(".gemini").join(&s.file_name),
            AgentId::Factory => ctx.project_path.join(".factory").join(&s.file_name),
            AgentId::Github => ctx.project_path.join(".github").join(&s.file_name),
            AgentId::Ampcode => ctx.project_path.join(".ampcode").join(&s.file_name),
        };
        if let Some(parent) = dest.parent() {
            plan.push(FsOp::MkdirP { path: parent.to_path_buf() });
        }
        match &s.body {
            SettingsBody::Empty => plan.push(FsOp::WriteFile {
                path: dest,
                overwrite: false,
                content: "{}\n".into(),
            }),
            SettingsBody::Inline(text) => plan.push(FsOp::WriteFile {
                path: dest,
                overwrite: false,
                content: text.clone(),
            }),
            SettingsBody::Source(src) => plan.push(FsOp::Link(PlannedLink {
                agent: s.agent,
                kind: LinkKind::HardLink,
                source: src.clone(),
                dest,
            })),
        }
    }
}

fn emit_mcp(mcp_json: &serde_json::Value, ctx: &CompileContext, plan: &mut CompiledPlan) {
    let rendered = serde_json::to_string_pretty(mcp_json).unwrap_or_else(|_| "{}".into());
    plan.push(FsOp::WriteFile {
        path: ctx.project_path.join(".mcp.json"),
        overwrite: false,
        content: rendered.clone() + "\n",
    });
    plan.push(FsOp::WriteFile {
        path: ctx.project_path.join(".cursor").join("mcp.json"),
        overwrite: false,
        content: rendered + "\n",
    });
}

impl AsRef<Path> for CompileContext {
    fn as_ref(&self) -> &Path {
        &self.project_path
    }
}
