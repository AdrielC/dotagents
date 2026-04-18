//! Apply phase: materialize [`PlannedLink`](crate::model::PlannedLink) on disk.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::symlink;

use crate::model::{LinkKind, PlannedLink};

use super::error::InstallError;
use super::types::{InstallOptions, InstallReport};

pub fn apply_planned(report: &mut InstallReport, opts: &InstallOptions) -> Result<(), InstallError> {
    let planned: Vec<_> = report.planned.clone();
    for link in planned {
        if opts.dry_run {
            report.skipped.push(format!("dry-run: {:?}", link.dest));
            continue;
        }
        match link.kind {
            LinkKind::HardLink => apply_hardlink(&link, opts.force, report)?,
            LinkKind::Symlink => apply_symlink(&link, opts.force, report)?,
        }
    }
    Ok(())
}

fn apply_hardlink(link: &PlannedLink, force: bool, report: &mut InstallReport) -> Result<(), InstallError> {
    if !link.source.is_file() {
        return Err(InstallError::MissingSource(link.source.clone()));
    }
    if let Some(parent) = link.dest.parent() {
        fs::create_dir_all(parent)?;
    }

    if let Ok(meta) = fs::symlink_metadata(&link.dest) {
        if meta.file_type().is_symlink() {
            if !force {
                return Err(InstallError::Exists(link.dest.clone()));
            }
            fs::remove_file(&link.dest)?;
        } else if link.dest.is_file() {
            if same_file(&link.source, &link.dest)? {
                report.applied.push(link.clone());
                return Ok(());
            }
            if !force {
                return Err(InstallError::Exists(link.dest.clone()));
            }
            fs::remove_file(&link.dest)?;
        } else if !force {
            return Err(InstallError::Exists(link.dest.clone()));
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

fn apply_symlink(link: &PlannedLink, force: bool, report: &mut InstallReport) -> Result<(), InstallError> {
    #[cfg(not(unix))]
    {
        let _ = (link, force, report);
        return Err(InstallError::SymlinkNotSupported);
    }
    #[cfg(unix)]
    {
        if !link.source.exists() {
            return Err(InstallError::MissingSource(link.source.clone()));
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
                return Err(InstallError::Exists(link.dest.clone()));
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
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "could not compute relative symlink target",
        )
    })
}
