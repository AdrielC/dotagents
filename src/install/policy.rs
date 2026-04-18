//! Which built-in agents participate in an install (from `config.json` + CLI override).

use std::collections::HashSet;

use crate::config::AgentsConfig;
use crate::model::AgentId;

use super::types::InstallOptions;

pub fn agents_enabled_filter(cfg: &AgentsConfig, opt: &InstallOptions) -> HashSet<AgentId> {
    if let Some(ref list) = opt.agents {
        return list.iter().copied().collect();
    }
    let mut set = HashSet::new();
    for a in AgentId::all() {
        if agent_enabled_in_config(cfg, *a) {
            set.insert(*a);
        }
    }
    if set.is_empty() {
        AgentId::all().iter().copied().collect()
    } else {
        set
    }
}

fn agent_enabled_in_config(cfg: &AgentsConfig, agent: AgentId) -> bool {
    if let Some(v) = cfg.extra.get("agents") {
        if let Some(obj) = v.as_object() {
            if let Some(agent_obj) = obj.get(agent.as_str()) {
                if let Some(en) = agent_obj.get("enabled") {
                    return en.as_bool().unwrap_or(true);
                }
            }
        }
    }
    match agent {
        AgentId::OpenCode => cfg
            .extra
            .get("agents")
            .and_then(|v| v.get("opencode"))
            .and_then(|o| o.get("enabled"))
            .and_then(|b| b.as_bool())
            .unwrap_or(false),
        _ => true,
    }
}
