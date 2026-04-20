#![allow(clippy::print_stdout, clippy::print_stderr)]
//! **`agentz` CLI.** Six subcommands, all thin wrappers around the library:
//!
//! | Subcommand        | What it does |
//! |-------------------|--------------|
//! | `agentz validate` | Parse `agentz.toml`, print the resulting `AgentsTree` summary. |
//! | `agentz compile`  | Parse `agentz.toml` + compile; print `CompiledPlan.ops` count. |
//! | `agentz diff`     | Compute a structured diff against the real FS and print it. |
//! | `agentz apply`    | Compile + apply. `--dry-run` is equivalent to `diff`. |
//! | `agentz ingest`   | Ingest a per-agent config dir and print the resulting tree. |
//! | `agentz migrate`  | Ingest one agent's dir, compile for all agents, apply. |
//!
//! Every subcommand honours `--config <path>` (default `./agentz.toml`) and exits 0 on success,
//! non-zero with a structured message on error. No side-effects happen outside `apply` /
//! `migrate`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use agentz::apply::{apply_plan, ApplyOptions};
use agentz::ingest;
use agentz_core::compile::{compile, CompileContext};
use agentz_core::config::AgentzConfig;
use agentz_core::diff;
use agentz_core::render::{render_tree, RenderOptions};
use agentz_core::tree::AgentsTree;
use agentz_core::{AgentId, RealFileSource};
use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(
    name = "agentz",
    about = "Unified agent config compiler / migrator / diff tool",
    version
)]
struct Cli {
    /// Path to the declarative `agentz.toml` config. Optional for `ingest` / `migrate`.
    #[arg(long, short = 'c', global = true, default_value = "agentz.toml")]
    config: PathBuf,

    /// Target project root (the dir that receives `.claude/`, `.cursor/`, etc.). Defaults to cwd.
    #[arg(long, global = true)]
    target: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Parse `agentz.toml` and dump its `AgentsTree`.
    Validate,

    /// Compile `agentz.toml` to ops without writing to disk.
    Compile,

    /// Show what `apply` would change, relative to the current filesystem state.
    Diff,

    /// Compile + write. `--dry-run` for a preview.
    Apply {
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        force: bool,
    },

    /// Read an existing per-agent config directory and print the ingested tree.
    Ingest {
        /// Which agent's format to parse (`claude-code`, `cursor`, `codex`, …).
        #[arg(long, value_enum, default_value_t = AgentIdArg::ClaudeCode)]
        agent: AgentIdArg,
        /// Directory to read. E.g. `./.claude` for Claude Code.
        path: PathBuf,
    },

    /// Ingest one agent's dir + compile for every agent + apply into `--target`.
    Migrate {
        #[arg(long, value_enum, default_value_t = AgentIdArg::ClaudeCode)]
        from: AgentIdArg,
        /// The source directory to ingest.
        source: PathBuf,
    },
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum AgentIdArg {
    Cursor,
    ClaudeCode,
    Codex,
    OpenCode,
    Gemini,
    Factory,
    Github,
    Ampcode,
}

impl From<AgentIdArg> for AgentId {
    fn from(a: AgentIdArg) -> Self {
        match a {
            AgentIdArg::Cursor => AgentId::Cursor,
            AgentIdArg::ClaudeCode => AgentId::ClaudeCode,
            AgentIdArg::Codex => AgentId::Codex,
            AgentIdArg::OpenCode => AgentId::OpenCode,
            AgentIdArg::Gemini => AgentId::Gemini,
            AgentIdArg::Factory => AgentId::Factory,
            AgentIdArg::Github => AgentId::Github,
            AgentIdArg::Ampcode => AgentId::Ampcode,
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("agentz: {e}");
            ExitCode::from(1)
        }
    }
}

fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    let target = cli
        .target
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    match &cli.command {
        Command::Validate => cmd_validate(&cli.config),
        Command::Compile => cmd_compile(&cli.config, &target).map(|_| ()),
        Command::Diff => cmd_diff(&cli.config, &target),
        Command::Apply { dry_run, force } => cmd_apply(&cli.config, &target, *dry_run, *force),
        Command::Ingest { agent, path } => cmd_ingest((*agent).into(), path),
        Command::Migrate { from, source } => cmd_migrate((*from).into(), source, &target),
    }
}

// ── Subcommands ───────────────────────────────────────────────────────────────

fn cmd_validate(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let cfg = load_config(path)?;
    let tree = cfg.to_tree()?;
    println!("ok   : {}", path.display());
    println!("key  : {}", cfg.workspace.project_key);
    summarise_tree(&tree);
    Ok(())
}

