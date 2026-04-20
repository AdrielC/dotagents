//! **Templating** — a pure pre-emit pass that expands minijinja templates in string-valued tree
//! nodes, with a data-driven [`RenderContext`].
//!
//! Why templating belongs here: agent rules, skill bodies, hook commands, subagent system
//! prompts, settings blobs — all of them are strings the user writes once and wants parameterised
//! by project name, workstream id, environment detection, repo metadata, etc. Without templating
//! users end up copy-pasting near-identical files per-project; with it they stay DRY.
//!
//! Why this module: we're not pulling minijinja inline into every emit path — that would couple
//! every `Dialect` impl to templating. Instead, [`render_tree`] walks an [`AgentsTree`] and
//! returns a new tree with every string body rendered. The compile pipeline runs this once up
//! front, then dispatches the result to dialects that see only plain strings.
//!
//! ## Template markers
//!
//! A string body is treated as a template **only** if it contains either `{{` or `{%`. Plain
//! strings pass through untouched — no surprise re-renders of `---` fences or URL fragments. You
//! can opt out even then with a [`RenderOptions::force_literal`] body prefix (default: no prefix,
//! so this is an explicit caller choice).

use std::collections::BTreeMap;

use minijinja::{Environment, Value as MjValue};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::tree::{
    AgentBody, AgentNode, AgentsTree, HookBinding, HookHandler, RuleBody, RuleNode, SettingsBody,
    SettingsNode, SkillBody, SkillNode,
};

/// The bag of variables a template can reference via `{{ project.key }}`, `{{ workstream.slug }}`,
/// `{{ env.CI }}`, etc. Pure data; serializable for `agentz.toml` round-tripping.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RenderContext {
    /// Arbitrary user-supplied variables, merged as the outermost scope under `{{ vars.* }}`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub vars: BTreeMap<String, Value>,
    /// Project metadata. Populated from [`crate::compile::CompileContext`] at render time.
    #[serde(default)]
    pub project: ProjectVars,
    /// Workstream metadata (empty if not in a workstream scope).
    #[serde(default)]
    pub workstream: WorkstreamVars,
    /// Environment-variable pass-through. Keys prefixed `AGENTZ_` by the convention in
    /// [`agentz::env`](../agentz/env/index.html); filled by the caller, not read here.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ProjectVars {
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub path: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkstreamVars {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub slug: String,
    #[serde(default)]
    pub kind: String,
}

/// Per-call render options. Defaults render every auto-detected templated body; you can disable
/// templating selectively by setting [`Self::enabled`] to `false`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderOptions {
    pub enabled: bool,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, Error)]
pub enum RenderError {
    #[error("render of `{where_}` failed: {message}")]
    Render { where_: String, message: String },
}

/// Apply templating to every string body in a tree. Returns a new tree; original is untouched.
pub fn render_tree(
    tree: &AgentsTree,
    ctx: &RenderContext,
    opts: &RenderOptions,
) -> Result<AgentsTree, RenderError> {
    if !opts.enabled {
        return Ok(tree.clone());
    }
    let env = build_env();
    render_node(tree, &env, ctx)
}

fn build_env<'a>() -> Environment<'a> {
    let mut env = Environment::new();
    // Strict undefined so typos blow up visibly.
    env.set_undefined_behavior(minijinja::UndefinedBehavior::Strict);
    env
}

fn context_value(ctx: &RenderContext) -> MjValue {
    MjValue::from_serialize(ctx)
}

fn looks_templated(s: &str) -> bool {
    s.contains("{{") || s.contains("{%")
}

fn render_str(
    env: &Environment<'_>,
    ctx: &RenderContext,
    where_: &str,
    s: &str,
) -> Result<String, RenderError> {
    if !looks_templated(s) {
        return Ok(s.to_string());
    }
    let tmpl = env.template_from_str(s).map_err(|e| RenderError::Render {
        where_: where_.into(),
        message: e.to_string(),
    })?;
    tmpl.render(context_value(ctx))
        .map_err(|e| RenderError::Render {
            where_: where_.into(),
            message: e.to_string(),
        })
}

