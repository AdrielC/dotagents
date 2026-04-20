//! **Ingest** — read an existing per-agent directory on disk and lift it into an `AgentsTree`.
//!
//! Symmetric to [`crate::apply`]: `apply` walks a plan and creates files, `ingest` walks files and
//! produces a typed tree you can then `compile` against any other agent target. That's the
//! round-trip the rest of this workspace was built to enable — take a `.claude/` dir, `ingest`,
//! compile, apply into `.cursor/`.
//!
//! Sub-modules live per source format (one per agent we can read). Today that's
//! [`claude`], which covers `.claude/` project directories documented at
//! <https://code.claude.com/docs/en/overview>.

pub mod claude;
