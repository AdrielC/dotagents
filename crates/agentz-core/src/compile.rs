//! Pure compilation pass: `(AgentsTree, CompileContext) → CompiledPlan`.
//!
//! `CompiledPlan` is a list of [`FsOp`] values describing the exact filesystem operations an IO
//! layer should perform. This module itself never touches disk.
//!
//! ## Profiles
//!
//! [`crate::tree::AgentsTree::ProfileDef`] registers a reusable bundle (`extends` + children).
//! [`crate::tree::ScopeKind::Profile`] scopes merge that bundle (base → tip, then local children)
//! and emit with the `profile--{id}` rule prefix.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;

use crate::id::{ProfileId, ProjectKey};
use crate::model::{cursor_display_name, AgentId, LinkKind, PlannedLink};
use crate::tree::{AgentsTree, RuleBody, ScopeKind, SettingsBody, SkillBody};

/// Context the compiler uses to resolve destinations relative to a project root.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CompileContext {
    pub project_path: PathBuf,
    /// Project key used for Cursor rule scope prefixes and Codex `AGENTS.md` selection.
    pub project_key: ProjectKey,
    /// When true, the compiler will emit copies instead of hard links for the rule rewrite.
    /// Useful for dry-runs and for filesystems that don't support hard links.
    #[serde(default)]
    pub force_copy_for_rules: bool,
}

impl CompileContext {
    #[must_use]
    pub fn new(project_path: impl Into<PathBuf>, project_key: impl Into<ProjectKey>) -> Self {
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
    #[error("duplicate profile id: {0}")]
    DuplicateProfileId(ProfileId),
    #[error("unknown profile id: {0}")]
    ProfileNotFound(ProfileId),
    #[error("cyclic profile inheritance involving {0}")]
    ProfileCycle(ProfileId),
    #[error("profile {0} contains nested scopes; use only rules/skills/settings/mcp/text leaves")]
    InvalidProfileChildren(ProfileId),
}

/// Map of [`ProfileId`] → merged leaf children (after resolving `extends`).
pub type ProfileRegistry = HashMap<ProfileId, Vec<AgentsTree>>;

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
    let registry = build_profile_registry(tree)?;
    let mut plan = CompiledPlan::default();
    compile_node(tree, ctx, "global", &registry, &mut plan)?;
    dedup_ops(&mut plan);
    Ok(plan)
}

/// Collapse identical operations emitted by overlapping scopes (e.g. nested rules bundles
/// requesting the same `MkdirP`). Conflicting `WriteFile`s at the same path get a warning; the
/// first winning content is kept so compilation remains deterministic.
fn dedup_ops(plan: &mut CompiledPlan) {
    let mut seen_mkdir: HashSet<PathBuf> = HashSet::new();
    let mut seen_write: HashMap<PathBuf, String> = HashMap::new();
    let mut seen_link: HashSet<(PathBuf, PathBuf)> = HashSet::new();
    let mut conflicts: Vec<PathBuf> = Vec::new();

    plan.ops.retain(|op| match op {
        FsOp::MkdirP { path } => seen_mkdir.insert(path.clone()),
        FsOp::WriteFile { path, content, .. } => match seen_write.get(path) {
            Some(prev) if prev == content => false,
            Some(_) => {
                conflicts.push(path.clone());
                false
            }
            None => {
                seen_write.insert(path.clone(), content.clone());
                true
            }
        },
        FsOp::Link(l) => seen_link.insert((l.source.clone(), l.dest.clone())),
    });

    for path in conflicts {
        plan.warnings.push(format!(
            "conflicting WriteFile content for {}; keeping first emission",
            path.display()
        ));
    }
}

fn build_profile_registry(tree: &AgentsTree) -> Result<ProfileRegistry, CompileError> {
    let mut raw: HashMap<ProfileId, (Vec<ProfileId>, Vec<AgentsTree>)> = HashMap::new();
    collect_profile_defs(tree, &mut raw)?;

    let mut registry = ProfileRegistry::new();
    for (id, (_extends, _children)) in raw.iter() {
        let merged = resolve_profile_merged(id, &raw, &mut HashSet::new())?;
        registry.insert(id.clone(), merged);
    }
    Ok(registry)
}

