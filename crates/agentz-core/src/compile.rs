//! Pure compilation pass: `(AgentsTree, CompileContext) → CompiledPlan`.
//!
//! `CompiledPlan` is a list of [`FsOp`] values describing the exact filesystem operations an IO
//! layer should perform. This module itself never touches disk.
//!
//! ## Data-driven emission
//!
//! This module used to host three hand-rolled `RuleBackend` impls (one per agent) that each
//! hardcoded its own directory and filename-rewrite rules. That's gone. Emission walks the single
//! [`crate::model::SPECS`] table and asks each [`AgentSpec`](crate::model::AgentSpec) — via typed
//! policies on [`RulesLayout`](crate::model::RulesLayout), [`SkillsLayout`](crate::model::SkillsLayout),
//! [`SettingsLayout`](crate::model::SettingsLayout), [`McpLayout`](crate::model::McpLayout),
//! [`HooksLayout`](crate::model::HooksLayout), [`IgnoreLayout`](crate::model::IgnoreLayout) — where
//! and how its files land. Adding an agent is one row in `SPECS`; no match arms to edit here.
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
use crate::model::{
    AgentId, AgentSpec, HooksLayout, IgnoreKind, LinkKind, McpLayout, PlannedLink, RulesLayout,
    SettingsLayout, SettingsScope, SkillsLayout, SPECS,
};
use crate::tree::{
    AgentsTree, HookBinding, RuleBody, RuleNode, ScopeKind, SettingsBody, SettingsNode, SkillBody,
    SkillNode, SCOPE_GLOBAL,
};

/// Context the compiler uses to resolve destinations relative to a project root.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CompileContext {
    pub project_path: PathBuf,
    /// Project key used for Cursor rule scope prefixes and Codex `AGENTS.md` selection.
    pub project_key: ProjectKey,
    /// When true, the compiler emits [`LinkKind::Copy`] instead of hard/symbolic links for rule
    /// materialisation. Useful for dry-runs and filesystems that don't support hard links.
    #[serde(default)]
    pub force_copy_for_rules: bool,
    /// Restrict emission to a subset of agents. `None` means "every agent in [`SPECS`]" (current
    /// default). Set to `Some(vec![AgentId::ClaudeCode])` to compile a Claude-only plan.
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

