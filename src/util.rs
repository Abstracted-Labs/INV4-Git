use std::{path::Path, process::Command};

use crate::primitives::BoxResult;

pub fn _create_bundle(bundle: &Path, ref_name: &str) -> BoxResult<()> {
    let cmd = Command::new("git")
        .args([
            "bundle",
            "create",
            bundle.to_str().ok_or("Invalid bundle path")?,
            ref_name,
        ])
        .output()?;
    if cmd.status.success() {
        Ok(())
    } else {
        Err("Git bundle failed".into())
    }
}

pub fn _unbundle(bundle: &Path, ref_name: &str) -> BoxResult<()> {
    let cmd = Command::new("git")
        .args([
            "bundle",
            "create",
            bundle.to_str().ok_or("Invalid bundle path")?,
            ref_name,
        ])
        .output()?;
    if cmd.status.success() {
        Ok(())
    } else {
        Err("Git unbundle failed".into())
    }
}

pub fn _is_ancestor(base_ref: &str, remote_ref: &str) -> BoxResult<bool> {
    let cmd = Command::new("git")
        .args(["merge-base", "--is-ancestor", remote_ref, base_ref])
        .output()?;
    Ok(cmd.status.success())
}

pub fn _config(setting: &str) -> BoxResult<String> {
    let cmd = Command::new("git").args(["config", setting]).output()?;
    if cmd.status.success() {
        Ok(String::from_utf8(cmd.stdout)?.trim().to_owned())
    } else {
        Err("Git config failed".into())
    }
}

pub fn _rev_parse(rev: &str) -> BoxResult<String> {
    let cmd = Command::new("git").args(["rev-parse", rev]).output()?;
    if cmd.status.success() {
        Ok(String::from_utf8(cmd.stdout)?.trim().to_owned())
    } else {
        Err("Git rev-parse failed".into())
    }
}
