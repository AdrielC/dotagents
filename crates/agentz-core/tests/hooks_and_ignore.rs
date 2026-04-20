//! Coverage for the hooks + ignore-file leaves of [`AgentsTree`].
//!
//! Hooks are routed into Claude's `settings.json` companion file and Cursor's standalone
//! `hooks.json`; the compiler shouldn't require the caller to know either filename.

use std::path::PathBuf;

use agentz_core::compile::{compile, CompileContext, FsOp};
use agentz_core::model::IgnoreKind;
use agentz_core::tree::{AgentsTree, HookBinding, HookHandler};
use agentz_core::AgentId;

fn project() -> PathBuf {
    PathBuf::from("/workspace/demo")
}

#[test]
fn hooks_land_in_cursor_hooks_json_and_claude_companion() {
    let tree = AgentsTree::global([AgentsTree::Hooks(vec![
        HookBinding::pre_tool_use("Bash", HookHandler::command("scripts/validate.sh")),
        HookBinding::post_tool_use("Edit|Write", HookHandler::command("scripts/format.sh")),
    ])]);
    let ctx = CompileContext::new(project(), "demo");
    let plan = compile(&tree, &ctx).unwrap();

    let cursor_hooks = project().join(".cursor/hooks.json");
    let claude_companion = project().join(".claude/settings.json.hooks.json");
    assert!(
        plan.ops
            .iter()
            .any(|op| matches!(op, FsOp::WriteFile { path, .. } if path == &cursor_hooks)),
        "cursor standalone hooks.json written"
    );
    assert!(
        plan.ops
            .iter()
            .any(|op| matches!(op, FsOp::WriteFile { path, .. } if path == &claude_companion)),
        "claude settings companion written"
    );

    let cursor_content = plan
        .ops
        .iter()
        .find_map(|op| match op {
            FsOp::WriteFile { path, content, .. } if path == &cursor_hooks => {
                Some(content.as_str())
            }
            _ => None,
        })
        .unwrap();
    assert!(cursor_content.contains("PreToolUse"));
    assert!(cursor_content.contains("Bash"));
    assert!(cursor_content.contains("scripts/validate.sh"));
}

#[test]
fn ignore_node_writes_primary_and_secondary_for_cursor() {
    let tree = AgentsTree::global([
        AgentsTree::Ignore {
            agent: AgentId::Cursor,
            kind: IgnoreKind::Primary,
            patterns: vec!["node_modules/".into(), ".env".into()],
        },
        AgentsTree::Ignore {
            agent: AgentId::Cursor,
            kind: IgnoreKind::Secondary,
            patterns: vec!["vendor/**".into()],
        },
    ]);
    let ctx = CompileContext::new(project(), "demo");
    let plan = compile(&tree, &ctx).unwrap();

    let primary = project().join(".cursorignore");
    let secondary = project().join(".cursorindexignore");

    let primary_content = plan
        .ops
        .iter()
        .find_map(|op| match op {
            FsOp::WriteFile { path, content, .. } if path == &primary => Some(content.as_str()),
            _ => None,
        })
        .expect("primary ignore");
    assert!(primary_content.contains("node_modules/"));
    assert!(primary_content.contains(".env"));

    let secondary_content = plan
        .ops
        .iter()
        .find_map(|op| match op {
            FsOp::WriteFile { path, content, .. } if path == &secondary => Some(content.as_str()),
            _ => None,
        })
        .expect("secondary ignore");
    assert!(secondary_content.contains("vendor/**"));
}

#[test]
fn claude_ignore_node_writes_claudeignore() {
    let tree = AgentsTree::global([AgentsTree::Ignore {
        agent: AgentId::ClaudeCode,
        kind: IgnoreKind::Primary,
        patterns: vec!["secrets/".into()],
    }]);
    let ctx = CompileContext::new(project(), "demo");
    let plan = compile(&tree, &ctx).unwrap();

    let dest = project().join(".claudeignore");
    let content = plan
        .ops
        .iter()
        .find_map(|op| match op {
            FsOp::WriteFile { path, content, .. } if path == &dest => Some(content.as_str()),
            _ => None,
        })
        .expect("claude ignore");
    assert!(content.contains("secrets/"));
}

#[test]
fn ignore_node_for_agent_without_support_warns() {
    let tree = AgentsTree::global([AgentsTree::Ignore {
        agent: AgentId::Codex,
        kind: IgnoreKind::Primary,
        patterns: vec!["build/".into()],
    }]);
    let ctx = CompileContext::new(project(), "demo");
    let plan = compile(&tree, &ctx).unwrap();

    assert!(
        plan.warnings.iter().any(|w| w.contains("no ignore file")),
        "warning for unsupported agent"
    );
}

#[test]
fn settings_respect_scope_filenames() {
    use agentz_core::tree::{SettingsBody, SettingsNode};

    let tree = AgentsTree::global([AgentsTree::Settings(vec![
        SettingsNode::claude_project(SettingsBody::Inline("{\"shared\": true}\n".into())),
        SettingsNode::claude_local(SettingsBody::Inline("{\"me\": true}\n".into())),
        SettingsNode::claude_managed(SettingsBody::Inline("{\"org\": true}\n".into())),
    ])]);
    let ctx = CompileContext::new(project(), "demo");
    let plan = compile(&tree, &ctx).unwrap();

    let paths: Vec<_> = plan
        .ops
        .iter()
        .filter_map(|op| match op {
            FsOp::WriteFile { path, .. } => Some(path.clone()),
            _ => None,
        })
        .collect();
    assert!(paths.contains(&project().join(".claude/settings.json")));
    assert!(paths.contains(&project().join(".claude/settings.local.json")));
    assert!(paths.contains(&project().join(".claude/managed-settings.json")));
}
