//! Pure compilation pass: `(AgentsTree, CompileContext) → CompiledPlan`.
//!
//! `CompiledPlan` is a list of [`FsOp`] values describing the exact filesystem operations an IO
//! layer should perform. This module itself never touches disk.
//!
//! ## Data-driven + dialect-driven emission
//!
//! Every per-agent behaviour — config directory, rule filename rewrites, Claude's settings scope
//! precedence, Cursor's `.mdc` vs Claude's `.md`, Codex's single-file `AGENTS.md`, etc. — lives
//! on a [`Dialect`](crate::dialect::Dialect) implementation in [`crate::dialects`]. This module
//! is the orchestrator: it walks the tree, merges profile inheritance, and dispatches each leaf
//! to every enabled dialect via `emit_*`. No `.cursor` / `.claude` string literals live here.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;

use crate::dialect::Dialect;
use crate::dialects;
use crate::id::{ProfileId, ProjectKey};
use crate::model::{AgentId, IgnoreKind, PlannedLink, SettingsScope};
use crate::tree::{
    AgentNode, AgentsTree, HookBinding, RuleNode, ScopeKind, SettingsNode, SkillNode, SCOPE_GLOBAL,
};

/// Context the compiler uses to resolve destinations relative to a project root.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CompileContext {
    pub project_path: PathBuf,
    /// Project key used for Cursor rule scope prefixes and Codex `AGENTS.md` selection.
    pub project_key: ProjectKey,
    /// When true, the compiler emits [`crate::model::LinkKind::Copy`] instead of hard/symbolic
    /// links for rule materialisation. Useful for dry-runs and filesystems that don't support
    /// hard links.
    #[serde(default)]
    pub force_copy_for_rules: bool,
    /// Restrict emission to a subset of agents. `None` (default) means "every registered
    /// dialect".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled_agents: Option<Vec<AgentId>>,
}

impl CompileContext {
    #[must_use]
    pub fn new(project_path: impl Into<PathBuf>, project_key: impl Into<ProjectKey>) -> Self {
        Self {
            project_path: project_path.into(),
            project_key: project_key.into(),
            force_copy_for_rules: false,
            enabled_agents: None,
        }
    }

    /// Narrow the plan to `agents` only. Non-listed agents are skipped entirely during emission.
    #[must_use]
    pub fn with_agents(mut self, agents: impl IntoIterator<Item = AgentId>) -> Self {
        self.enabled_agents = Some(agents.into_iter().collect());
        self
    }

    fn agent_enabled(&self, a: AgentId) -> bool {
        match &self.enabled_agents {
            Some(list) => list.contains(&a),
            None => true,
        }
    }
}

