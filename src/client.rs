#![allow(clippy::too_many_arguments, clippy::enum_variant_names)]
use std::{
    fs::{write, File},
    io::Write,
    path::{Path, PathBuf},
    process::exit,
    str::FromStr,
};

use cid::Cid;

use subxt::subxt;

#[subxt(runtime_metadata_path = "invarch_metadata.scale")]
pub mod invarch {}
