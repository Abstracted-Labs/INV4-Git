use std::{error::Error, path::PathBuf};

use codec::{Decode, Encode};
use git2::ReferenceNames;

#[derive(Debug)]
pub struct Settings {
    pub git_dir: PathBuf,
    pub remote_alias: String,
    pub root: Key,
}

#[derive(Debug)]
pub struct Key {
    pub ips_id: u32,
    pub subasset_id: Option<u32>,
}

pub type BoxResult<T> = Result<T, Box<dyn Error>>;

#[derive(Encode, Decode)]
pub struct RefsFile {
    pub refs: Vec<String>,
}

impl RefsFile {
    pub fn get_latest_ref(&self) -> String {
        self.refs.first().unwrap().to_string()
    }

    pub fn from_reference_names(
        reference_names: ReferenceNames<'_, '_>,
        repo: &git2::Repository,
    ) -> Self {
        Self {
            refs: reference_names
                .map(|ref_name| {
                    ref_name.map(|s| repo.refname_to_id(s).map(|oid| oid.to_string()).unwrap())
                })
                .collect::<Result<Vec<String>, git2::Error>>()
                .unwrap(),
        }
    }
}
