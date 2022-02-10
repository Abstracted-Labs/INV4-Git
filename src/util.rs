#![allow(dead_code)]
use std::{error::Error, path::Path, process::Command};

use crate::error::ErrorWrap;

pub fn create_bundle(bundle: &Path, ref_name: &str) -> Result<(), Box<dyn Error>> {
    let cmd = Command::new("git")
        .args([
            "bundle",
            "create",
            bundle.to_str().ok_or(ErrorWrap("Invalid bundle path"))?,
            ref_name,
        ])
        .output()?;
    if !cmd.status.success() {
        Err(Box::new(ErrorWrap("Git bundle failed")))
    } else {
        Ok(())
    }
}

pub fn unbundle(bundle: &Path, ref_name: &str) -> Result<(), Box<dyn Error>> {
    let cmd = Command::new("git")
        .args([
            "bundle",
            "create",
            bundle.to_str().ok_or(ErrorWrap("Invalid bundle path"))?,
            ref_name,
        ])
        .output()?;
    if !cmd.status.success() {
        Err(Box::new(ErrorWrap("Git unbundle failed")))
    } else {
        Ok(())
    }
}

pub fn is_ancestor(base_ref: &str, remote_ref: &str) -> Result<bool, Box<dyn Error>> {
    let cmd = Command::new("git")
        .args(["merge-base", "--is-ancestor", remote_ref, base_ref])
        .output()?;
    Ok(cmd.status.success())
}

pub fn config(setting: &str) -> Result<String, Box<dyn Error>> {
    let cmd = Command::new("git").args(["config", setting]).output()?;
    if !cmd.status.success() {
        Err(Box::new(ErrorWrap("Git config failed")))
    } else {
        Ok(String::from_utf8(cmd.stdout)?.trim().to_owned())
    }
}

pub fn rev_parse(rev: &str) -> Result<String, Box<dyn Error>> {
    let cmd = Command::new("git").args(["rev-parse", rev]).output()?;
    if !cmd.status.success() {
        Err(Box::new(ErrorWrap("Git rev-parse failed")))
    } else {
        Ok(String::from_utf8(cmd.stdout)?.trim().to_owned())
    }
}
