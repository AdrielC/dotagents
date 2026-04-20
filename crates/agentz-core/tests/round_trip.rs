//! Round-trip tests against vendored fixtures modelled on real Cursor `.mdc` and Claude
//! `SKILL.md` files. Parsing + re-emitting must preserve semantic content; a second parse of the
//! re-emitted text must match the first parse.
//!
//! These fixtures are hand-crafted minimal examples covering the shapes documented by Anthropic
//! (SKILL.md with `name`/`description`/`allowed-tools`) and Cursor (`.mdc` with `alwaysApply` or
//! `globs`). They intentionally don't vendor large upstream repos — anything above a couple of
//! keys belongs in docs.

use std::path::Path;

use agentz_core::parser::{claude, cursor};

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"))
}

#[test]
fn cursor_always_apply_fixture_parses_and_round_trips() {
    let raw = fixture("cursor_always_apply.mdc");
    let parsed = cursor::parse_mdc(&raw).expect("parse");
    assert_eq!(parsed.always_apply, Some(true));
    assert_eq!(
        parsed.description.as_deref(),
        Some("Always-on coding style rules for this repo")
    );
    assert!(parsed.globs.is_empty());
    assert!(parsed.body.starts_with("# Coding style"));

    let reemitted = cursor::emit_mdc(&parsed);
    let reparsed = cursor::parse_mdc(&reemitted).expect("re-parse");
    assert_eq!(parsed, reparsed);
}

#[test]
fn cursor_globs_fixture_parses_and_round_trips() {
    let raw = fixture("cursor_globs.mdc");
    let parsed = cursor::parse_mdc(&raw).expect("parse");
    assert_eq!(parsed.always_apply, Some(false));
    assert_eq!(parsed.globs, vec!["**/*.rs", "Cargo.toml"]);
    assert!(parsed.description.as_deref().unwrap_or("").contains("Rust"));

    let reemitted = cursor::emit_mdc(&parsed);
    let reparsed = cursor::parse_mdc(&reemitted).expect("re-parse");
    assert_eq!(parsed, reparsed);
}

#[test]
fn claude_skill_fixture_parses_and_round_trips() {
    let raw = fixture("claude_skill.md");
    let parsed = claude::parse_skill_md(&raw).expect("parse");
    assert_eq!(parsed.name, "code-reviewer");
    assert_eq!(parsed.allowed_tools, vec!["Read", "Grep", "Bash"]);
    assert!(parsed
        .description
        .as_deref()
        .unwrap_or("")
        .starts_with("Reviews a diff"));
    assert!(parsed.body.starts_with("# Code reviewer"));

    let reemitted = claude::emit_skill_md(&parsed);
    let reparsed = claude::parse_skill_md(&reemitted).expect("re-parse");
    assert_eq!(parsed, reparsed);
}

#[test]
fn cursor_mdc_then_compile_into_claude_md_path() {
    use agentz_core::compile::{compile, CompileContext, FsOp};
    use agentz_core::tree::{AgentsTree, RuleBody, RuleNode};

    // Parse an upstream Cursor fixture, then lift it into a RuleNode as if the user pulled a
    // rule from a repo. The compiler should fan it out to Cursor's .mdc path AND Claude's .md
    // path with the appropriate filename rewrites.
    let raw = fixture("cursor_always_apply.mdc");
    let parsed = cursor::parse_mdc(&raw).expect("parse");
    // Treat the whole document (frontmatter + body) as the rule body — that's what you'd
    // install if you wanted Cursor to re-parse its own frontmatter upstream.
    let rule = RuleNode {
        name: "style.md".into(),
        body: RuleBody::Inline(cursor::emit_mdc(&parsed)),
    };
    let tree = AgentsTree::global([AgentsTree::Rules(vec![rule])]);
    let ctx = CompileContext::new("/tmp/demo", "demo");
    let plan = compile(&tree, &ctx).unwrap();

    let cursor_dest = std::path::PathBuf::from("/tmp/demo/.cursor/rules/global--style.mdc");
    let claude_dest = std::path::PathBuf::from("/tmp/demo/.claude/rules/global--style.md");
    assert!(
        plan.ops
            .iter()
            .any(|op| matches!(op, FsOp::WriteFile { path, .. } if path == &cursor_dest)),
        "cursor .mdc path emitted"
    );
    assert!(
        plan.ops
            .iter()
            .any(|op| matches!(op, FsOp::WriteFile { path, .. } if path == &claude_dest)),
        "claude .md path emitted"
    );
}