fn collect_profile_defs(
    node: &AgentsTree,
    out: &mut HashMap<ProfileId, (Vec<ProfileId>, Vec<AgentsTree>)>,
) -> Result<(), CompileError> {
    match node {
        AgentsTree::ProfileDef {
            id,
            extends,
            children,
        } => {
            validate_profile_def_children(id, children)?;
            if out
                .insert(id.clone(), (extends.clone(), children.clone()))
                .is_some()
            {
                return Err(CompileError::DuplicateProfileId(id.clone()));
            }
        }
        AgentsTree::Scope { children, .. } => {
            for c in children {
                collect_profile_defs(c, out)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_profile_def_children(
    id: &ProfileId,
    children: &[AgentsTree],
) -> Result<(), CompileError> {
    for c in children {
        match c {
            AgentsTree::Rules(_)
            | AgentsTree::Skills(_)
            | AgentsTree::Settings(_)
            | AgentsTree::Mcp(_)
            | AgentsTree::TextFile { .. } => {}
            AgentsTree::Scope { .. } | AgentsTree::ProfileDef { .. } => {
                return Err(CompileError::InvalidProfileChildren(id.clone()));
            }
        }
    }
    Ok(())
}

fn resolve_profile_merged(
    id: &ProfileId,
    raw: &HashMap<ProfileId, (Vec<ProfileId>, Vec<AgentsTree>)>,
    visiting: &mut HashSet<ProfileId>,
) -> Result<Vec<AgentsTree>, CompileError> {
    if !visiting.insert(id.clone()) {
        return Err(CompileError::ProfileCycle(id.clone()));
    }
    let (extends, own) = raw
        .get(id)
        .ok_or_else(|| CompileError::ProfileNotFound(id.clone()))?
        .clone();

    let mut merged: Vec<AgentsTree> = Vec::new();
    for base in extends {
        let layer = resolve_profile_merged(&base, raw, visiting)?;
        merged = merge_scope_children(&merged, &layer);
    }
    merged = merge_scope_children(&merged, &own);
    let _ = visiting.remove(id);
    Ok(merged)
}

/// Merge leaf bundles: later `right` overrides earlier `left` by key (rule name, skill name, etc.).
fn merge_scope_children(left: &[AgentsTree], right: &[AgentsTree]) -> Vec<AgentsTree> {
    let mut rules: HashMap<String, crate::tree::RuleNode> = HashMap::new();
    let mut skills: HashMap<String, crate::tree::SkillNode> = HashMap::new();
    let mut settings: HashMap<(AgentId, String), crate::tree::SettingsNode> = HashMap::new();
    let mut mcp_layers: Vec<Value> = Vec::new();
    let mut text: HashMap<String, String> = HashMap::new();

    fn ingest(
        node: &AgentsTree,
        rules: &mut HashMap<String, crate::tree::RuleNode>,
        skills: &mut HashMap<String, crate::tree::SkillNode>,
        settings: &mut HashMap<(AgentId, String), crate::tree::SettingsNode>,
        mcp_layers: &mut Vec<Value>,
        text: &mut HashMap<String, String>,
    ) {
        match node {
            AgentsTree::Rules(rs) => {
                for r in rs {
                    rules.insert(r.name.clone(), r.clone());
                }
            }
            AgentsTree::Skills(ss) => {
                for s in ss {
                    skills.insert(s.name.clone(), s.clone());
                }
            }
            AgentsTree::Settings(st) => {
                for s in st {
                    settings.insert((s.agent, s.file_name.clone()), s.clone());
                }
            }
            AgentsTree::Mcp(v) => mcp_layers.push(v.clone()),
            AgentsTree::TextFile { name, body } => {
                text.insert(name.clone(), body.clone());
            }
            AgentsTree::Scope { .. } | AgentsTree::ProfileDef { .. } => {}
        }
    }

    for n in left {
        ingest(
            n,
            &mut rules,
            &mut skills,
            &mut settings,
            &mut mcp_layers,
            &mut text,
        );
    }
    for n in right {
        ingest(
            n,
            &mut rules,
            &mut skills,
            &mut settings,
            &mut mcp_layers,
            &mut text,
        );
    }

    let mut out: Vec<AgentsTree> = Vec::new();
    let mut rule_names: Vec<_> = rules.keys().cloned().collect();
    rule_names.sort();
    if !rule_names.is_empty() {
        out.push(AgentsTree::Rules(
            rule_names
                .into_iter()
                .map(|k| rules.remove(&k).unwrap())
                .collect(),
        ));
    }
    let mut skill_names: Vec<_> = skills.keys().cloned().collect();
    skill_names.sort();
    if !skill_names.is_empty() {
        out.push(AgentsTree::Skills(
            skill_names
                .into_iter()
                .map(|k| skills.remove(&k).unwrap())
                .collect(),
        ));
    }
    let mut setting_keys: Vec<_> = settings.keys().cloned().collect();
    setting_keys.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()).then_with(|| a.1.cmp(&b.1)));
    if !setting_keys.is_empty() {
        out.push(AgentsTree::Settings(
            setting_keys
                .into_iter()
                .map(|k| settings.remove(&k).unwrap())
                .collect(),
        ));
    }
    if !mcp_layers.is_empty() {
        let merged_mcp = mcp_layers
            .into_iter()
            .reduce(|a, b| merge_json_objects(&a, &b))
            .unwrap_or_else(|| Value::Object(Map::new()));
        out.push(AgentsTree::Mcp(merged_mcp));
    }
    let mut text_names: Vec<_> = text.keys().cloned().collect();
    text_names.sort();
    for name in text_names {
        let body = text.remove(&name).unwrap();
        out.push(AgentsTree::TextFile { name, body });
    }
    out
}

fn merge_json_objects(a: &Value, b: &Value) -> Value {
    match (a, b) {
        (Value::Object(am), Value::Object(bm)) => {
            let mut out = am.clone();
            for (k, v) in bm.iter() {
                match out.get_mut(k) {
                    Some(old) => *old = merge_json_objects(old, v),
                    None => {
                        out.insert(k.clone(), v.clone());
                    }
                }
            }
            Value::Object(out)
        }
        (_, b) => b.clone(),
    }
}

fn compile_node(
    node: &AgentsTree,
    ctx: &CompileContext,
    current_scope: &str,
    registry: &ProfileRegistry,
    plan: &mut CompiledPlan,
) -> Result<(), CompileError> {
    match node {
        AgentsTree::ProfileDef { .. } => {
            // Registered at build time; emission happens from Profile scopes or extends.
        }
        AgentsTree::Scope { kind, children } => {
            let prefix = kind.rule_prefix();
            if let ScopeKind::Profile { id } = kind {
                let inherited = registry.get(id).cloned().unwrap_or_default();
                let local: Vec<AgentsTree> = children.to_vec();
                let effective = merge_scope_children(&inherited, &local);
                for c in &effective {
                    compile_node(c, ctx, &prefix, registry, plan)?;
                }
            } else {
                for c in children {
                    compile_node(c, ctx, &prefix, registry, plan)?;
                }
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
        if scope != "global" && scope != ctx.project_key.as_str() {
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

fn emit_skill_commands(
    skills: &[crate::tree::SkillNode],
    ctx: &CompileContext,
    plan: &mut CompiledPlan,
) {
    let cursor_dir = ctx.project_path.join(".cursor").join("commands");
    let claude_dir = ctx.project_path.join(".claude").join("skills");
    let codex_dir = ctx.project_path.join(".codex").join("skills");
    plan.push(FsOp::MkdirP {
        path: cursor_dir.clone(),
    });
    plan.push(FsOp::MkdirP {
        path: claude_dir.clone(),
    });
    plan.push(FsOp::MkdirP {
        path: codex_dir.clone(),
    });

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

fn emit_settings(
    settings: &[crate::tree::SettingsNode],
    ctx: &CompileContext,
    plan: &mut CompiledPlan,
) {
    for s in settings {
        let dest = ctx
            .project_path
            .join(s.agent.config_dir())
            .join(&s.file_name);
        if let Some(parent) = dest.parent() {
            plan.push(FsOp::MkdirP {
                path: parent.to_path_buf(),
            });
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
