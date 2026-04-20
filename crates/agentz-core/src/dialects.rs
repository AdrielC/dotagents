//! Concrete [`Dialect`] impls for every built-in agent. The old `compile.rs` switched on
//! [`AgentSpec`] table rows; this module replaces those switches with one `impl` per agent.
//!
//! Adding a new agent is exactly one thing: append an `AgentId` variant, a short `AgentSpec` row
//! in [`crate::model::SPECS`] (for static metadata only), and one `impl Dialect for …` block
//! here.

use std::io;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::compile::{CompileContext, FsOp};
use crate::dialect::{Dialect, FileSource};
use crate::model::{
    AgentId, AgentSpec, AgentsLayout, HooksLayout, IgnoreKind, LinkKind, McpLayout, PlannedLink,
    RulesLayout, SettingsLayout, SettingsScope, SkillsLayout,
};
use crate::tree::{
    AgentBody, AgentNode, HookBinding, RuleBody, RuleNode, SettingsBody, SettingsNode, SkillBody,
    SkillNode, SCOPE_GLOBAL,
};

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// Registry of every built-in dialect, in [`AgentId::all`] order.
#[must_use]
pub fn all() -> &'static [&'static dyn Dialect] {
    &[
        &CursorOverride,
        &ClaudeDialect,
        &CodexDialect,
        &OpenCodeDialect,
        &GeminiDialect,
        &FactoryDialect,
        &GithubDialect,
        &AmpcodeDialect,
    ]
}

/// Look up a single dialect by [`AgentId`].
#[must_use]
pub fn get(agent: AgentId) -> &'static dyn Dialect {
    *all()
        .iter()
        .find(|d| d.agent() == agent)
        .expect("every AgentId must have a Dialect in `all()`")
}

// ---------------------------------------------------------------------------
// Shared helpers — policy driven purely by `AgentSpec` layout fields.
// ---------------------------------------------------------------------------

fn spec_of(agent: AgentId) -> &'static AgentSpec {
    agent.spec()
}

