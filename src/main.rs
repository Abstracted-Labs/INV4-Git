#![warn(clippy::pedantic)]
#![allow(clippy::unnecessary_wraps)] // Allowed while TODOs in functions exists
use std::{
    env::{args, current_dir, var},
    fmt::format,
    fs::create_dir_all,
    io::stdin,
    io::Write,
    path::{Path, PathBuf},
};

use client::GitArch;
use primitives::{BoxResult, Key, Settings};

mod client;
mod primitives;
mod util;

#[tokio::main]
async fn main() -> BoxResult<()> {
    let (client, alias, raw_url) = {
        let mut args = args();
        args.next();
        (
            GitArch::new().await?,
            args.next().ok_or("Missing alias argument.")?,
            args.next().ok_or("Missing url argument.")?,
        )
    };

    let (ips_id, subasset_id) = {
        let mut url = Path::new(&raw_url).components();
        url.next();
        (
            url.next()
                .ok_or("Missing IPS id. Expected: 'gitarch://>ips_id<'")?
                .as_os_str()
                .to_str()
                .ok_or("Input was not UTF-8")?
                .parse::<u32>()?,
            if let Some(component) = url.next() {
                Some(
                    component
                        .as_os_str()
                        .to_str()
                        .ok_or("Input was not UTF-8")?
                        .parse::<u32>()?,
                )
            } else {
                None
            },
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
            ips_id,
            subasset_id,
        },
    };

    loop {
        let mut input = String::new();
        stdin().read_line(&mut input)?;

        if input.is_empty() {
            return Ok(());
        }

        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .append(true)
            .create(true) // This is needed to append to file
            .open("log")
            .unwrap();
        file.write_all(&input.clone().into_bytes()).unwrap();

        let mut args = input.split_ascii_whitespace();

        match (args.next(), args.next(), args.next()) {
            (Some("push"), Some(ref_arg), None) => push(&client, &settings, ref_arg).await,
            (Some("fetch"), Some(sha), Some(name)) => fetch(&client, &settings, sha, name).await,
            (Some("capabilities"), None, None) => capabilities(),
            (Some("list"), Some(path), None) => list(&client, &settings, &path).await,
            (None, None, None) => Ok(()),
            _ => {
                println!("unknown command\n");
                Ok(())
            }
        }?;
    }
}

async fn push(client: &GitArch, settings: &Settings, ref_arg: &str) -> BoxResult<()> {
    let mut ref_args = ref_arg.split(':');

    let src_ref = if ref_arg.starts_with('+') {
        &ref_args
            .next()
            .ok_or("Unexpected error while parsing refs")?[1..]
    } else {
        ref_args
            .next()
            .ok_or("Unexpected error while parsing refs")?
    };

    let dst_ref = ref_args
        .next()
        .ok_or("Unexpected error while parsing refs")?;
    if src_ref != dst_ref {
        return Err("src_ref != dst_ref".into());
    }

    client.push(settings).await?;
    println!("ok {dst_ref}\n");
    Ok(())
}

async fn fetch(client: &GitArch, settings: &Settings, _sha: &str, name: &str) -> BoxResult<()> {
    if name == "HEAD" {
        return Ok(());
    }

    client.fetch(settings).await?;
    println!();
    Ok(())
}

fn capabilities() -> BoxResult<()> {
    println!("push");
    println!("fetch\n");
    Ok(())
}

async fn list(client: &GitArch, settings: &Settings, path: &str) -> BoxResult<()> {
    println!("{}", client.list(settings, path).await?);
    println!();
    Ok(())
}
