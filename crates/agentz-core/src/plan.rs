//! **Plan DAG** — deterministic, typed flow of steps for a workstream.
//!
//! A [`Plan`] is an [`Objective`] plus a directed acyclic graph of [`Step`]s. Each step carries a
//! [`StepKind`] variant describing what the runtime should do. Building a plan is pure data; the
//! DAG guarantees a deterministic topological order.
//!
//! ```
//! use agentz_core::plan::{Dag, Objective, Plan, Step, StepKind};
//! use agentz_core::id::StepId;
//!
//! let install = Step::new("install", "install agent links", StepKind::Install { project_key: "demo".into() });
//! let compile = Step::new("compile", "compile agent tree", StepKind::Compile);
//! let commit  = Step::new("commit",  "commit config",       StepKind::Shell { command: "git commit -am wip".into() });
//!
//! let mut dag = Dag::new();
//! dag.add(compile.clone());
//! dag.add(install.clone());
//! dag.add(commit.clone());
//! dag.edge(&compile.id, &install.id);      // compile -> install
//! dag.edge(&install.id, &commit.id);       // install -> commit
//!
//! let order = dag.topo().unwrap();
//! assert_eq!(order[0].as_str(), "compile");
//! let plan = Plan {
//!     objective: Objective::new("ship the feature"),
//!     dag,
//! };
//! assert_eq!(plan.objective.summary, "ship the feature");
//! ```

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::id::StepId;
use crate::model::AgentId;

/// Short, human-readable statement of the plan's goal.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Objective {
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acceptance: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

impl Objective {
    pub fn new(summary: impl Into<String>) -> Self {
        Self { summary: summary.into(), acceptance: None, tags: Vec::new() }
    }

    pub fn with_acceptance(mut self, acceptance: impl Into<String>) -> Self {
        self.acceptance = Some(acceptance.into());
        self
    }

    pub fn with_tags<S: Into<String>>(mut self, tags: impl IntoIterator<Item = S>) -> Self {
        self.tags = tags.into_iter().map(Into::into).collect();
        self
    }
}

/// Lifecycle status of a step. `Pending` is the default — runtimes advance this during execution.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    #[default]
    Pending,
    Ready,
    InProgress,
    Done,
    Skipped,
    Failed,
}

/// Discriminator for what a step should accomplish. Purely declarative.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum StepKind {
    /// Compile an [`crate::tree::AgentsTree`] into [`crate::compile::CompiledPlan`]s.
    Compile,
    /// Install the compiled links for a project.
    Install { project_key: String },
    /// Invoke a plugin by id with a JSON payload.
    Plugin { id: String, payload: serde_json::Value },
    /// Call an MCP tool hosted by the runtime.
    McpTool { tool: String, arguments: serde_json::Value },
    /// Dispatch a prompt to the per-workstream agent (ACP).
    AgentPrompt { agent: AgentId, prompt: String },
    /// Shell command (runtime decides whether it is allowed).
    Shell { command: String },
    /// A marker step (useful for DAG joins / fan-in/out).
    Noop,
}

/// A single step in a [`Plan`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Step {
    pub id: StepId,
    pub title: String,
    pub kind: StepKind,
    #[serde(default)]
    pub status: StepStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

impl Step {
    pub fn new(id: impl Into<String>, title: impl Into<String>, kind: StepKind) -> Self {
        Self {
            id: StepId::new(id),
            title: title.into(),
            kind,
            status: StepStatus::Pending,
            tags: Vec::new(),
        }
    }
}

#[derive(Debug, Error)]
pub enum DagError {
    #[error("duplicate step id `{0}`")]
    Duplicate(String),
    #[error("unknown step id `{0}`")]
    Unknown(String),
    #[error("plan has a cycle involving step `{0}`")]
    Cycle(String),
}

/// A directed acyclic graph of [`Step`]s.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Dag {
    pub steps: BTreeMap<StepId, Step>,
    /// Adjacency list: `edges[from]` = set of steps that depend on `from`.
    pub edges: BTreeMap<StepId, BTreeSet<StepId>>,
}

impl Dag {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, step: Step) -> Result<(), DagError> {
        if self.steps.contains_key(&step.id) {
            return Err(DagError::Duplicate(step.id.0.clone()));
        }
        let id = step.id.clone();
        self.steps.insert(id.clone(), step);
        self.edges.entry(id).or_default();
        Ok(())
    }

    /// Add a dependency edge `from → to` (meaning `to` runs after `from`).
    pub fn edge(&mut self, from: &StepId, to: &StepId) -> Result<(), DagError> {
        if !self.steps.contains_key(from) {
            return Err(DagError::Unknown(from.0.clone()));
        }
        if !self.steps.contains_key(to) {
            return Err(DagError::Unknown(to.0.clone()));
        }
        self.edges.entry(from.clone()).or_default().insert(to.clone());
        Ok(())
    }

    /// Kahn's algorithm. Deterministic because we iterate `BTreeSet`s.
    pub fn topo(&self) -> Result<Vec<StepId>, DagError> {
        let mut indegree: BTreeMap<StepId, usize> =
            self.steps.keys().map(|id| (id.clone(), 0)).collect();
        for children in self.edges.values() {
            for child in children {
                if let Some(v) = indegree.get_mut(child) {
                    *v += 1;
                }
            }
        }

        let mut ready: VecDeque<StepId> = indegree
            .iter()
            .filter_map(|(id, &d)| if d == 0 { Some(id.clone()) } else { None })
            .collect();

        let mut out = Vec::with_capacity(self.steps.len());
        while let Some(id) = ready.pop_front() {
            out.push(id.clone());
            if let Some(children) = self.edges.get(&id) {
                for child in children {
                    let d = indegree.get_mut(child).expect("child in graph");
                    *d -= 1;
                    if *d == 0 {
                        ready.push_back(child.clone());
                    }
                }
            }
        }

        if out.len() != self.steps.len() {
            let offender = indegree
                .into_iter()
                .find(|(_, d)| *d > 0)
                .map(|(id, _)| id.0)
                .unwrap_or_else(|| "?".into());
            return Err(DagError::Cycle(offender));
        }
        Ok(out)
    }
}

/// A [`Plan`] is an [`Objective`] + a [`Dag`] of steps.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Plan {
    pub objective: Objective,
    pub dag: Dag,
}

impl Plan {
    pub fn new(objective: Objective) -> Self {
        Self { objective, dag: Dag::new() }
    }

    pub fn topo(&self) -> Result<Vec<&Step>, DagError> {
        let ids = self.dag.topo()?;
        Ok(ids.into_iter().map(|id| &self.dag.steps[&id]).collect())
    }
}