/// Emit a bundle of rules given an [`AgentSpec`]'s [`RulesLayout`]. Re-usable from every dialect.
fn emit_rules_via_layout(
    spec: &AgentSpec,
    layout: RulesLayout,
    ctx: &CompileContext,
    scope: &str,
    rules: &[RuleNode],
) -> Vec<FsOp> {
    let mut ops = Vec::new();

    if let Some(dir) = layout.dir {
        let dest_dir = ctx.project_path.join(spec.config_dir).join(dir);
        ops.push(FsOp::MkdirP {
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
            ops.push(rule_body_op(spec.agent, link_kind, dest, &r.body));
        }
        return ops;
    }

    if let Some(filename) = layout.single_file {
        if scope != SCOPE_GLOBAL && scope != ctx.project_key.as_str() {
            return ops;
        }
        let selected = match layout.single_file_rule_stem {
            Some(stem) => rules.iter().find(|r| r.name.starts_with(stem)),
            None => rules.first(),
        };
        if let Some(r) = selected {
            let dest = ctx.project_path.join(filename);
            ops.push(rule_body_op(spec.agent, layout.link_kind, dest, &r.body));
        }
    }

    ops
}

fn rule_body_op(agent: AgentId, link_kind: LinkKind, dest: PathBuf, body: &RuleBody) -> FsOp {
    match body {
        RuleBody::Inline(text) => FsOp::WriteFile {
            path: dest,
            overwrite: false,
            content: text.clone(),
        },
        RuleBody::Source(src) => FsOp::Link(PlannedLink {
            agent,
            kind: link_kind,
            source: src.clone(),
            dest,
        }),
    }
}

fn emit_skills_via_layout(
    spec: &AgentSpec,
    ctx: &CompileContext,
    skills: &[SkillNode],
) -> Vec<FsOp> {
    let mut ops = Vec::new();
    match spec.skills {
        SkillsLayout::FlatFile { dir, extension } => {
            let dest_dir = ctx.project_path.join(spec.config_dir).join(dir);
            ops.push(FsOp::MkdirP {
                path: dest_dir.clone(),
            });
            for s in skills {
                let dest = dest_dir.join(format!("{}.{}", s.name, extension));
                match &s.body {
                    SkillBody::Inline(md) => ops.push(FsOp::WriteFile {
                        path: dest,
                        overwrite: false,
                        content: md.clone(),
                    }),
                    SkillBody::Source(src_dir) => ops.push(FsOp::Link(PlannedLink {
                        agent: spec.agent,
                        kind: LinkKind::Symlink,
                        source: src_dir.join("SKILL.md"),
                        dest,
                    })),
                }
            }
        }
        SkillsLayout::Directory { dir, manifest_file } => {
            let dest_dir = ctx.project_path.join(spec.config_dir).join(dir);
            ops.push(FsOp::MkdirP {
                path: dest_dir.clone(),
            });
            for s in skills {
                let skill_dir = dest_dir.join(&s.name);
                match &s.body {
                    SkillBody::Inline(md) => ops.push(FsOp::WriteFile {
                        path: skill_dir.join(manifest_file),
                        overwrite: false,
                        content: md.clone(),
                    }),
                    SkillBody::Source(src_dir) => ops.push(FsOp::Link(PlannedLink {
                        agent: spec.agent,
                        kind: LinkKind::Symlink,
                        source: src_dir.clone(),
                        dest: skill_dir,
                    })),
                }
            }
        }
        SkillsLayout::None => {}
    }
    ops
}

fn emit_agents_via_layout(
    spec: &AgentSpec,
    ctx: &CompileContext,
    agents: &[AgentNode],
) -> Vec<FsOp> {
    let AgentsLayout::FlatFile { dir, extension } = spec.agents else {
        return Vec::new();
    };
    let mut ops = Vec::new();
    let dest_dir = ctx.project_path.join(spec.config_dir).join(dir);
    ops.push(FsOp::MkdirP {
        path: dest_dir.clone(),
    });
    for a in agents {
        let dest = dest_dir.join(format!("{}.{}", a.name, extension));
        match &a.body {
            AgentBody::Inline(text) => ops.push(FsOp::WriteFile {
                path: dest,
                overwrite: false,
                content: text.clone(),
            }),
            AgentBody::Source(src) => ops.push(FsOp::Link(PlannedLink {
                agent: spec.agent,
                kind: LinkKind::Symlink,
                source: src.clone(),
                dest,
            })),
        }
    }
    ops
}

fn emit_settings_via_layout(
    spec: &AgentSpec,
    ctx: &CompileContext,
    settings: &[SettingsNode],
) -> Vec<FsOp> {
    let mut ops = Vec::new();
    for s in settings.iter().filter(|s| s.agent == spec.agent) {
        let Some(file_name) = resolve_settings_file_name(&spec.settings, s) else {
            continue;
        };
        let dest = ctx.project_path.join(spec.config_dir).join(file_name);
        if let Some(parent) = dest.parent() {
            ops.push(FsOp::MkdirP {
                path: parent.to_path_buf(),
            });
        }
        match &s.body {
            SettingsBody::Empty => ops.push(FsOp::WriteFile {
                path: dest,
                overwrite: false,
                content: empty_settings_body(spec),
            }),
            SettingsBody::Inline(text) => ops.push(FsOp::WriteFile {
                path: dest,
                overwrite: false,
                content: text.clone(),
            }),
            SettingsBody::Source(src) => ops.push(FsOp::Link(PlannedLink {
                agent: s.agent,
                kind: LinkKind::HardLink,
                source: src.clone(),
                dest,
            })),
        }
    }
    ops
}

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

fn emit_hooks_via_layout(
    spec: &AgentSpec,
    ctx: &CompileContext,
    hooks: &[HookBinding],
) -> Vec<FsOp> {
    let mut ops = Vec::new();
    match spec.hooks {
        HooksLayout::StandaloneFile { filename } => {
            let dest = ctx.project_path.join(spec.config_dir).join(filename);
            if let Some(parent) = dest.parent() {
                ops.push(FsOp::MkdirP {
                    path: parent.to_path_buf(),
                });
            }
            ops.push(FsOp::WriteFile {
                path: dest,
                overwrite: false,
                content: render_hooks_json(hooks),
            });
        }
        HooksLayout::InSettings { key } => {
            let Some(base) = spec.settings.base else {
                return ops;
            };
            let companion = format!("{base}.{key}.json");
            let dest = ctx.project_path.join(spec.config_dir).join(&companion);
            if let Some(parent) = dest.parent() {
                ops.push(FsOp::MkdirP {
                    path: parent.to_path_buf(),
                });
            }
            ops.push(FsOp::WriteFile {
                path: dest,
                overwrite: false,
                content: render_hooks_in_settings(hooks, key),
            });
        }
        HooksLayout::None => {}
    }
    ops
}

fn emit_ignore_via_layout(
    spec: &AgentSpec,
    ctx: &CompileContext,
    kind: IgnoreKind,
    patterns: &[String],
) -> Vec<FsOp> {
    let Some(filename) = spec.ignore_filename(kind) else {
        return Vec::new();
    };
    let content = if patterns.is_empty() {
        String::new()
    } else {
        let mut s = patterns.join("\n");
        s.push('\n');
        s
    };
    vec![FsOp::WriteFile {
        path: ctx.project_path.join(filename),
        overwrite: false,
        content,
    }]
}

fn emit_mcp_via_layout(spec: &AgentSpec, ctx: &CompileContext, mcp: &Value) -> Vec<FsOp> {
    let McpLayout {
        project_file: Some(rel),
    } = spec.mcp
    else {
        return Vec::new();
    };
    let rendered = serde_json::to_string_pretty(mcp).unwrap_or_else(|_| "{}".into()) + "\n";
    let dest = ctx.project_path.join(rel);
    let mut ops = Vec::new();
    if let Some(parent) = dest.parent() {
        ops.push(FsOp::MkdirP {
            path: parent.to_path_buf(),
        });
    }
    ops.push(FsOp::WriteFile {
        path: dest,
        overwrite: false,
        content: rendered,
    });
    ops
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

// ---------------------------------------------------------------------------
// Shared ingest helpers
// ---------------------------------------------------------------------------

fn file_name_string(path: &Path) -> Option<String> {
    path.file_name().and_then(|s| s.to_str()).map(str::to_owned)
}

fn file_stem_string(path: &Path) -> Option<String> {
    path.file_stem().and_then(|s| s.to_str()).map(str::to_owned)
}

fn ingest_rules_from_dir(fs: &dyn FileSource, dir: &Path) -> io::Result<Vec<RuleNode>> {
    if !fs.is_dir(dir) {
        return Ok(Vec::new());
    }
    let mut rules: Vec<RuleNode> = Vec::new();
    let mut entries = fs.read_dir(dir)?;
    entries.sort();
    for p in entries {
        if !fs.is_file(&p) {
            continue;
        }
        let Some(name) = file_name_string(&p) else {
            continue;
        };
        let body = fs.read_to_string(&p)?;
        rules.push(RuleNode {
            name,
            body: RuleBody::Inline(body),
        });
    }
    Ok(rules)
}

fn ingest_skills_from_dir(
    fs: &dyn FileSource,
    dir: &Path,
    manifest: &str,
) -> io::Result<Vec<SkillNode>> {
    if !fs.is_dir(dir) {
        return Ok(Vec::new());
    }
    let mut out: Vec<SkillNode> = Vec::new();
    let mut entries = fs.read_dir(dir)?;
    entries.sort();
    for p in entries {
        if !fs.is_dir(&p) {
            continue;
        }
        let Some(name) = file_name_string(&p) else {
            continue;
        };
        let m = p.join(manifest);
        if fs.is_file(&m) {
            out.push(SkillNode {
                name,
                body: SkillBody::Inline(fs.read_to_string(&m)?),
            });
        }
    }
    Ok(out)
}

fn ingest_flat_skills_from_dir(
    fs: &dyn FileSource,
    dir: &Path,
    extension: &str,
) -> io::Result<Vec<SkillNode>> {
    if !fs.is_dir(dir) {
        return Ok(Vec::new());
    }
    let mut out: Vec<SkillNode> = Vec::new();
    let mut entries = fs.read_dir(dir)?;
    entries.sort();
    for p in entries {
        if !fs.is_file(&p) {
            continue;
        }
        let matches_ext = p
            .extension()
            .and_then(|s| s.to_str())
            .map(|e| e == extension)
            .unwrap_or(false);
        if !matches_ext {
            continue;
        }
        let Some(stem) = file_stem_string(&p) else {
            continue;
        };
        out.push(SkillNode {
            name: stem,
            body: SkillBody::Inline(fs.read_to_string(&p)?),
        });
    }
    Ok(out)
}

fn ingest_agents_from_dir(
    fs: &dyn FileSource,
    dir: &Path,
    extension: &str,
) -> io::Result<Vec<AgentNode>> {
    if !fs.is_dir(dir) {
        return Ok(Vec::new());
    }
    let mut out: Vec<AgentNode> = Vec::new();
    let mut entries = fs.read_dir(dir)?;
    entries.sort();
    for p in entries {
        if !fs.is_file(&p) {
            continue;
        }
        let matches_ext = p
            .extension()
            .and_then(|s| s.to_str())
            .map(|e| e == extension)
            .unwrap_or(false);
        if !matches_ext {
            continue;
        }
        let Some(stem) = file_stem_string(&p) else {
            continue;
        };
        out.push(AgentNode {
            name: stem,
            body: AgentBody::Inline(fs.read_to_string(&p)?),
        });
    }
    Ok(out)
}

fn ingest_settings_from_layout(
    spec: &AgentSpec,
    fs: &dyn FileSource,
    root: &Path,
) -> io::Result<Vec<SettingsNode>> {
    let mut out = Vec::new();
    let candidates: &[(SettingsScope, Option<&'static str>)] = &[
        (SettingsScope::Project, spec.settings.base),
        (SettingsScope::Local, spec.settings.local),
        (SettingsScope::Managed, spec.settings.managed),
    ];
    for (scope, maybe_name) in candidates {
        let Some(name) = maybe_name else {
            continue;
        };
        let path = root.join(name);
        if fs.is_file(&path) {
            out.push(SettingsNode {
                agent: spec.agent,
                scope: *scope,
                file_name: None,
                body: SettingsBody::Inline(fs.read_to_string(&path)?),
            });
        }
    }
    Ok(out)
}

fn ingest_ignore_from_layout(
    spec: &AgentSpec,
    fs: &dyn FileSource,
    repo_root: &Path,
) -> io::Result<Vec<(IgnoreKind, Vec<String>)>> {
    let mut out = Vec::new();
    for (kind, name) in [
        (IgnoreKind::Primary, spec.ignore.primary),
        (IgnoreKind::Secondary, spec.ignore.secondary),
    ] {
        let Some(filename) = name else {
            continue;
        };
        let path = repo_root.join(filename);
        if !fs.is_file(&path) {
            continue;
        }
        let text = fs.read_to_string(&path)?;
        let patterns: Vec<String> = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(str::to_owned)
            .collect();
        if !patterns.is_empty() {
            out.push((kind, patterns));
        }
    }
    Ok(out)
}

fn ingest_mcp_from_layout(
    spec: &AgentSpec,
    fs: &dyn FileSource,
    repo_root: &Path,
) -> io::Result<Option<Value>> {
    let McpLayout {
        project_file: Some(rel),
    } = spec.mcp
    else {
        return Ok(None);
    };
    let path = repo_root.join(rel);
    if !fs.is_file(&path) {
        return Ok(None);
    }
    let text = fs.read_to_string(&path)?;
    match serde_json::from_str::<Value>(&text) {
        Ok(v) => Ok(Some(v)),
        Err(_) => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// Per-agent structs
// ---------------------------------------------------------------------------

macro_rules! plain_dialect {
    ($ty:ident, $agent:expr) => {
        #[derive(Copy, Clone, Debug, Default)]
        pub struct $ty;
        impl Dialect for $ty {
            fn agent(&self) -> AgentId {
                $agent
            }
            fn emit_rules(
                &self,
                ctx: &CompileContext,
                scope: &str,
                rules: &[RuleNode],
            ) -> Vec<FsOp> {
                let spec = spec_of(self.agent());
                let Some(layout) = spec.rules else {
                    return Vec::new();
                };
                emit_rules_via_layout(spec, layout, ctx, scope, rules)
            }
            fn emit_skills(&self, ctx: &CompileContext, skills: &[SkillNode]) -> Vec<FsOp> {
                emit_skills_via_layout(spec_of(self.agent()), ctx, skills)
            }
            fn emit_agents(&self, ctx: &CompileContext, agents: &[AgentNode]) -> Vec<FsOp> {
                emit_agents_via_layout(spec_of(self.agent()), ctx, agents)
            }
            fn emit_settings(&self, ctx: &CompileContext, settings: &[SettingsNode]) -> Vec<FsOp> {
                emit_settings_via_layout(spec_of(self.agent()), ctx, settings)
            }
            fn emit_hooks(&self, ctx: &CompileContext, hooks: &[HookBinding]) -> Vec<FsOp> {
                emit_hooks_via_layout(spec_of(self.agent()), ctx, hooks)
            }
            fn emit_ignore(
                &self,
                ctx: &CompileContext,
                kind: IgnoreKind,
                patterns: &[String],
            ) -> Vec<FsOp> {
                emit_ignore_via_layout(spec_of(self.agent()), ctx, kind, patterns)
            }
            fn emit_mcp(&self, ctx: &CompileContext, mcp: &Value) -> Vec<FsOp> {
                emit_mcp_via_layout(spec_of(self.agent()), ctx, mcp)
            }
            fn ingest_rules(&self, fs: &dyn FileSource, root: &Path) -> io::Result<Vec<RuleNode>> {
                let spec = spec_of(self.agent());
                let Some(layout) = spec.rules else {
                    return Ok(Vec::new());
                };
                if let Some(dir) = layout.dir {
                    ingest_rules_from_dir(fs, &root.join(dir))
                } else {
                    Ok(Vec::new())
                }
            }
            fn ingest_skills(
                &self,
                fs: &dyn FileSource,
                root: &Path,
            ) -> io::Result<Vec<SkillNode>> {
                let spec = spec_of(self.agent());
                match spec.skills {
                    SkillsLayout::Directory { dir, manifest_file } => {
                        ingest_skills_from_dir(fs, &root.join(dir), manifest_file)
                    }
                    SkillsLayout::FlatFile { dir, extension } => {
                        ingest_flat_skills_from_dir(fs, &root.join(dir), extension)
                    }
                    SkillsLayout::None => Ok(Vec::new()),
                }
            }
            fn ingest_agents(
                &self,
                fs: &dyn FileSource,
                root: &Path,
            ) -> io::Result<Vec<AgentNode>> {
                let spec = spec_of(self.agent());
                let AgentsLayout::FlatFile { dir, extension } = spec.agents else {
                    return Ok(Vec::new());
                };
                ingest_agents_from_dir(fs, &root.join(dir), extension)
            }
            fn ingest_settings(
                &self,
                fs: &dyn FileSource,
                root: &Path,
            ) -> io::Result<Vec<SettingsNode>> {
                ingest_settings_from_layout(spec_of(self.agent()), fs, root)
            }
            fn ingest_ignore(
                &self,
                fs: &dyn FileSource,
                repo_root: &Path,
            ) -> io::Result<Vec<(IgnoreKind, Vec<String>)>> {
                ingest_ignore_from_layout(spec_of(self.agent()), fs, repo_root)
            }
            fn ingest_mcp(
                &self,
                fs: &dyn FileSource,
                repo_root: &Path,
            ) -> io::Result<Option<Value>> {
                ingest_mcp_from_layout(spec_of(self.agent()), fs, repo_root)
            }
        }
    };
}

plain_dialect!(CursorDialect, AgentId::Cursor);
plain_dialect!(ClaudeDialect, AgentId::ClaudeCode);

// ── Cursor-specific overrides ────────────────────────────────────────────────
//
// These correct for format divergences documented at
// <https://cursor.com/docs/agent/hooks> and the general Cursor docs.

impl CursorDialect {
    fn emit_hooks_cursor_shape(&self, ctx: &CompileContext, hooks: &[HookBinding]) -> Vec<FsOp> {
        if hooks.is_empty() {
            return Vec::new();
        }
        let spec = spec_of(self.agent());
        let HooksLayout::StandaloneFile { filename } = spec.hooks else {
            return Vec::new();
        };
        let dest = ctx.project_path.join(spec.config_dir).join(filename);
        let mut ops = Vec::new();
        if let Some(parent) = dest.parent() {
            ops.push(FsOp::MkdirP {
                path: parent.to_path_buf(),
            });
        }
        ops.push(FsOp::WriteFile {
            path: dest,
            overwrite: false,
            content: render_cursor_hooks_json(hooks),
        });
        ops
    }
}

/// Cursor's `.cursor/hooks.json` has three differences from Claude's embedded hooks:
///   - `version` is an integer `1`, not the string `"1.0"`.
///   - Event names are **camelCase** (`preToolUse`, `sessionStart`, …).
///   - `matcher` lives on the **handler** entry, not a wrapping group object.
///
/// Claude-only events with no Cursor equivalent (`PermissionRequest`, `PostCompact`, …) are
/// dropped silently — we don't fabricate meaning for them.
fn render_cursor_hooks_json(hooks: &[HookBinding]) -> String {
    use std::collections::BTreeMap;
    let mut grouped: BTreeMap<&'static str, Vec<Value>> = BTreeMap::new();
    for h in hooks {
        let Some(event_name) = h.event.cursor_name() else {
            continue;
        };
        let entry = cursor_handler_value(h);
        grouped.entry(event_name).or_default().push(entry);
    }
    let payload = serde_json::json!({
        "version": 1,
        "hooks": grouped,
    });
    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".into()) + "\n"
}

fn cursor_handler_value(h: &HookBinding) -> Value {
    use crate::tree::HookHandler;
    let mut obj = serde_json::Map::new();
    if !h.matcher.is_empty() {
        obj.insert("matcher".into(), Value::String(h.matcher.clone()));
    }
    match &h.handler {
        HookHandler::Command {
            command,
            args,
            timeout_secs,
            ..
        } => {
            obj.insert("type".into(), Value::String("command".into()));
            if args.is_empty() {
                obj.insert("command".into(), Value::String(command.clone()));
            } else {
                let joined = std::iter::once(command.clone())
                    .chain(args.iter().cloned())
                    .collect::<Vec<_>>()
                    .join(" ");
                obj.insert("command".into(), Value::String(joined));
            }
            if let Some(t) = timeout_secs {
                obj.insert("timeout".into(), Value::from(*t));
            }
        }
        HookHandler::Prompt {
            prompt,
            timeout_secs,
            ..
        } => {
            obj.insert("type".into(), Value::String("prompt".into()));
            obj.insert("prompt".into(), Value::String(prompt.clone()));
            if let Some(t) = timeout_secs {
                obj.insert("timeout".into(), Value::from(*t));
            }
        }
        HookHandler::Http { url, .. } => {
            // Cursor doesn't document an HTTP hook shape; fall back to a command that POSTs.
            obj.insert("type".into(), Value::String("command".into()));
            obj.insert(
                "command".into(),
                Value::String(format!("curl -fsS -X POST {url}")),
            );
        }
        HookHandler::Agent { prompt, .. } => {
            obj.insert("type".into(), Value::String("prompt".into()));
            obj.insert("prompt".into(), Value::String(prompt.clone()));
        }
    }
    Value::Object(obj)
}

// Actual override goes here — we replace the macro-generated `emit_hooks` with the Cursor shape,
// and suppress `emit_skills` since Cursor has no slash-commands directory analogous to Claude's.
// We use a wrapper type so we don't have to duplicate the huge macro body. Instead, the
// dialects registry in `all()` exposes `&'static CursorOverride`, which forwards everything to
// the plain dialect except the two methods below.

pub struct CursorOverride;
impl Dialect for CursorOverride {
    fn agent(&self) -> AgentId {
        AgentId::Cursor
    }
    fn emit_rules(&self, ctx: &CompileContext, scope: &str, rules: &[RuleNode]) -> Vec<FsOp> {
        CursorDialect.emit_rules(ctx, scope, rules)
    }
    fn emit_skills(&self, _ctx: &CompileContext, _skills: &[SkillNode]) -> Vec<FsOp> {
        // Cursor has no on-disk slash-commands directory that mirrors Claude's
        // `.claude/commands/` — skills stay on Claude/Codex until Cursor ships one.
        Vec::new()
    }
    fn emit_agents(&self, ctx: &CompileContext, agents: &[AgentNode]) -> Vec<FsOp> {
        CursorDialect.emit_agents(ctx, agents)
    }
    fn emit_settings(&self, ctx: &CompileContext, settings: &[SettingsNode]) -> Vec<FsOp> {
        CursorDialect.emit_settings(ctx, settings)
    }
    fn emit_hooks(&self, ctx: &CompileContext, hooks: &[HookBinding]) -> Vec<FsOp> {
        CursorDialect.emit_hooks_cursor_shape(ctx, hooks)
    }
    fn emit_ignore(
        &self,
        ctx: &CompileContext,
        kind: IgnoreKind,
        patterns: &[String],
    ) -> Vec<FsOp> {
        CursorDialect.emit_ignore(ctx, kind, patterns)
    }
    fn emit_mcp(&self, ctx: &CompileContext, mcp: &Value) -> Vec<FsOp> {
        CursorDialect.emit_mcp(ctx, mcp)
    }
    fn ingest_rules(&self, fs: &dyn FileSource, root: &Path) -> io::Result<Vec<RuleNode>> {
        CursorDialect.ingest_rules(fs, root)
    }
    fn ingest_skills(&self, fs: &dyn FileSource, root: &Path) -> io::Result<Vec<SkillNode>> {
        CursorDialect.ingest_skills(fs, root)
    }
    fn ingest_agents(&self, fs: &dyn FileSource, root: &Path) -> io::Result<Vec<AgentNode>> {
        CursorDialect.ingest_agents(fs, root)
    }
    fn ingest_settings(&self, fs: &dyn FileSource, root: &Path) -> io::Result<Vec<SettingsNode>> {
        CursorDialect.ingest_settings(fs, root)
    }
    fn ingest_ignore(
        &self,
        fs: &dyn FileSource,
        repo_root: &Path,
    ) -> io::Result<Vec<(IgnoreKind, Vec<String>)>> {
        CursorDialect.ingest_ignore(fs, repo_root)
    }
    fn ingest_mcp(&self, fs: &dyn FileSource, repo_root: &Path) -> io::Result<Option<Value>> {
        CursorDialect.ingest_mcp(fs, repo_root)
    }
}
plain_dialect!(CodexDialect, AgentId::Codex);
plain_dialect!(OpenCodeDialect, AgentId::OpenCode);
plain_dialect!(GeminiDialect, AgentId::Gemini);
plain_dialect!(FactoryDialect, AgentId::Factory);
plain_dialect!(GithubDialect, AgentId::Github);
plain_dialect!(AmpcodeDialect, AgentId::Ampcode);