fn render_node(
    node: &AgentsTree,
    env: &Environment<'_>,
    ctx: &RenderContext,
) -> Result<AgentsTree, RenderError> {
    Ok(match node {
        AgentsTree::Scope { kind, children } => AgentsTree::Scope {
            kind: kind.clone(),
            children: children
                .iter()
                .map(|c| render_node(c, env, ctx))
                .collect::<Result<_, _>>()?,
        },
        AgentsTree::ProfileDef {
            id,
            extends,
            children,
        } => AgentsTree::ProfileDef {
            id: id.clone(),
            extends: extends.clone(),
            children: children
                .iter()
                .map(|c| render_node(c, env, ctx))
                .collect::<Result<_, _>>()?,
        },
        AgentsTree::Rules(rs) => AgentsTree::Rules(
            rs.iter()
                .map(|r| render_rule(r, env, ctx))
                .collect::<Result<_, _>>()?,
        ),
        AgentsTree::Skills(ss) => AgentsTree::Skills(
            ss.iter()
                .map(|s| render_skill(s, env, ctx))
                .collect::<Result<_, _>>()?,
        ),
        AgentsTree::Agents(ags) => AgentsTree::Agents(
            ags.iter()
                .map(|a| render_agent(a, env, ctx))
                .collect::<Result<_, _>>()?,
        ),
        AgentsTree::Settings(st) => AgentsTree::Settings(
            st.iter()
                .map(|s| render_settings(s, env, ctx))
                .collect::<Result<_, _>>()?,
        ),
        AgentsTree::Hooks(hs) => AgentsTree::Hooks(
            hs.iter()
                .map(|h| render_hook(h, env, ctx))
                .collect::<Result<_, _>>()?,
        ),
        AgentsTree::Ignore {
            agent,
            kind,
            patterns,
        } => AgentsTree::Ignore {
            agent: *agent,
            kind: *kind,
            patterns: patterns
                .iter()
                .enumerate()
                .map(|(i, p)| render_str(env, ctx, &format!("ignore[{i}]"), p))
                .collect::<Result<_, _>>()?,
        },
        AgentsTree::Mcp(v) => AgentsTree::Mcp(v.clone()),
        AgentsTree::TextFile { name, body } => AgentsTree::TextFile {
            name: name.clone(),
            body: render_str(env, ctx, &format!("text:{name}"), body)?,
        },
    })
}

fn render_rule(
    r: &RuleNode,
    env: &Environment<'_>,
    ctx: &RenderContext,
) -> Result<RuleNode, RenderError> {
    Ok(RuleNode {
        name: r.name.clone(),
        body: match &r.body {
            RuleBody::Inline(t) => {
                RuleBody::Inline(render_str(env, ctx, &format!("rule:{}", r.name), t)?)
            }
            other => other.clone(),
        },
    })
}

fn render_skill(
    s: &SkillNode,
    env: &Environment<'_>,
    ctx: &RenderContext,
) -> Result<SkillNode, RenderError> {
    Ok(SkillNode {
        name: s.name.clone(),
        body: match &s.body {
            SkillBody::Inline(t) => {
                SkillBody::Inline(render_str(env, ctx, &format!("skill:{}", s.name), t)?)
            }
            other => other.clone(),
        },
    })
}

fn render_agent(
    a: &AgentNode,
    env: &Environment<'_>,
    ctx: &RenderContext,
) -> Result<AgentNode, RenderError> {
    Ok(AgentNode {
        name: a.name.clone(),
        body: match &a.body {
            AgentBody::Inline(t) => {
                AgentBody::Inline(render_str(env, ctx, &format!("agent:{}", a.name), t)?)
            }
            other => other.clone(),
        },
    })
}

fn render_settings(
    s: &SettingsNode,
    env: &Environment<'_>,
    ctx: &RenderContext,
) -> Result<SettingsNode, RenderError> {
    Ok(SettingsNode {
        agent: s.agent,
        scope: s.scope,
        file_name: s.file_name.clone(),
        body: match &s.body {
            SettingsBody::Inline(t) => SettingsBody::Inline(render_str(
                env,
                ctx,
                &format!("settings:{:?}:{:?}", s.agent, s.scope),
                t,
            )?),
            other => other.clone(),
        },
    })
}

