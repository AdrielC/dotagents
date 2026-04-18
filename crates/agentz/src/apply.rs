//! Apply a pure [`CompiledPlan`] by walking its [`FsOp`] values. This is the only IO the
//! installer performs — all planning happens in `agentz-core::compile`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::symlink;

use agentz_core::compile::{CompiledPlan, FsOp};
use agentz_core::model::{LinkKind, PlannedLink};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Debug, Default)]
pub struct ApplyOptions {
    pub force: bool,
    pub dry_run: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ApplyReport {
    pub applied: Vec<PlannedLink>,
    pub wrote: Vec<PathBuf>,
    pub skipped: Vec<String>,
    pub warnings: Vec<String>,
    /// schema.org JSON-LD for audit, if requested by the caller.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_org_json_ld: Option<serde_json::Value>,
}

#[derive(Debug, Error)]
pub enum ApplyError {
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("refusing to replace existing path without force: {0}")]
    Exists(PathBuf),
    #[error("source missing: {0}")]
    MissingSource(PathBuf),
    #[error("symlinks are only supported on unix targets in this build")]
    SymlinkNotSupported,
}

pub fn apply_plan(plan: &CompiledPlan, opts: &ApplyOptions) -> Result<ApplyReport, ApplyError> {
    let mut report = ApplyReport::default();
    for op in &plan.ops {
        if opts.dry_run {
            report.skipped.push(format!("dry-run: {:?}", op));
            continue;
        }
        match op {
            FsOp::MkdirP { path } => fs::create_dir_all(path)?,
            FsOp::WriteFile { path, overwrite, content } => {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                if path.exists() && !overwrite && !opts.force {
                    report.skipped.push(format!("exists: {}", path.display()));
                    continue;
                }
                fs::write(path, content)?;
                report.wrote.push(path.clone());
            }
            FsOp::Link(link) => match link.kind {
                LinkKind::HardLink => apply_hardlink(link, opts.force, &mut report)?,
                LinkKind::Symlink => apply_symlink(link, opts.force, &mut report)?,
                LinkKind::Copy => apply_copy(link, opts.force, &mut report)?,
            },
        }
    }
    Ok(report)
}

fn apply_copy(link: &PlannedLink, force: bool, report: &mut ApplyReport) -> Result<(), ApplyError> {
    if !link.source.is_file() {
        return Err(ApplyError::MissingSource(link.source.clone()));
    }
    if let Some(parent) = link.dest.parent() {
        fs::create_dir_all(parent)?;
    }
    if link.dest.exists() && !force {
        return Err(ApplyError::Exists(link.dest.clone()));
    }
    fs::copy(&link.source, &link.dest)?;
    report.applied.push(link.clone());
    Ok(())
}

fn apply_hardlink(link: &PlannedLink, force: bool, report: &mut ApplyReport) -> Result<(), ApplyError> {
    if !link.source.is_file() {
        return Err(ApplyError::MissingSource(link.source.clone()));
    }
    if let Some(parent) = link.dest.parent() {
        fs::create_dir_all(parent)?;
    }
    if let Ok(meta) = fs::symlink_metadata(&link.dest) {
        if meta.file_type().is_symlink() {
            if !force {
                return Err(ApplyError::Exists(link.dest.clone()));
            }
            fs::remove_file(&link.dest)?;
        } else if link.dest.is_file() {
            if same_file(&link.source, &link.dest)? {
                report.applied.push(link.clone());
                return Ok(());
            }
            if !force {
                return Err(ApplyError::Exists(link.dest.clone()));
            }
            fs::remove_file(&link.dest)?;
        } else if !force {
            return Err(ApplyError::Exists(link.dest.clone()));
        } else {
            fs::remove_dir_all(&link.dest)?;
        }
    }
    fs::hard_link(&link.source, &link.dest)?;
    report.applied.push(link.clone());
    Ok(())
}

fn same_file(a: &Path, b: &Path) -> io::Result<bool> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let ma = fs::metadata(a)?;
        let mb = fs::metadata(b)?;
        Ok(ma.dev() == mb.dev() && ma.ino() == mb.ino())
    }
    #[cfg(not(unix))]
    {
        let _ = (a, b);
        Ok(false)
    }
}

fn apply_symlink(link: &PlannedLink, force: bool, report: &mut ApplyReport) -> Result<(), ApplyError> {
    #[cfg(not(unix))]
    {
        let _ = (link, force, report);
        return Err(ApplyError::SymlinkNotSupported);
    }
    #[cfg(unix)]
    {
        if !link.source.exists() {
            return Err(ApplyError::MissingSource(link.source.clone()));
        }
        if let Some(parent) = link.dest.parent() {
            fs::create_dir_all(parent)?;
        }
        let target = compute_symlink_target(&link.source, &link.dest)?;
        if link.dest.symlink_metadata().is_ok() {
            if link.dest.is_symlink() {
                if let Ok(cur) = fs::read_link(&link.dest) {
                    if cur == target {
                        report.applied.push(link.clone());
                        return Ok(());
                    }
                }
            }
            if !force {
                return Err(ApplyError::Exists(link.dest.clone()));
            }
            if link.dest.is_dir() {
                fs::remove_dir_all(&link.dest)?;
            } else {
                fs::remove_file(&link.dest)?;
            }
        }
        symlink(&target, &link.dest)?;
        report.applied.push(link.clone());
        Ok(())
    }
}

fn compute_symlink_target(source: &Path, dest: &Path) -> io::Result<PathBuf> {
    let dest_dir = dest.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "symlink dest has no parent")
    })?;
    pathdiff::diff_paths(source, dest_dir).ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "could not compute relative symlink target")
    })
}
