#![allow(clippy::print_stdout)]
//! **End-to-end demo.** Build a tree, compile, apply, inspect, re-apply.
//!
//! Run: `cargo run -p agentz --example demo`
//! Override target dir: `AGENTZ_DEMO_DIR=/tmp/foo cargo run -p agentz --example demo`
//!
//! The demo exercises the whole pipeline every other binary in this workspace would:
//!
//! 1. **Parse** an upstream Cursor `.mdc` fixture with [`agentz_core::parser::cursor::parse_mdc`].
//! 2. **Lift** the parsed rule into a `Repo` (a named catalogue of `AgentsTree` fragments — this
//!    is the "concept of a repo" that can come from local paths, git, or embedded content).
//! 3. **Compose** that repo with user-level overrides in a [`agentz_core::Workspace`].
//! 4. **Compile** the materialised tree to a pure `CompiledPlan` of `FsOp` values.
//! 5. **Apply** the plan — the only step that touches disk.
//! 6. **Inspect** what landed (paths + content of two key files).
//! 7. **Re-apply** to prove idempotency (identical content → skipped; no churn).

use std::path::PathBuf;

use agentz::apply::{apply_plan, ApplyOptions};
use agentz::env::{keys, var};
use agentz_core::compile::{compile, CompileContext};
use agentz_core::model::IgnoreKind;
use agentz_core::parser::cursor;
use agentz_core::tree::{
    AgentsTree, HookBinding, HookHandler, RuleBody, RuleNode, SettingsBody, SettingsNode,
    SkillBody, SkillNode,
};
use agentz_core::{AgentId, Repo, Workspace};

fn main() {
    let target = var(keys::DEMO_DIR)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("agentz-demo"));
    let _ = std::fs::remove_dir_all(&target);
    std::fs::create_dir_all(&target).expect("mkdir target");

    banner("1. parse an upstream Cursor .mdc fixture");
    let mdc_text = include_str!("../../agentz-core/tests/fixtures/cursor_always_apply.mdc");
    let parsed = cursor::parse_mdc(mdc_text).expect("parse fixture");
    println!(
        "  description : {}",
        parsed.description.as_deref().unwrap_or("")
    );
    println!("  alwaysApply : {:?}", parsed.always_apply);
    println!("  body bytes  : {}", parsed.body.len());

    banner("2. lift the parsed rule into an embedded Repo");
    let upstream = Repo::embedded(
        "upstream-cursor-rules",
        AgentsTree::global([AgentsTree::Rules(vec![RuleNode {
            // Pipe the parsed rule back out as the rule body so both Cursor and Claude read
            // the exact same frontmatter + body the upstream repo ships.
            name: "010-style.md".into(),
            body: RuleBody::Inline(cursor::emit_mdc(&parsed)),
        }])]),
    );
    println!("  repo id     : {}", upstream.id);
    println!("  source      : {:?}", upstream.source);

    banner("3. compose repo + user overrides in a Workspace");
    let workspace = Workspace::new(&target, "demo-project")
        .with_repo(upstream)
        .with_defaults(AgentsTree::global([
            // One skill, Cursor fans it to `.cursor/commands/review.md`; Claude to
            // `.claude/skills/review/SKILL.md`.
            AgentsTree::Skills(vec![SkillNode {
                name: "review".into(),
                body: SkillBody::Inline(
                    "---\nname: review\ndescription: Walk a diff and flag regressions\n---\n# Review\n\nRead the diff, run the tests, write up findings.\n"
                        .into(),
                ),
            }]),
            // Scope-aware settings: team-shared + personal.
            AgentsTree::Settings(vec![
                SettingsNode::claude_project(SettingsBody::Inline(
                    "{\n  \"includeCoAuthoredBy\": true\n}\n".into(),
                )),
                SettingsNode::claude_local(SettingsBody::Inline(
                    "{\n  \"env\": { \"DEBUG\": \"1\" }\n}\n".into(),
                )),
            ]),
            // One hook, the compiler routes it to both Cursor's hooks.json and Claude's
            // settings companion.
            AgentsTree::Hooks(vec![HookBinding::pre_tool_use(
                "Bash",
                HookHandler::command("scripts/audit.sh"),
            )]),
            // Ignores, one per agent that supports them.
            AgentsTree::Ignore {
                agent: AgentId::Cursor,
                kind: IgnoreKind::Primary,
                patterns: vec!["node_modules/".into(), ".env".into()],
            },
            AgentsTree::Ignore {
                agent: AgentId::ClaudeCode,
                kind: IgnoreKind::Primary,
                patterns: vec!["node_modules/".into(), ".env".into()],
            },
            // MCP servers.
            AgentsTree::Mcp(serde_json::json!({
                "mcpServers": {
                    "github": { "command": "mcp-github", "args": ["--token", "$GH_TOKEN"] }
                }
            })),
        ]));
    let tree = workspace.materialize();

    banner("4. compile (pure — no IO)");
    let ctx = CompileContext::new(&target, "demo-project");
    let plan = compile(&tree, &ctx).expect("compile");
    println!("  ops      : {}", plan.ops.len());
    println!("  warnings : {}", plan.warnings.len());

    banner("5. apply (the only IO)");
    let report = apply_plan(
        &plan,
        &ApplyOptions {
            force: false,
            dry_run: false,
        },
    )
    .expect("apply");
    println!("  wrote    : {}", report.wrote.len());
    println!("  linked   : {}", report.applied.len());
    println!("  skipped  : {}", report.skipped.len());

    banner("6. files landed");
    let mut paths: Vec<_> = report.wrote.iter().collect();
    paths.sort();
    for p in &paths {
        let rel = p.strip_prefix(&target).unwrap_or(p);
        println!("  {}", rel.display());
    }

    banner("7. content preview: .cursor/rules/global--010-style.mdc");
    let preview = target.join(".cursor/rules/global--010-style.mdc");
    print_with_prefix(&preview, "  │ ");

    banner("8. content preview: .claude/settings.local.json");
    let local = target.join(".claude/settings.local.json");
    print_with_prefix(&local, "  │ ");

    banner("9. idempotent re-apply");
    let report2 = apply_plan(
        &plan,
        &ApplyOptions {
            force: false,
            dry_run: false,
        },
    )
    .expect("apply-again");
    println!(
        "  wrote    : {}  (was {})",
        report2.wrote.len(),
        report.wrote.len()
    );
    println!(
        "  skipped  : {}  (same-content paths short-circuited)",
        report2.skipped.len()
    );

    println!("\nDone. Target: {}", target.display());
    println!("Override it with `{}=/some/dir`.", keys::DEMO_DIR);
}

fn banner(title: &str) {
    println!("\n── {title} ──");
}

fn print_with_prefix(path: &std::path::Path, prefix: &str) {
    match std::fs::read_to_string(path) {
        Ok(content) => {
            for line in content.lines() {
                println!("{prefix}{line}");
            }
        }
        Err(e) => println!("  (failed to read {}: {e})", path.display()),
    }
}
