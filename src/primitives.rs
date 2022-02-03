use std::{env::VarError, io::Error as IOError, path::PathBuf};

#[derive(Debug)]
pub enum Error {
    Args(String),
    Url(String),
    Var(VarError),
    IO(IOError),
}

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
