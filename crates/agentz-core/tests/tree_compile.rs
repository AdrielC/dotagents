//! Pure compilation tests. Walks an `AgentsTree` AST and asserts the produced `FsOp` list.
//!
//! **Cursor ↔ Claude:** One [`AgentsTree::Rules`] node is the unified bundle; [`compile`]
//! emits **both** `.cursor/rules/…` (Cursor: `.md`→`.mdc`, hard link or copy) and
//! `.claude/rules/…` (Claude: `…--{base}.md`, symlink) so a single tree is the
//! cross-agent “conversion” without a separate IO step in `agentz-core`.

use std::path::PathBuf;

use agentz_core::compile::{compile, CompileContext, FsOp};
use agentz_core::model::{AgentId, LinkKind};
use agentz_core::ProfileId;
use agentz_core::tree::{AgentsTree, RuleBody, RuleNode, SettingsBody, SettingsNode, SkillBody, SkillNode};

fn project() -> PathBuf {
    PathBuf::from("/workspace/demo")
}

#[test]
fn tree_compiles_rules_into_cursor_and_claude() {
    let tree = AgentsTree::global([
        AgentsTree::Rules(vec![RuleNode {
            name: "010-style.md".into(),
            body: RuleBody::Inline("# style\n".into()),
        }]),
        AgentsTree::Skills(vec![SkillNode {
            name: "reviewer".into(),
            body: SkillBody::Inline("---\nname: reviewer\n---\n".into()),
        }]),
        AgentsTree::Settings(vec![SettingsNode {
            agent: AgentId::Cursor,
            file_name: "settings.json".into(),
            body: SettingsBody::Inline("{\"x\":1}".into()),
        }]),
        AgentsTree::Mcp(serde_json::json!({ "mcpServers": {} })),
    ]);
    let ctx = CompileContext::new(project(), "demo");

    let plan = compile(&tree, &ctx).unwrap();

    // Cursor rule was rewritten .md → .mdc and prefixed with `global--`.
    assert!(plan.ops.iter().any(|op| matches!(op,
        FsOp::WriteFile { path, .. }
            if path == &project().join(".cursor/rules/global--010-style.mdc"))));
    // Claude rule stays .md.
    assert!(plan.ops.iter().any(|op| matches!(op,
        FsOp::WriteFile { path, .. }
            if path == &project().join(".claude/rules/global--010-style.md"))));
    // Skill wrote to all three target families.
    assert!(plan.ops.iter().any(|op| matches!(op,
        FsOp::WriteFile { path, .. }
            if path == &project().join(".cursor/commands/reviewer.md"))));
    assert!(plan.ops.iter().any(|op| matches!(op,
        FsOp::WriteFile { path, .. }
            if path == &project().join(".claude/skills/reviewer/SKILL.md"))));
    // MCP fan-out.
    assert!(plan.ops.iter().any(|op| matches!(op,
        FsOp::WriteFile { path, .. } if path == &project().join(".mcp.json"))));
    assert!(plan.ops.iter().any(|op| matches!(op,
        FsOp::WriteFile { path, .. } if path == &project().join(".cursor/mcp.json"))));
}

/// One rules bundle → Cursor file + Claude file with the **same** scope prefix and body.
#[test]
fn unified_inline_rule_maps_to_cursor_mdc_and_claude_md() {
    let body = "# One source of truth\n";
    let tree = AgentsTree::global([AgentsTree::Rules(vec![RuleNode {
        name: "010-style.md".into(),
        body: RuleBody::Inline(body.into()),
    }])]);
    let ctx = CompileContext::new(project(), "demo");
    let plan = compile(&tree, &ctx).unwrap();

    let cursor_write = plan.ops.iter().find_map(|op| match op {
        FsOp::WriteFile { path, content, .. }
            if path == &project().join(".cursor/rules/global--010-style.mdc") =>
        {
            Some(content.as_str())
        }
        _ => None,
    });
    let claude_write = plan.ops.iter().find_map(|op| match op {
        FsOp::WriteFile { path, content, .. }
            if path == &project().join(".claude/rules/global--010-style.md") =>
        {
            Some(content.as_str())
        }
        _ => None,
    });
    assert_eq!(cursor_write, Some(body));
    assert_eq!(claude_write, Some(body));
}