fn cmd_compile(
    path: &Path,
    target: &Path,
) -> Result<agentz_core::CompiledPlan, Box<dyn std::error::Error>> {
    let cfg = load_config(path)?;
    let tree = cfg.to_tree()?;
    let rendered = render_tree(
        &tree,
        &cfg.render_context(&target.display().to_string()),
        &RenderOptions::default(),
    )?;
    let mut ctx = CompileContext::new(target, cfg.project_key());
    if !cfg.workspace.target_agents.is_empty() {
        ctx = ctx.with_agents(cfg.workspace.target_agents.iter().copied());
    }
    let plan = compile(&rendered, &ctx)?;
    println!("compiled : {} ops", plan.ops.len());
    if !plan.warnings.is_empty() {
        println!("warnings : {}", plan.warnings.len());
        for w in &plan.warnings {
            println!("  - {w}");
        }
    }
    Ok(plan)
}

fn cmd_diff(path: &Path, target: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let plan = cmd_compile(path, target)?;
    let diff = diff::compute(&plan, &RealFileSource);
    print!("{}", diff.render_tf_style(Some(target)));
    Ok(())
}

fn cmd_apply(
    path: &Path,
    target: &Path,
    dry_run: bool,
    force: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let plan = cmd_compile(path, target)?;
    if dry_run {
        let diff = diff::compute(&plan, &RealFileSource);
        print!("{}", diff.render_tf_style(Some(target)));
        return Ok(());
    }
    let report = apply_plan(
        &plan,
        &ApplyOptions {
            force,
            dry_run: false,
        },
    )?;
    println!(
        "wrote {}, linked {}, skipped {}.",
        report.wrote.len(),
        report.applied.len(),
        report.skipped.len()
    );
    Ok(())
}

fn cmd_ingest(agent: AgentId, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let report = ingest::ingest_dir(agent, path, &ingest::IngestOptions::default())?;
    println!("from     : {}", path.display());
    summarise_tree(&report.tree);
    if !report.unknown_paths.is_empty() {
        println!("unknown  : {}", report.unknown_paths.len());
        for p in &report.unknown_paths {
            println!("  - {}", p.display());
        }
    }
    Ok(())
}

fn cmd_migrate(
    from: AgentId,
    source: &Path,
    target: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    // Mirror ignores to every other built-in agent so `.claude → .cursor` migrations produce
    // both `.claudeignore` and `.cursorignore`. Callers that want finer control go through the
    // library API directly.
    let mirror_targets: Vec<AgentId> = AgentId::all()
        .iter()
        .copied()
        .filter(|a| *a != from)
        .collect();
    let opts = ingest::IngestOptions {
        mirror_ignores_to: mirror_targets,
        ..Default::default()
    };
    let report = ingest::ingest_dir(from, source, &opts)?;
    let ctx = CompileContext::new(target, "migrated");
    let plan = compile(&report.tree, &ctx)?;
    let diff = diff::compute(&plan, &RealFileSource);
    println!("── plan ──");
    print!("{}", diff.render_tf_style(Some(target)));
    if diff.is_empty_change_set() {
        println!("(nothing to do)");
        return Ok(());
    }
    let applied = apply_plan(
        &plan,
        &ApplyOptions {
            force: true,
            dry_run: false,
        },
    )?;
    println!(
        "\napplied  : wrote {}, linked {}",
        applied.wrote.len(),
        applied.applied.len()
    );
    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn load_config(path: &Path) -> Result<AgentzConfig, Box<dyn std::error::Error>> {
    let text =
        fs::read_to_string(path).map_err(|e| format!("read config {}: {e}", path.display()))?;
    Ok(AgentzConfig::from_toml(&text)?)
}

fn summarise_tree(tree: &AgentsTree) {
    let AgentsTree::Scope { children, .. } = tree else {
        println!("(unexpected root)");
        return;
    };
    for child in children {
        match child {
            AgentsTree::Rules(rs) => println!("rules    : {}", rs.len()),
            AgentsTree::Skills(ss) => println!("skills   : {}", ss.len()),
            AgentsTree::Agents(ags) => println!("subagents: {}", ags.len()),
            AgentsTree::Settings(st) => println!("settings : {}", st.len()),
            AgentsTree::Hooks(hs) => println!("hooks    : {}", hs.len()),
            AgentsTree::Ignore {
                agent, patterns, ..
            } => {
                println!("ignore   : {:?} ({})", agent, patterns.len());
            }
            AgentsTree::Mcp(_) => println!("mcp      : 1"),
            AgentsTree::TextFile { name, .. } => println!("text     : {name}"),
            AgentsTree::Scope { .. } | AgentsTree::ProfileDef { .. } => {}
        }
    }
}
