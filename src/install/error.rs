use std::io;
use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum InstallError {
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("plugin schema: {0}")]
    PluginSchema(#[from] crate::schema::SchemaError),
    #[error("symlinks are only supported on unix targets in this build")]
    SymlinkNotSupported,
    #[error("refusing to replace existing path without force: {0}")]
    Exists(PathBuf),
    #[error("source missing: {0}")]
    MissingSource(PathBuf),
}