/// Iterate every agent spec enabled in this context, in `SPECS` order.
fn enabled_specs<'c>(ctx: &'c CompileContext) -> impl Iterator<Item = &'static AgentSpec> + 'c {
    SPECS.iter().filter(move |s| ctx.agent_enabled(s.agent))
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
fn merge_scope_children(left: &[AgentsTree], right: &[AgentsTree]) -> Vec<AgentsTree> {
    #[derive(Default)]
    struct Acc {
        rules: HashMap<String, RuleNode>,
        skills: HashMap<String, SkillNode>,
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
        AgentsTree::Rules(rules) => emit_rules(rules, ctx, current_scope, plan),
        AgentsTree::Skills(skills) => emit_skills(skills, ctx, plan),
        AgentsTree::Settings(settings) => emit_settings(settings, ctx, plan),
        AgentsTree::Hooks(hooks) => emit_hooks(hooks, ctx, plan),
        AgentsTree::Ignore {
            agent,
            kind,
            patterns,
        } => emit_ignore(*agent, *kind, patterns, ctx, plan),
        AgentsTree::Mcp(mcp_json) => emit_mcp(mcp_json, ctx, plan),
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

/// Data-driven rule emission. Walks every agent spec with a [`RulesLayout`] and produces the
/// right filesystem ops — directory vs single-file layout, Cursor/Claude filename rewrites, hard
/// vs symbolic links, and scope-filter policy (Codex's `agents.*` picker).
fn emit_rules(rules: &[RuleNode], ctx: &CompileContext, scope: &str, plan: &mut CompiledPlan) {
    for spec in enabled_specs(ctx) {
        let Some(layout) = spec.rules else { continue };
        emit_rules_for(spec, layout, rules, ctx, scope, plan);
    }
}

fn emit_rules_for(
    spec: &AgentSpec,
    layout: RulesLayout,
    rules: &[RuleNode],
    ctx: &CompileContext,
    scope: &str,
    plan: &mut CompiledPlan,
) {
    if let Some(dir) = layout.dir {
        let dest_dir = ctx.project_path.join(spec.config_dir).join(dir);
        plan.push(FsOp::MkdirP {
            path: dest_dir.clone(),
        });

        let link_kind = if ctx.force_copy_for_rules && layout.link_kind == LinkKind::HardLink {
            LinkKind::Copy
        } else {
            layout.link_kind
        };

        for r in rules {
            let rewritten = layout.name_rewrite.apply(&r.name);
            let dest = dest_dir.join(format!("{scope}{sep}{rewritten}", sep = layout.scope_sep));
            emit_rule_body(plan, spec.agent, link_kind, dest, &r.body);
        }
        return;
    }

    if let Some(filename) = layout.single_file {
        // Single-file layouts (e.g. Codex's `AGENTS.md`): one rule wins. Prefer the global and
        // project scopes; select rules whose name matches `single_file_rule_stem` if set.
        if scope != SCOPE_GLOBAL && scope != ctx.project_key.as_str() {
            return;
        }
        let selected = match layout.single_file_rule_stem {
            Some(stem) => rules.iter().find(|r| r.name.starts_with(stem)),
            None => rules.first(),
        };
        let Some(r) = selected else { return };
        let dest = ctx.project_path.join(filename);
        emit_rule_body(plan, spec.agent, layout.link_kind, dest, &r.body);
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

/// Data-driven skill emission. Cursor gets flat-file commands; Claude/Codex get skill folders.
fn emit_skills(skills: &[SkillNode], ctx: &CompileContext, plan: &mut CompiledPlan) {
    for spec in enabled_specs(ctx) {
        match spec.skills {
            SkillsLayout::FlatFile { dir, extension } => {
                let dest_dir = ctx.project_path.join(spec.config_dir).join(dir);
                plan.push(FsOp::MkdirP {
                    path: dest_dir.clone(),
                });
                for s in skills {
                    let file_name = format!("{}.{}", s.name, extension);
                    let dest = dest_dir.join(&file_name);
                    match &s.body {
                        SkillBody::Inline(md) => plan.push(FsOp::WriteFile {
                            path: dest,
                            overwrite: false,
                            content: md.clone(),
                        }),
                        SkillBody::Source(src_dir) => {
                            // Flat-file targets link the manifest (e.g. Claude/Codex SKILL.md or
                            // a flat README.md) so Cursor's command file tracks upstream edits.
                            let manifest = src_dir.join("SKILL.md");
                            plan.push(FsOp::Link(PlannedLink {
                                agent: spec.agent,
                                kind: LinkKind::Symlink,
                                source: manifest,
                                dest,
                            }));
                        }
                    }
                }
            }
            SkillsLayout::Directory { dir, manifest_file } => {
                let dest_dir = ctx.project_path.join(spec.config_dir).join(dir);
                plan.push(FsOp::MkdirP {
                    path: dest_dir.clone(),
                });
                for s in skills {
                    let skill_dir = dest_dir.join(&s.name);
                    match &s.body {
                        SkillBody::Inline(md) => {
                            plan.push(FsOp::WriteFile {
                                path: skill_dir.join(manifest_file),
                                overwrite: false,
                                content: md.clone(),
                            });
                        }
                        SkillBody::Source(src_dir) => {
                            plan.push(FsOp::Link(PlannedLink {
                                agent: spec.agent,
                                kind: LinkKind::Symlink,
                                source: src_dir.clone(),
                                dest: skill_dir,
                            }));
                        }
                    }
                }
            }
            SkillsLayout::None => {}
        }
    }
}

fn emit_settings(settings: &[SettingsNode], ctx: &CompileContext, plan: &mut CompiledPlan) {
    for s in settings {
        if !ctx.agent_enabled(s.agent) {
            continue;
        }
        let spec = s.agent.spec();
        let Some(file_name) = resolve_settings_file_name(&spec.settings, s) else {
            plan.warnings.push(format!(
                "agent `{}` has no settings file for scope `{:?}`; set SettingsNode.file_name to override",
                spec.id, s.scope
            ));
            continue;
        };

        let dest = ctx.project_path.join(spec.config_dir).join(file_name);
        if let Some(parent) = dest.parent() {
            plan.push(FsOp::MkdirP {
                path: parent.to_path_buf(),
            });
        }
        match &s.body {
            SettingsBody::Empty => plan.push(FsOp::WriteFile {
                path: dest,
                overwrite: false,
                content: empty_settings_body(spec),
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

/// Pick the right filename for a settings node: explicit override wins, else the spec's layout.
fn resolve_settings_file_name(layout: &SettingsLayout, s: &SettingsNode) -> Option<String> {
    if let Some(name) = &s.file_name {
        return Some(name.clone());
    }
    match s.scope {
        SettingsScope::Managed => layout.managed.map(str::to_owned),
        SettingsScope::User | SettingsScope::Project => layout.base.map(str::to_owned),
        SettingsScope::Local => layout.local.map(str::to_owned),
    }
}

/// Empty-body content per agent. GitHub's `copilot-instructions.md` is a markdown file, not JSON —
/// checking the file extension is enough to pick the right placeholder.
fn empty_settings_body(spec: &AgentSpec) -> String {
    let is_markdown = spec
        .settings
        .base
        .map(|n| n.ends_with(".md"))
        .unwrap_or(false);
    if is_markdown {
        String::new()
    } else {
        "{}\n".into()
    }
}

/// Emit hooks to every agent that supports them, rewriting into the agent's preferred format:
/// Claude folds them into `settings.json` (or `settings.local.json`); Cursor writes
/// `.cursor/hooks.json`.
fn emit_hooks(hooks: &[HookBinding], ctx: &CompileContext, plan: &mut CompiledPlan) {
    for spec in enabled_specs(ctx) {
        match spec.hooks {
            HooksLayout::StandaloneFile { filename } => {
                let dest = ctx.project_path.join(spec.config_dir).join(filename);
                if let Some(parent) = dest.parent() {
                    plan.push(FsOp::MkdirP {
                        path: parent.to_path_buf(),
                    });
                }
                let rendered = render_hooks_json(hooks);
                plan.push(FsOp::WriteFile {
                    path: dest,
                    overwrite: false,
                    content: rendered,
                });
            }
            HooksLayout::InSettings { key } => {
                // Claude carries hooks inside settings.json. Emit the hooks block as a separate
                // `hooks.emitted.json` sibling that applyers can merge into settings.json; we
                // keep emission pure here and leave the merge to the IO layer.
                let Some(filename) = spec.settings.base else {
                    continue;
                };
                let companion = format!("{filename}.{key}.json");
                let dest = ctx.project_path.join(spec.config_dir).join(&companion);
                if let Some(parent) = dest.parent() {
                    plan.push(FsOp::MkdirP {
                        path: parent.to_path_buf(),
                    });
                }
                let rendered = render_hooks_in_settings(hooks, key);
                plan.push(FsOp::WriteFile {
                    path: dest,
                    overwrite: false,
                    content: rendered,
                });
            }
            HooksLayout::None => {
                // Silently skip — not every agent supports hooks.
            }
        }
    }
}

fn render_hooks_json(hooks: &[HookBinding]) -> String {
    let json = serde_json::json!({
        "version": "1.0",
        "hooks": hooks_by_event(hooks),
    });
    serde_json::to_string_pretty(&json).unwrap_or_else(|_| "{}".into()) + "\n"
}

fn render_hooks_in_settings(hooks: &[HookBinding], key: &str) -> String {
    let json = serde_json::json!({ key: hooks_by_event(hooks) });
    serde_json::to_string_pretty(&json).unwrap_or_else(|_| "{}".into()) + "\n"
}

fn hooks_by_event(hooks: &[HookBinding]) -> Value {
    use std::collections::BTreeMap;
    let mut grouped: BTreeMap<&'static str, Vec<Value>> = BTreeMap::new();
    for h in hooks {
        let entry = serde_json::json!({
            "matcher": h.matcher,
            "hooks": [serde_json::to_value(&h.handler).unwrap_or(Value::Null)],
        });
        grouped.entry(h.event.as_str()).or_default().push(entry);
    }
    serde_json::to_value(&grouped).unwrap_or(Value::Null)
}

fn emit_ignore(
    agent: AgentId,
    kind: IgnoreKind,
    patterns: &[String],
    ctx: &CompileContext,
    plan: &mut CompiledPlan,
) {
    if !ctx.agent_enabled(agent) {
        return;
    }
    let Some(filename) = agent.spec().ignore_filename(kind) else {
        plan.warnings.push(format!(
            "agent `{}` has no ignore file for kind `{:?}`; skipping",
            agent.spec().id,
            kind
        ));
        return;
    };
    let dest = ctx.project_path.join(filename);
    let content = if patterns.is_empty() {
        String::new()
    } else {
        let mut s = patterns.join("\n");
        s.push('\n');
        s
    };
    plan.push(FsOp::WriteFile {
        path: dest,
        overwrite: false,
        content,
    });
}

/// Data-driven MCP fan-out. Every agent that declares an [`McpLayout::project_file`] gets the
/// rendered JSON at its configured path.
fn emit_mcp(mcp_json: &Value, ctx: &CompileContext, plan: &mut CompiledPlan) {
    let rendered = serde_json::to_string_pretty(mcp_json).unwrap_or_else(|_| "{}".into()) + "\n";
    let mut seen: HashSet<PathBuf> = HashSet::new();
    for spec in enabled_specs(ctx) {
        let McpLayout {
            project_file: Some(rel),
        } = spec.mcp
        else {
            continue;
        };
        let dest = ctx.project_path.join(rel);
        if !seen.insert(dest.clone()) {
            continue;
        }
        if let Some(parent) = dest.parent() {
            plan.push(FsOp::MkdirP {
                path: parent.to_path_buf(),
            });
        }
        plan.push(FsOp::WriteFile {
            path: dest,
            overwrite: false,
            content: rendered.clone(),
        });
    }
}

impl AsRef<Path> for CompileContext {
    fn as_ref(&self) -> &Path {
        &self.project_path
    }
}
