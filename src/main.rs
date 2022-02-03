use std::{
    env::{args, current_dir, var},
    fs::create_dir_all,
    io::stdin,
    path::PathBuf,
};

use client::GitArchClient;
use primitives::{Error, Key, Settings};

mod client;
mod primitives;

fn main() -> Result<(), Error> {
    let mut args = args();
    args.next();

    let client = GitArchClient {};
    let alias = args
        .next()
        .ok_or_else(|| Error::Args(String::from("missing alias argument")))?;
    let url = args
        .next()
        .ok_or_else(|| Error::Args(String::from("missing url argument")))?;

    let (account_id, ips_id): (Result<&str, Error>, Result<&str, Error>) = {
        if !url.starts_with("gitarch://") {
            (
                Err(Error::Url(String::from(
                    "Invalid url format. expected 'gitarch://publickey/ips'",
                ))),
                Ok(""),
            )
        } else {
            let url = &url[9..];
            let slash = match url.find('/') {
                Some(index) => Ok(index),
                None => Err(Error::Url(String::from(
                    "Url does not have a prefix. Expected 'gitarch://publickey/ips",
                ))),
            }?;
            let account_id = url.get(..slash).ok_or_else(|| {
                Error::Url(String::from(
                    "An exception ocurred while parsing the account_id",
                ))
            })?;
            let end = if url.ends_with('/') {
                url.len() - 1
            } else {
                url.len()
            };
            let ips_id = url.get((slash + 1)..end).ok_or_else(|| {
                Error::Url(String::from(
                    "An exception ocurred while parsing the ips_id",
                ))
            })?;
            (Ok(account_id), Ok(ips_id))
        }
    };

    let git_dir = PathBuf::from(var("GIT_DIR").map_err(Error::Var)?);
    let current_dir = current_dir().map_err(Error::IO)?;
    let working_dir = current_dir
        .join(&git_dir)
        .join("remote-gitarch")
        .join(&alias);

    create_dir_all(working_dir).map_err(Error::IO)?;

    let settings = Settings {
        git_dir,
        remote_url: url.to_owned(),
        remote_alias: alias,
        root: Key {
            account_id: String::from(account_id?),
            ips_id: String::from(ips_id?),
        },
    };

    loop {
        let mut input = String::new();
        stdin().read_line(&mut input).map_err(Error::IO)?;

        if input.is_empty() {
            return Ok(());
        }

        let mut args = input.split_ascii_whitespace();
        let cmd = args.next();
        let arg1 = args.next();
        let arg2 = args.next();

        match (cmd, arg1, arg2) {
            (Some("push"), Some(ref_arg), None) => push(&client, &settings, ref_arg),
            (Some("fetch"), Some(sha), Some(name)) => fetch(&client, &settings, sha, name),
            (Some("capabilities"), None, None) => capabilities(),
            (Some("list"), None, None) => list(&client, &settings),
            (Some("list"), Some("for-push"), None) => list(&client, &settings),
            (None, None, None) => return Ok(()),
            _ => {
                println!("Unknown command.\n");
                Ok(())
            }
        }?
    }
}

#[allow(unused_variables)]
fn push(client: &GitArchClient, settings: &Settings, ref_arg: &str) -> Result<(), Error> {
    todo!()
}

#[allow(unused_variables)]
fn fetch(client: &GitArchClient, settings: &Settings, sha: &str, name: &str) -> Result<(), Error> {
    todo!()
}

fn capabilities() -> Result<(), Error> {
    todo!()
}

#[allow(unused_variables)]
fn list(client: &GitArchClient, settings: &Settings) -> Result<(), Error> {
    todo!()
}
