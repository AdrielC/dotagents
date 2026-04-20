#![cfg(unix)]
//! Round-trip: build a fake `.claude/` dir on disk, `ingest` it, `compile` the resulting tree
//! against *every* agent target, `apply`, and assert the output mirrors the input plus the
//! expected Cursor / Codex fan-out.

use std::fs;

use agentz::apply::{apply_plan, ApplyOptions};
use agentz::ingest::claude::{ingest, IngestOptions};
use agentz_core::compile::{compile, CompileContext};

fn write(path: &std::path::Path, content: &str) {
    if let Some(p) = path.parent() {
        fs::create_dir_all(p).unwrap();
    }
    fs::write(path, content).unwrap();
}

#[test]
fn claude_dir_round_trips_into_cursor_and_claude() {
    // ── arrange: a realistic .claude/ dir with every leaf kind ──
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    let claude = repo.join(".claude");
    write(
        &claude.join("rules/010-style.md"),
        "---\nalwaysApply: true\n---\n# style\n- small functions\n",
    );
    write(
        &claude.join("skills/review/SKILL.md"),
        "---\nname: review\ndescription: review diffs\n---\nbody\n",
    );
    write(
        &claude.join("agents/planner.md"),
        "---\nname: planner\ndescription: plan work\n---\nprompt body\n",
    );
    write(&claude.join("settings.json"), "{\"team\": true}\n");
    write(&claude.join("settings.local.json"), "{\"me\": true}\n");
    write(
        &repo.join(".mcp.json"),
        "{\"mcpServers\": {\"gh\": {\"command\": \"mcp-gh\"}}}\n",
    );
    write(&repo.join(".claudeignore"), "node_modules/\n.env\n");

    // ── act: ingest → compile → apply into a fresh output repo ──
    let report = ingest(&claude, &IngestOptions::default()).expect("ingest");
    assert!(
        report.warnings.is_empty(),
        "no warnings expected: {:?}",
        report.warnings
    );

    let out = tmp.path().join("out");
    fs::create_dir_all(&out).unwrap();
    let ctx = CompileContext::new(&out, "demo");
    let plan = compile(&report.tree, &ctx).expect("compile");
    let _apply = apply_plan(
        &plan,
        &ApplyOptions {
            force: true,
            dry_run: false,
        },
    )
    .expect("apply");

    // ── assert: Claude target reconstructed ──
    assert!(out.join(".claude/rules/global--010-style.md").is_file());
    assert!(out.join(".claude/skills/review/SKILL.md").is_file());
    assert!(out.join(".claude/agents/planner.md").is_file());
    assert!(out.join(".claude/settings.json").is_file());
    assert!(out.join(".claude/settings.local.json").is_file());
    assert!(out.join(".claudeignore").is_file());
    assert!(out.join(".mcp.json").is_file());

    // ── assert: Cursor target also populated (the whole point of the refactor) ──
    assert!(
        out.join(".cursor/rules/global--010-style.mdc").is_file(),
        "cursor .mdc produced from claude rules"
    );
    // Cursor has no slash-commands directory analogous to Claude's — skills stay Claude/Codex.
    assert!(
        !out.join(".cursor/commands/review.md").exists(),
        "skills do not leak into .cursor/commands/"
    );
    assert!(
        out.join(".cursor/mcp.json").is_file(),
        "cursor mcp.json fanned out"
    );
    // Cursor has no subagent concept — the Agents leaf silently skips.
    assert!(
        !out.join(".cursor/agents/planner.md").exists(),
        "subagents don't leak into cursor"
    );

    // ── assert: Codex target also populated ──
    assert!(out.join(".codex/skills/review/SKILL.md").is_file());
}

#[test]
fn ingest_is_idempotent_when_output_equals_input() {
    // Ingest a .claude/, compile to the same .claude/ tree, and the second apply must be a no-op.
    let tmp = tempfile::tempdir().unwrap();
    let claude = tmp.path().join(".claude");
    write(&claude.join("rules/foo.md"), "# foo\n");
    write(&claude.join("settings.json"), "{}\n");

    let report = ingest(&claude, &IngestOptions::default()).unwrap();
    let ctx = CompileContext::new(tmp.path(), "demo");
    let plan = compile(&report.tree, &ctx).unwrap();

    let first = apply_plan(
        &plan,
        &ApplyOptions {
            force: true,
            dry_run: false,
        },
    )
    .unwrap();
    let second = apply_plan(
        &plan,
        &ApplyOptions {
            force: false,
            dry_run: false,
        },
    )
    .unwrap();

    assert!(!first.wrote.is_empty(), "first apply wrote something");
    assert_eq!(second.wrote.len(), 0, "second apply is a no-op");
    assert!(
        second.skipped.len() >= first.wrote.len(),
        "same-content paths short-circuit on re-apply"
    );
}
