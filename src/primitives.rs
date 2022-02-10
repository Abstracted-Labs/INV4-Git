use std::path::PathBuf;

#[derive(Debug)]
pub struct Settings {
    pub git_dir: PathBuf,
    pub remote_alias: String,
    pub remote_url: String,
    pub root: Key,
}

#[derive(Debug)]
pub struct Key {
    pub account_id: String,
    pub ips_id: String,
}

#[derive(Debug)]
pub struct GitRef {
    pub name: String,
    pub sha: String,
}

impl GitRef {
    #[allow(dead_code)]
    fn bundle_path(&self, root: String) -> String {
        let mut path = String::new();

        path.push_str(&format!("{}/{}/{}.bundle", root, self.name, self.sha));
        path
    }
}