fn render_hook(
    h: &HookBinding,
    env: &Environment<'_>,
    ctx: &RenderContext,
) -> Result<HookBinding, RenderError> {
    let matcher = render_str(env, ctx, "hook.matcher", &h.matcher)?;
    let handler = match &h.handler {
        HookHandler::Command {
            command,
            args,
            shell,
            timeout_secs,
        } => HookHandler::Command {
            command: render_str(env, ctx, "hook.command", command)?,
            args: args
                .iter()
                .enumerate()
                .map(|(i, a)| render_str(env, ctx, &format!("hook.args[{i}]"), a))
                .collect::<Result<_, _>>()?,
            shell: shell.clone(),
            timeout_secs: *timeout_secs,
        },
        HookHandler::Http {
            url,
            headers,
            timeout_secs,
        } => HookHandler::Http {
            url: render_str(env, ctx, "hook.url", url)?,
            headers: headers.clone(),
            timeout_secs: *timeout_secs,
        },
        HookHandler::Prompt {
            prompt,
            model,
            timeout_secs,
        } => HookHandler::Prompt {
            prompt: render_str(env, ctx, "hook.prompt", prompt)?,
            model: model.clone(),
            timeout_secs: *timeout_secs,
        },
        HookHandler::Agent {
            prompt,
            timeout_secs,
        } => HookHandler::Agent {
            prompt: render_str(env, ctx, "hook.agent.prompt", prompt)?,
            timeout_secs: *timeout_secs,
        },
    };
    Ok(HookBinding {
        event: h.event,
        matcher,
        handler,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::{AgentsTree, RuleBody, RuleNode};

    #[test]
    fn plain_strings_pass_through() {
        let tree = AgentsTree::global([AgentsTree::Rules(vec![RuleNode {
            name: "plain.md".into(),
            body: RuleBody::Inline("no templates here".into()),
        }])]);
        let ctx = RenderContext::default();
        let rendered = render_tree(&tree, &ctx, &RenderOptions::default()).unwrap();
        assert_eq!(tree, rendered);
    }

    #[test]
    fn template_expands_project_key() {
        let tree = AgentsTree::global([AgentsTree::Rules(vec![RuleNode {
            name: "tmpl.md".into(),
            body: RuleBody::Inline("project: {{ project.key }}".into()),
        }])]);
        let ctx = RenderContext {
            project: ProjectVars {
                key: "demo".into(),
                path: "/tmp/demo".into(),
            },
            ..Default::default()
        };
        let rendered = render_tree(&tree, &ctx, &RenderOptions::default()).unwrap();
        let AgentsTree::Scope { children, .. } = &rendered else {
            panic!()
        };
        let AgentsTree::Rules(rs) = &children[0] else {
            panic!()
        };
        let RuleBody::Inline(body) = &rs[0].body else {
            panic!()
        };
        assert_eq!(body, "project: demo");
    }

    #[test]
    fn undefined_variable_errors() {
        let tree = AgentsTree::global([AgentsTree::Rules(vec![RuleNode {
            name: "missing.md".into(),
            body: RuleBody::Inline("{{ nope.deeply.missing }}".into()),
        }])]);
        let ctx = RenderContext::default();
        let err = render_tree(&tree, &ctx, &RenderOptions::default()).unwrap_err();
        let RenderError::Render { where_, .. } = err;
        assert!(where_.starts_with("rule:missing.md"));
    }

    #[test]
    fn vars_bag_is_accessible() {
        let mut vars = BTreeMap::new();
        vars.insert("greeting".into(), Value::String("hi".into()));
        let tree = AgentsTree::global([AgentsTree::Rules(vec![RuleNode {
            name: "v.md".into(),
            body: RuleBody::Inline("{{ vars.greeting }}".into()),
        }])]);
        let ctx = RenderContext {
            vars,
            ..Default::default()
        };
        let rendered = render_tree(&tree, &ctx, &RenderOptions::default()).unwrap();
        let AgentsTree::Scope { children, .. } = &rendered else {
            panic!()
        };
        let AgentsTree::Rules(rs) = &children[0] else {
            panic!()
        };
        let RuleBody::Inline(body) = &rs[0].body else {
            panic!()
        };
        assert_eq!(body, "hi");
    }
}
