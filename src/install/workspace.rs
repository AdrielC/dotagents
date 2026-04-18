//! Filesystem layout under [`crate::domain::AgentsHome`] for a managed project key.

use std::fs;
use std::io;
use std::path::Path;

pub fn create_project_dirs(agents_home: &Path, project_key: &str, dry_run: bool) -> io::Result<()> {
    if dry_run {
        return Ok(());
    }
    for sub in ["rules", "settings", "mcp", "skills"] {
        fs::create_dir_all(agents_home.join(sub).join(project_key))?;
    }
    Ok(())
}
