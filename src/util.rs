use cid::{multihash::MultihashGeneric, CidGeneric};
use std::{path::Path, process::Command};
use subxt::sp_core::H256;

use crate::primitives::BoxResult;

#[macro_export]
macro_rules! error {
    ($x:expr) => {{
        return Err($x.into());
    }};
}

pub fn create_bundle(bundle: &Path) -> BoxResult<()> {
    let cmd = Command::new("git")
        .args([
            "bundle",
            "create",
            bundle.to_str().ok_or("Invalid bundle path")?,
            "--all",
        ])
        .output()?;
    if cmd.status.success() {
        Ok(())
    } else {
        Err("Git bundle failed".into())
    }
}

pub fn pull_from_bundle(dir: &Path) -> BoxResult<()> {
    let cmd = Command::new("git")
        .current_dir(dir)
        .args(["pull", "gitarch.bundle"])
        .output()?;

    if cmd.status.success() {
        Ok(())
    } else {
        error!("Pull from bundle failed")
    }
}

pub fn show_ref(dir: &Path) -> BoxResult<String> {
    let cmd = Command::new("git")
        .current_dir(dir)
        .arg("show-ref")
        .output()?;

    if cmd.status.success() {
        Ok(String::from_utf8(cmd.stdout)?)
    } else {
        error!("git show-ref failed")
    }
}

pub fn generate_cid(hash: H256) -> BoxResult<CidGeneric<32>> {
    Ok(CidGeneric::new_v0(MultihashGeneric::<32>::from_bytes(
        hex::decode(format!("{:?}", hash).replace("0x", "1220"))?.as_slice(),
    )?)?)
}
