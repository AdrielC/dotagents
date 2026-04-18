//! Pure compilation tests. Walks an `AgentsTree` AST and asserts the produced `FsOp` list.

use std::path::PathBuf;

use agentz_core::compile::{compile, CompileContext, FsOp};
use agentz_core::model::{AgentId, LinkKind};
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
    let tree = AgentsTree::Scope {
        name: "root".into(),
        children: vec![
            AgentsTree::global([AgentsTree::Rules(vec![RuleNode {
                name: "shared.md".into(),
                body: RuleBody::Inline("global\n".into()),
            }])]),
            AgentsTree::project(
                "demo",
                [AgentsTree::Rules(vec![RuleNode {
                    name: "shared.md".into(),
                    body: RuleBody::Inline("project\n".into()),
                }])],
            ),
        ],
    };
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
