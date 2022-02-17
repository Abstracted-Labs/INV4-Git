use std::path::PathBuf;

use subxt::sp_runtime::AccountId32;

#[derive(Debug)]
pub struct Settings {
    pub git_dir: PathBuf,
    pub remote_alias: String,
    pub root: Key,
}

#[derive(Debug)]
pub struct Key {
    pub account_id: AccountId32,
    pub ips_id: String,
}

#[derive(Debug)]
pub struct GitRef {
    pub name: String,
    pub sha: String,
}

impl GitRef {
    fn _bundle_path(&self, root: String) -> PathBuf {
        PathBuf::from(root).join(&self.name).join(&self.sha)
    }
}
