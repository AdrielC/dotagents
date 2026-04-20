//! Claude-specific ingest convenience wrapper. The real work lives in
//! [`super::ingest_dir_with`] driven by [`agentz_core::dialects::ClaudeDialect`]; this module
//! preserves the legacy `ingest::claude::ingest(...)` call sites.

use std::path::Path;

pub use super::{IngestError, IngestOptions, IngestReport};
use agentz_core::AgentId;

/// Shortcut for `ingest_dir(AgentId::ClaudeCode, root, opts)`.
pub fn ingest(root: &Path, opts: &IngestOptions) -> Result<IngestReport, IngestError> {
    super::ingest_dir(AgentId::ClaudeCode, root, opts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentz_core::model::IgnoreKind;
    use agentz_core::tree::AgentsTree;
    use std::fs;
    use tempfile::tempdir;

    fn write(path: &Path, content: &str) {
        if let Some(p) = path.parent() {
            fs::create_dir_all(p).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    #[test]
    fn ingest_minimal_claude_dir() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join(".claude");
        write(
            &root.join("rules/010-style.md"),
            "---\nalwaysApply: true\n---\n# Style\n",
        );
        write(
            &root.join("skills/review/SKILL.md"),
            "---\nname: review\ndescription: review diffs\n---\nbody\n",
        );
        write(
            &root.join("agents/planner.md"),
            "---\nname: planner\ndescription: plan the work\n---\nprompt\n",
        );
        write(&root.join("settings.json"), "{\"ok\":true}\n");
        write(&root.join("settings.local.json"), "{\"me\":true}\n");

        let report = ingest(&root, &IngestOptions::default()).unwrap();
        let AgentsTree::Scope { children, .. } = &report.tree else {
            panic!("expected Scope");
        };
        assert!(children.iter().any(|c| matches!(c, AgentsTree::Rules(_))));
        assert!(children.iter().any(|c| matches!(c, AgentsTree::Skills(_))));
        assert!(children.iter().any(|c| matches!(c, AgentsTree::Agents(_))));
        assert!(children
            .iter()
            .any(|c| matches!(c, AgentsTree::Settings(_))));
    }

    #[test]
    fn ingest_picks_up_repo_root_mcp_and_ignore() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join(".claude");
        fs::create_dir_all(&root).unwrap();
        write(
            &tmp.path().join(".mcp.json"),
            "{\"mcpServers\": {\"x\": {\"command\": \"echo\"}}}\n",
        );
        write(
            &tmp.path().join(".claudeignore"),
            "node_modules/\n.env\n# comment\n",
        );

        let report = ingest(&root, &IngestOptions::default()).unwrap();
        let AgentsTree::Scope { children, .. } = &report.tree else {
            panic!()
        };
        assert!(children.iter().any(|c| matches!(c, AgentsTree::Mcp(_))));
        let ignore = children
            .iter()
            .find_map(|c| match c {
                AgentsTree::Ignore {
                    agent,
                    kind,
                    patterns,
                } if *agent == AgentId::ClaudeCode && *kind == IgnoreKind::Primary => {
                    Some(patterns.clone())
                }
                _ => None,
            })
            .expect("ignore leaf");
        assert_eq!(ignore, vec!["node_modules/", ".env"]);
    }
}
