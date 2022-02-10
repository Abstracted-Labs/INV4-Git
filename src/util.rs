#![allow(dead_code)]
use std::{path::Path, process::Command};

use crate::primitives::Error;

pub fn create_bundle(bundle: &Path, ref_name: &str) -> Result<(), Error> {
    let cmd = Command::new("git")
        .args([
            "bundle",
            "create",
            bundle
                .to_str()
                .ok_or_else(|| Error::Path(String::from("Invalid bundle path")))?,
            ref_name,
        ])
        .output()
        .map_err(Error::Command)?;
    if !cmd.status.success() {
        Err(Error::Custom(String::from("Git bundle failed")))
    } else {
        Ok(())
    }
}

pub fn unbundle(bundle: &Path, ref_name: &str) -> Result<(), Error> {
    let cmd = Command::new("git")
        .args([
            "bundle",
            "create",
            bundle
                .to_str()
                .ok_or_else(|| Error::Path(String::from("Invalid bundle path")))?,
            ref_name,
        ])
        .output()
        .map_err(Error::Command)?;
    if !cmd.status.success() {
        Err(Error::Custom(String::from("Git unbundle failed")))
    } else {
        Ok(())
    }
}

pub fn is_ancestor(base_ref: &str, remote_ref: &str) -> Result<bool, Error> {
    let cmd = Command::new("git")
        .args(["merge-base", "--is-ancestor", remote_ref, base_ref])
        .output()
        .map_err(Error::Command)?;
    Ok(cmd.status.success())
}

pub fn config(setting: &str) -> Result<String, Error> {
    let cmd = Command::new("git")
        .args(["config", setting])
        .output()
        .map_err(Error::Command)?;
    if !cmd.status.success() {
        Err(Error::Custom(String::from("Git config failed")))
    } else {
        Ok(String::from_utf8(cmd.stdout)?.trim().to_owned())
    }
}

pub fn rev_parse(rev: &str) -> Result<String, Error> {
    let cmd = Command::new("git")
        .args(["rev-parse", rev])
        .output()
        .map_err(Error::Command)?;
    if !cmd.status.success() {
        Err(Error::Custom(String::from("Git rev-parse failed")))
    } else {
        Ok(String::from_utf8(cmd.stdout)?.trim().to_owned())
    }
}
