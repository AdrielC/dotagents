#![cfg(unix)]
//! Integration test covering the full "pure data → IO" split:
//!   1. Build an `AgentsTree` AST (pure).
//!   2. Compile to `CompiledPlan` (pure, in `agentz-core`).
//!   3. Apply with `agentz::apply::apply_plan` (the only IO).

use std::fs;

use agentz::apply::{apply_plan, ApplyOptions};
use agentz_core::compile::{compile, CompileContext};
use agentz_core::tree::{AgentsTree, RuleBody, RuleNode, SettingsBody, SettingsNode};
use agentz_core::AgentId;

#[test]
fn compile_then_apply_produces_real_files() {
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path().join("repo");
    fs::create_dir_all(&project).unwrap();

    let tree = AgentsTree::global([
        AgentsTree::Rules(vec![RuleNode {
            name: "010-style.md".into(),
            body: RuleBody::Inline("# style\n".into()),
        }]),
        AgentsTree::Settings(vec![SettingsNode {
            agent: AgentId::ClaudeCode,
            file_name: "settings.local.json".into(),
            body: SettingsBody::Inline("{\"ok\":true}\n".into()),
        }]),
    ]);
    let ctx = CompileContext::new(&project, "demo");
    let plan = compile(&tree, &ctx).unwrap();

    let report = apply_plan(
        &plan,
        &ApplyOptions {
            force: true,
            dry_run: false,
        },
    )
    .unwrap();

    let rule_cursor = project.join(".cursor/rules/global--010-style.mdc");
    let rule_claude = project.join(".claude/rules/global--010-style.md");
    let claude_settings = project.join(".claude/settings.local.json");
    assert!(rule_cursor.is_file(), "cursor rule written");
    assert!(rule_claude.is_file(), "claude rule written");
    assert!(claude_settings.is_file(), "claude settings written");
    assert_eq!(fs::read_to_string(&rule_cursor).unwrap(), "# style\n");
    assert!(report.wrote.iter().any(|p| p == &rule_cursor));
}
