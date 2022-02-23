use std::{
    env::{args, current_dir, var},
    fs::create_dir_all,
    io::stdin,
    path::{Path, PathBuf},
    str::FromStr,
};

use client::GitArchClient;
use error::ErrorWrap;
use primitives::{BoxResult, GitRef, Key, Settings};
use subxt::sp_runtime::AccountId32;

mod client;
mod error;
mod primitives;
mod util;

fn main() -> BoxResult<()> {
    let (client, alias, raw_url) = {
        let mut args = args();
        args.next();
        (
            GitArchClient::default(),
            args.next().ok_or(ErrorWrap("Missing alias argument."))?,
            args.next().ok_or(ErrorWrap("Missing url argument."))?,
        )
    };

    let (account_id, ips_id) = {
        let mut url = Path::new(&raw_url).components();
        url.next();
        (
            AccountId32::from_str(
                url.next()
                    .ok_or(ErrorWrap(
                        "Missing Account ID. Expected 'gitarch://>account_id</ips_id'",
                    ))?
                    .as_os_str()
                    .to_str()
                    .ok_or(ErrorWrap(
                        "Missing account id. Expected: 'gitarch://>account_id</ips_id'",
                    ))?,
            )?,
            url.next()
                .ok_or(ErrorWrap(
                    "Missing IPS id. Expectec: 'gitarch://account_id/>ips_id<'",
                ))?
                .as_os_str()
                .to_str()
                .ok_or(ErrorWrap("Input was not UTF-8"))?,
        )
    };

    let git_dir = PathBuf::from(var("GIT_DIR")?);
    create_dir_all(
        current_dir()?
            .join(&git_dir)
            .join("remote-gitarch")
            .join(&alias),
    )?;

    let settings = Settings {
        git_dir,
        remote_alias: alias,
        root: Key {
            account_id,
            ips_id: String::from(ips_id),
        },
    };

    loop {
        let mut input = String::new();
        stdin().read_line(&mut input)?;

        if input.is_empty() {
            return Ok(());
        }

        let mut args = input.split_ascii_whitespace();

        match (args.next(), args.next(), args.next()) {
            (Some("push"), Some(ref_arg), None) => push(&client, &settings, ref_arg),
            (Some("fetch"), Some(sha), Some(name)) => fetch(&client, &settings, sha, name),
            (Some("capabilities"), None, None) => capabilities(),
            (Some("list"), None, None) => list(&client, &settings),
            (Some("list"), Some("for-push"), None) => list(&client, &settings),
            (None, None, None) => Ok(()),
            _ => {
                println!("unknown command\n");
                Ok(())
            }
        }?
    }
}

fn push(_client: &GitArchClient, _settings: &Settings, ref_arg: &str) -> BoxResult<()> {
    let mut ref_args = ref_arg.split(':');

    let src_ref = if ref_arg.starts_with('+') {
        &ref_args
            .next()
            .ok_or(ErrorWrap("Unexpected error while parsing refs"))?[1..]
    } else {
        ref_args
            .next()
            .ok_or(ErrorWrap("Unexpected error while parsing refs"))?
    };

    let dst_ref = ref_args
        .next()
        .ok_or(ErrorWrap("Unexpected error while parsing refs"))?;
    if src_ref != dst_ref {
        return Err(ErrorWrap("src_ref != dst_ref").into());
    }

    // TODO: push refs to remote
    println!("ok {}\n", dst_ref);
    Ok(())
}

fn fetch(_client: &GitArchClient, _settings: &Settings, sha: &str, name: &str) -> BoxResult<()> {
    if name == "HEAD" {
        return Ok(());
    }
    let _git_ref = GitRef {
        name: String::from(name),
        sha: String::from(sha),
    };
    println!();

    // TODO: fetch from remote
    Ok(())
}

fn capabilities() -> BoxResult<()> {
    println!("push");
    println!("list\n");
    Ok(())
}

fn list(_client: &GitArchClient, _settings: &Settings) -> BoxResult<()> {
    // TODO: fetch refs from remote
    println!();
    Ok(())
}
