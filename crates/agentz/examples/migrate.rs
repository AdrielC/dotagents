#![allow(clippy::print_stdout, clippy::print_stderr)]
//! **Migrate a `.claude/` directory into every supported agent (notably Cursor).**
//!
//! Run on the workspace's own scratch `.claude/` fixture:
//!
//! ```bash
//! cargo run -p agentz --example migrate
//! ```
//!
//! Point at a real directory:
//!
//! ```bash
//! cargo run -p agentz --example migrate -- /path/to/repo/.claude [/path/to/output]
//! ```
//!
//! The output path defaults to `$AGENTZ_DEMO_DIR` or `$TMPDIR/agentz-migrate`.
//!
//! ## What happens
//!
//! 1. `agentz::ingest::claude::ingest(source)` walks `.claude/` and returns an agent-agnostic
//!    [`AgentsTree`] (rules + skills are bare leaves, subagents land in `Agents`, settings are
//!    locked to [`AgentId::ClaudeCode`]).
//! 2. `agentz_core::compile::compile(tree, ctx)` fans the tree out to **every** agent in `SPECS`
//!    (Cursor, Claude, Codex, …) — this is the one place the data-driven refactor pays off most
//!    visibly: the same source tree produces a `.cursor/` layout without a single branch.
//! 3. `agentz::apply::apply_plan(&plan, ..)` writes the files.
//!
//! No flags, no config — because the whole point of the library is that the data model describes
//! what to do.

use std::path::{Path, PathBuf};

use agentz::apply::{apply_plan, ApplyOptions};
use agentz::env::{keys, var};
use agentz::ingest::claude::{ingest, IngestOptions};
use agentz_core::compile::{compile, CompileContext};
use agentz_core::tree::AgentsTree;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let source = args
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(default_source);
    let target = args
        .get(1)
        .map(PathBuf::from)
        .or_else(|| var(keys::DEMO_DIR).map(PathBuf::from))
        .unwrap_or_else(|| std::env::temp_dir().join("agentz-migrate"));

    ensure_source(&source);
    reset_target(&target);

    banner(&format!("1. ingest {}", source.display()));
    let report = ingest(&source, &IngestOptions::default()).expect("ingest");
    print_tree_summary(&report.tree);
    if !report.warnings.is_empty() {
        println!("  warnings:");
        for w in &report.warnings {
            println!("    - {w}");
        }
    }
    if !report.unknown_paths.is_empty() {
        println!("  unknown files skipped:");
        for p in &report.unknown_paths {
            println!("    - {}", p.display());
        }
    }

    banner("2. compile the ingested tree for every target agent");
    let ctx = CompileContext::new(&target, "migrated");
    let plan = compile(&report.tree, &ctx).expect("compile");
    println!("  ops      : {}", plan.ops.len());
    println!("  warnings : {}", plan.warnings.len());

    banner(&format!("3. apply into {}", target.display()));
    let applied = apply_plan(
        &plan,
        &ApplyOptions {
            force: true,
            dry_run: false,
        },
    )
    .expect("apply");
    println!("  wrote    : {}", applied.wrote.len());
    println!("  linked   : {}", applied.applied.len());

    banner("4. files produced per agent");
    summarise_target(&target);

    println!(
        "\nDone. Source: {}\n      Target: {}",
        source.display(),
        target.display()
    );
    println!("Open `.cursor/rules/` and `.claude/rules/` under the target to compare.");
}

/// When no argument is passed, synthesise a small `.claude/` on the fly so the example always
/// produces something meaningful without requiring the caller to prepare a fixture.
fn default_source() -> PathBuf {
    let scratch = std::env::temp_dir().join("agentz-migrate-src");
    let claude = scratch.join(".claude");
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&claude).unwrap();
    write(
        &claude.join("rules/010-style.md"),
        "---\nalwaysApply: true\ndescription: Small functions, doc everything\n---\n# Style\n\n- Prefer composable functions.\n- Every public item has a doc.\n",
    );
    write(
        &claude.join("skills/review/SKILL.md"),
        "---\nname: review\ndescription: Walk a diff and flag regressions\n---\n# Review\n\n1. Read the diff.\n2. Run the tests.\n3. Write up findings.\n",
    );
    write(
        &claude.join("agents/planner.md"),
        "---\nname: planner\ndescription: Plan complex work before executing\nmodel: claude-opus-4-7\n---\n# Planner\n\nBreak the task into 3-7 concrete steps before coding.\n",
    );
    write(
        &claude.join("settings.json"),
        "{\n  \"includeCoAuthoredBy\": true\n}\n",
    );
    write(
        &claude.join("settings.local.json"),
        "{\n  \"env\": { \"DEBUG\": \"1\" }\n}\n",
    );
    write(
        &scratch.join(".mcp.json"),
        "{\n  \"mcpServers\": {\n    \"github\": { \"command\": \"mcp-gh\" }\n  }\n}\n",
    );
    write(&scratch.join(".claudeignore"), "node_modules/\n.env\n");
    println!(
        "(no source passed; synthesised a demo .claude at {})\n",
        claude.display()
    );
    claude
}

fn ensure_source(source: &Path) {
    if !source.exists() {
        eprintln!("error: source does not exist: {}", source.display());
        std::process::exit(2);
    }
    if !source.is_dir() {
        eprintln!("error: source is not a directory: {}", source.display());
        std::process::exit(2);
    }
}

fn reset_target(target: &Path) {
    let _ = std::fs::remove_dir_all(target);
    std::fs::create_dir_all(target).expect("mkdir target");
}

fn banner(title: &str) {
    println!("\n── {title} ──");
}

fn write(path: &Path, content: &str) {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p).unwrap();
    }
    std::fs::write(path, content).unwrap();
}

fn print_tree_summary(tree: &AgentsTree) {
    let AgentsTree::Scope { children, .. } = tree else {
        println!("  (unexpected: top-level is not a Scope)");
        return;
    };
    for child in children {
        match child {
            AgentsTree::Rules(rs) => println!("  rules    : {} file(s)", rs.len()),
            AgentsTree::Skills(ss) => println!("  skills   : {} skill(s)", ss.len()),
            AgentsTree::Agents(ags) => println!("  subagents: {} agent(s)", ags.len()),
            AgentsTree::Settings(st) => println!("  settings : {} scope(s)", st.len()),
            AgentsTree::Hooks(hs) => println!("  hooks    : {} binding(s)", hs.len()),
            AgentsTree::Ignore {
                agent, patterns, ..
            } => {
                println!("  ignore   : {:?} ({} patterns)", agent, patterns.len());
            }
            AgentsTree::Mcp(_) => println!("  mcp      : 1 config blob"),
            AgentsTree::TextFile { name, .. } => println!("  text     : {name}"),
            AgentsTree::Scope { .. } | AgentsTree::ProfileDef { .. } => {}
        }
    }
}

fn summarise_target(target: &Path) {
    for dir in [".claude", ".cursor", ".codex"] {
        let sub = target.join(dir);
        if !sub.exists() {
            continue;
        }
        println!("  {dir}/");
        walk(&sub, target, 2);
    }
    for flat in [".mcp.json", ".cursorignore", ".claudeignore", "CLAUDE.md"] {
        let p = target.join(flat);
        if p.is_file() {
            println!("  {flat}");
        }
    }
}

fn walk(dir: &Path, target: &Path, indent: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.path());
    for entry in entries {
        let p = entry.path();
        let rel = p.strip_prefix(target).unwrap_or(&p);
        println!("{:indent$}{}", "", rel.display(), indent = indent);
        if p.is_dir() {
            walk(&p, target, indent + 2);
        }
    }
}
