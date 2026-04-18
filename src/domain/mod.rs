//! Bounded contexts for the agents-unified **installation domain**.
//!
//! | Context | Responsibility |
//! |---------|----------------|
//! | [`AgentsHome`](crate::domain::AgentsHome) | Canonical store layout under `~/.agents` (or override). |
//! | [`ProjectWorkspace`](crate::domain::ProjectWorkspace) | A managed repository root receiving links. |
//! | `install::*` | Planning (pure + IO scan) and application (filesystem) phases. |
//! | [`crate::vocabulary`] | schema.org JSON-LD for audit and interoperability. |

mod paths;

pub use paths::{AgentsHome, ProjectWorkspace};