/// `.mdc` in the unified tree keeps Cursor’s `.mdc` dest; Claude normalizes to `{base}.md`.
#[test]
fn mdc_rule_name_compiles_to_claude_md_and_cursor_mdc() {
    let tree = AgentsTree::global([AgentsTree::Rules(vec![RuleNode {
        name: "020-strict.mdc".into(),
        body: RuleBody::Inline("strict\n".into()),
    }])]);
    let ctx = CompileContext::new(project(), "demo");
    let plan = compile(&tree, &ctx).unwrap();

    assert!(plan.ops.iter().any(|op| matches!(op,
        FsOp::WriteFile { path, .. }
            if path == &project().join(".cursor/rules/global--020-strict.mdc"))));
    assert!(plan.ops.iter().any(|op| matches!(op,
        FsOp::WriteFile { path, .. }
            if path == &project().join(".claude/rules/global--020-strict.md"))));
}

#[test]
fn tree_compiles_source_rules_as_hardlink_for_cursor_and_symlink_for_claude() {
    let src = PathBuf::from("/home/x/.agents/rules/global/hello.md");
    let tree = AgentsTree::global([AgentsTree::Rules(vec![RuleNode {
        name: "hello.md".into(),
        body: RuleBody::Source(src.clone()),
    }])]);
    let ctx = CompileContext::new(project(), "demo");
    let plan = compile(&tree, &ctx).unwrap();

    let cursor_link = plan
        .links()
        .find(|l| l.agent == AgentId::Cursor)
        .expect("cursor link");
    assert_eq!(cursor_link.kind, LinkKind::HardLink);
    assert_eq!(cursor_link.dest, project().join(".cursor/rules/global--hello.mdc"));

    let claude_link = plan
        .links()
        .find(|l| l.agent == AgentId::ClaudeCode)
        .expect("claude link");
    assert_eq!(claude_link.kind, LinkKind::Symlink);
    assert_eq!(claude_link.dest, project().join(".claude/rules/global--hello.md"));
}

#[test]
fn project_scope_overrides_global_with_project_prefix() {
    let tree = AgentsTree::global([
        AgentsTree::Rules(vec![RuleNode {
            name: "shared.md".into(),
            body: RuleBody::Inline("global\n".into()),
        }]),
        AgentsTree::project(
            "demo",
            [AgentsTree::Rules(vec![RuleNode {
                name: "shared.md".into(),
                body: RuleBody::Inline("project\n".into()),
            }])],
        ),
    ]);
    let ctx = CompileContext::new(project(), "demo");
    let plan = compile(&tree, &ctx).unwrap();

    // Both scopes emit WriteFile ops with different scope prefixes.
    assert!(plan.ops.iter().any(|op| matches!(op,
        FsOp::WriteFile { path, .. }
            if path == &project().join(".cursor/rules/global--shared.mdc"))));
    assert!(plan.ops.iter().any(|op| matches!(op,
        FsOp::WriteFile { path, .. }
            if path == &project().join(".cursor/rules/demo--shared.mdc"))));
}

#[test]
fn profile_scope_inherits_base_and_overrides_rules() {
    let tree = AgentsTree::global([
        AgentsTree::profile_def(
            "base",
            [],
            [AgentsTree::Rules(vec![RuleNode {
                name: "010-a.md".into(),
                body: RuleBody::Inline("from base\n".into()),
            }])],
        ),
        AgentsTree::profile_def(
            "derived",
            [ProfileId::from("base")],
            [AgentsTree::Rules(vec![RuleNode {
                name: "010-a.md".into(),
                body: RuleBody::Inline("from derived\n".into()),
            }])],
        ),
        AgentsTree::profile("derived", []),
    ]);
    let ctx = CompileContext::new(project(), "demo");
    let plan = compile(&tree, &ctx).unwrap();

    let cursor = plan.ops.iter().find_map(|op| match op {
        FsOp::WriteFile { path, content, .. }
            if path == &project().join(".cursor/rules/profile--derived--010-a.mdc") =>
        {
            Some(content.as_str())
        }
        _ => None,
    });
    assert_eq!(cursor, Some("from derived\n"));
}

#[test]
fn nested_workstream_scope_uses_ws_prefix() {
    let tree = AgentsTree::global([AgentsTree::project(
        "demo",
        [AgentsTree::workstream(
            "feat-auth",
            [AgentsTree::Rules(vec![RuleNode {
                name: "ws-rule.md".into(),
                body: RuleBody::Inline("ws\n".into()),
            }])],
        )],
    )]);
    let ctx = CompileContext::new(project(), "demo");
    let plan = compile(&tree, &ctx).unwrap();
    assert!(plan.ops.iter().any(|op| matches!(op,
        FsOp::WriteFile { path, .. }
            if path == &project().join(".cursor/rules/ws--feat-auth--ws-rule.mdc"))));
}