/// Every dialect enabled in the current [`CompileContext`], in registry order.
fn enabled_dialects<'c>(
    ctx: &'c CompileContext,
) -> impl Iterator<Item = &'static dyn Dialect> + 'c {
    dialects::all()
        .iter()
        .copied()
        .filter(move |d| ctx.agent_enabled(d.agent()))
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
    #[error("template render failure in {where_}: {message}")]
    TemplateRender { where_: String, message: String },
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
    compile_node(tree, ctx, SCOPE_GLOBAL, &registry, &mut plan)?;
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
    for id in raw.keys() {
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
            | AgentsTree::Agents(_)
            | AgentsTree::Settings(_)
            | AgentsTree::Hooks(_)
            | AgentsTree::Ignore { .. }
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
pub(crate) fn merge_scope_children(left: &[AgentsTree], right: &[AgentsTree]) -> Vec<AgentsTree> {
    #[derive(Default)]
    struct Acc {
        rules: HashMap<String, RuleNode>,
        skills: HashMap<String, SkillNode>,
        agents: HashMap<String, AgentNode>,
        settings: HashMap<(AgentId, SettingsScope, Option<String>), SettingsNode>,
        hooks: Vec<HookBinding>,
        ignores: HashMap<(AgentId, IgnoreKind), Vec<String>>,
        mcp_layers: Vec<Value>,
        text: HashMap<String, String>,
    }

    fn ingest(acc: &mut Acc, node: &AgentsTree) {
        match node {
            AgentsTree::Rules(rs) => {
                for r in rs {
                    acc.rules.insert(r.name.clone(), r.clone());
                }
            }
            AgentsTree::Skills(ss) => {
                for s in ss {
                    acc.skills.insert(s.name.clone(), s.clone());
                }
            }
            AgentsTree::Agents(ags) => {
                for a in ags {
                    acc.agents.insert(a.name.clone(), a.clone());
                }
            }
            AgentsTree::Settings(st) => {
                for s in st {
                    acc.settings
                        .insert((s.agent, s.scope, s.file_name.clone()), s.clone());
                }
            }
            AgentsTree::Hooks(hs) => {
                acc.hooks.extend(hs.iter().cloned());
            }
            AgentsTree::Ignore {
                agent,
                kind,
                patterns,
            } => {
                let entry = acc.ignores.entry((*agent, *kind)).or_default();
                for p in patterns {
                    if !entry.contains(p) {
                        entry.push(p.clone());
                    }
                }
            }
            AgentsTree::Mcp(v) => acc.mcp_layers.push(v.clone()),
            AgentsTree::TextFile { name, body } => {
                acc.text.insert(name.clone(), body.clone());
            }
            AgentsTree::Scope { .. } | AgentsTree::ProfileDef { .. } => {}
        }
    }

    let mut acc = Acc::default();
    for n in left {
        ingest(&mut acc, n);
    }
    for n in right {
        ingest(&mut acc, n);
    }

    let mut out: Vec<AgentsTree> = Vec::new();

    let mut rule_names: Vec<_> = acc.rules.keys().cloned().collect();
    rule_names.sort();
    if !rule_names.is_empty() {
        out.push(AgentsTree::Rules(
            rule_names
                .into_iter()
                .map(|k| acc.rules.remove(&k).unwrap())
                .collect(),
        ));
    }

    let mut skill_names: Vec<_> = acc.skills.keys().cloned().collect();
    skill_names.sort();
    if !skill_names.is_empty() {
        out.push(AgentsTree::Skills(
            skill_names
                .into_iter()
                .map(|k| acc.skills.remove(&k).unwrap())
                .collect(),
        ));
    }

    let mut agent_names: Vec<_> = acc.agents.keys().cloned().collect();
    agent_names.sort();
    if !agent_names.is_empty() {
        out.push(AgentsTree::Agents(
            agent_names
                .into_iter()
                .map(|k| acc.agents.remove(&k).unwrap())
                .collect(),
        ));
    }

    let mut setting_keys: Vec<_> = acc.settings.keys().cloned().collect();
    setting_keys.sort();
    if !setting_keys.is_empty() {
        out.push(AgentsTree::Settings(
            setting_keys
                .into_iter()
                .map(|k| acc.settings.remove(&k).unwrap())
                .collect(),
        ));
    }

    if !acc.hooks.is_empty() {
        out.push(AgentsTree::Hooks(acc.hooks));
    }

    let mut ignore_keys: Vec<_> = acc.ignores.keys().cloned().collect();
    ignore_keys.sort();
    for k in ignore_keys {
        let patterns = acc.ignores.remove(&k).unwrap_or_default();
        out.push(AgentsTree::Ignore {
            agent: k.0,
            kind: k.1,
            patterns,
        });
    }

    if !acc.mcp_layers.is_empty() {
        let merged_mcp = acc
            .mcp_layers
            .into_iter()
            .reduce(|a, b| merge_json_objects(&a, &b))
            .unwrap_or_else(|| Value::Object(Map::new()));
        out.push(AgentsTree::Mcp(merged_mcp));
    }

    let mut text_names: Vec<_> = acc.text.keys().cloned().collect();
    text_names.sort();
    for name in text_names {
        let body = acc.text.remove(&name).unwrap();
        out.push(AgentsTree::TextFile { name, body });
    }

    out
}

pub(crate) fn merge_json_objects(a: &Value, b: &Value) -> Value {
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
        AgentsTree::ProfileDef { .. } => {}
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
            for d in enabled_dialects(ctx) {
                plan.extend(d.emit_rules(ctx, current_scope, rules));
            }
        }
        AgentsTree::Skills(skills) => {
            for d in enabled_dialects(ctx) {
                plan.extend(d.emit_skills(ctx, skills));
            }
        }
        AgentsTree::Agents(agents) => {
            for d in enabled_dialects(ctx) {
                plan.extend(d.emit_agents(ctx, agents));
            }
        }
        AgentsTree::Settings(settings) => {
            for d in enabled_dialects(ctx) {
                plan.extend(d.emit_settings(ctx, settings));
            }
        }
        AgentsTree::Hooks(hooks) => {
            for d in enabled_dialects(ctx) {
                plan.extend(d.emit_hooks(ctx, hooks));
            }
        }
        AgentsTree::Ignore {
            agent,
            kind,
            patterns,
        } => {
            if ctx.agent_enabled(*agent) {
                let d = dialects::get(*agent);
                let emitted = d.emit_ignore(ctx, *kind, patterns);
                if emitted.is_empty() && agent.spec().ignore_filename(*kind).is_none() {
                    plan.warnings.push(format!(
                        "agent `{}` has no ignore file for kind `{:?}`; skipping",
                        agent.as_str(),
                        kind
                    ));
                } else {
                    plan.extend(emitted);
                }
            } else {
                plan.warnings.push(format!(
                    "ignore leaf for disabled agent `{}` dropped",
                    agent.as_str()
                ));
            }
        }
        AgentsTree::Mcp(mcp_json) => {
            for d in enabled_dialects(ctx) {
                plan.extend(d.emit_mcp(ctx, mcp_json));
            }
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

impl AsRef<Path> for CompileContext {
    fn as_ref(&self) -> &Path {
        &self.project_path
    }
}
