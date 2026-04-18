//! First-run initialization of the agents home directory.

use std::fs;
use std::io;
use std::path::Path;

use crate::config::{read_config, write_config, AgentsConfig};

use super::types::InitOptions;

/// Create the standard `~/.agents/` tree when missing (safe to call repeatedly).
pub fn init_agents_home(agents_home: &Path, opts: &InitOptions) -> io::Result<()> {
    let dirs = [
        agents_home.join("rules/global"),
        agents_home.join("rules/_example"),
        agents_home.join("settings/global"),
        agents_home.join("mcp/global"),
        agents_home.join("skills/global"),
        agents_home.join("scripts"),
        agents_home.join("local"),
    ];
    for d in &dirs {
        fs::create_dir_all(d)?;
    }

    let config_path = agents_home.join("config.json");
    if !config_path.exists() || opts.force {
        let cfg = if config_path.exists() && opts.force {
            read_config(&config_path).unwrap_or_default()
        } else {
            AgentsConfig::default()
        };
        write_config(&config_path, &cfg)?;
    }

    let starter = agents_home.join("rules/global/rules.mdc");
    if !starter.exists() {
        fs::write(
            starter,
            b"---\ndescription: Starter rules managed by agents-unified\nglobs: []\nalwaysApply: true\n---\n\n# Rules\n\nEdit shared rules under `~/.agents/rules/global/`.\n",
        )?;
    }

    Ok(())
}
