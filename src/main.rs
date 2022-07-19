#![allow(clippy::too_many_arguments)]

use dirs::config_dir;
use git2::Repository;
use ipfs_api::IpfsClient;
use log::debug;
use primitives::{BoxResult, Chain, Config, INV4AnyId, NFTInfo, RepoData};
use std::{
    env::args,
    io::{self, Read, Write},
    path::Path,
    process::Stdio,
};
use subxt::sp_core::Pair;
use subxt::subxt;
use subxt::{DefaultConfig, PairSigner};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

mod primitives;

#[subxt(runtime_metadata_path = "invarch_metadata.scale")]
pub mod tinkernet {}

pub async fn set_repo(ips_id: u32, chain: &Chain) -> BoxResult<RepoData> {
    let mut ipfs_client = IpfsClient::default();
    let data = chain.ips_data(ips_id).await?;

    eprintln!("past data");

    for file in data {
        if let Some(nft_id) = file.nft_id() {
            let nft_info = chain.nft_info(&nft_id).await?;
            if String::from_utf8(nft_info.metadata())? == *"RepoData" {
                return RepoData::from_ipfs(nft_info.data()?, &mut ipfs_client).await;
            }
        }
    }

    Ok(RepoData {
        refs: Default::default(),
        objects: Default::default(),
    })
}

#[tokio::main]
async fn main() -> BoxResult<()> {
    let raw_url = {
        let mut args = args();
        args.next();
        args.next();
        args.next().ok_or("Missing url argument.")?
    };

    eprintln!("past raw url");

    let (chain_string, ips_id, subasset_id) = {
        let mut url = Path::new(&raw_url).components();
        url.next();
        (
            url.next()
                .ok_or("Missing chain. Expected: 'inv4://>chain</ips_id'")?
                .as_os_str()
                .to_str()
                .ok_or("Input was not UTF-8")?
                .parse::<String>()?,
            url.next()
                .ok_or("Missing IPS id. Expected: 'inv4://chain/>ips_id<'")?
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

    eprintln!("past chain string");

    let mut config_file_path =
        config_dir().expect("Operating system's configs directory not found");
    config_file_path.push("INV4-Git/config.toml");

    std::fs::create_dir_all(config_file_path.parent().unwrap()).unwrap();

    let config: Config = if config_file_path.exists() {
        let mut contents = String::new();
        std::fs::File::options()
            .write(true)
            .read(true)
            .create(false)
            .open(config_file_path.clone())?
            .read_to_string(&mut contents)?;

        toml::from_str(&contents)?
    } else {
        let c = Config {
            chain_endpoint: String::from("ws://127.0.0.1:9944"),
        };

        let mut f = std::fs::File::create(config_file_path)?;

        f.write_all(toml::to_string(&c)?.as_bytes())?;

        c
    };

    let chain = Chain::from_str(chain_string, config).await?;

    eprintln!("past chain");

    let mut remote_repo = set_repo(ips_id, &chain).await?;
    debug!("RepoData: {:#?}", remote_repo);

    eprintln!("past remote repo");

    loop {
        let repo = Repository::open_from_env().unwrap();

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        if input.is_empty() {
            return Ok(());
        }

        eprintln!("input: {}", &input.clone());

        debug!("{}", &input.clone());

        let mut args = input.split_ascii_whitespace();

        match (args.next(), args.next(), args.next()) {
            (Some("push"), Some(ref_arg), None) => {
                push(
                    &chain,
                    &mut remote_repo,
                    ips_id,
                    subasset_id,
                    repo,
                    IpfsClient::default(),
                    ref_arg,
                )
                .await
            }
            (Some("fetch"), Some(sha), Some(name)) => {
                fetch(
                    &remote_repo,
                    &chain,
                    ips_id,
                    repo,
                    IpfsClient::default(),
                    sha,
                    name,
                )
                .await
            }
            (Some("capabilities"), None, None) => capabilities(),
            (Some("list"), _, None) => list(&remote_repo),
            (None, None, None) => Ok(()),
            _ => {
                eprintln!("unknown command\n");
                Ok(())
            }
        }?;
    }
}

async fn push(
    chain: &Chain,
    remote_repo: &mut RepoData,
    ips_id: u32,
    subasset_id: Option<u32>,
    mut repo: Repository,
    mut ipfs: IpfsClient,
    ref_arg: &str,
) -> BoxResult<()> {
    let mut cmd = Command::new("git");
    cmd.arg("credential");
    cmd.arg("fill");
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::null());

    let mut child = cmd.spawn().expect("failed to spawn command");

    let stdout = child
        .stdout
        .take()
        .expect("child did not have a handle to stdout");

    let mut stdin = child
        .stdin
        .take()
        .expect("child did not have a handle to stdin");

    let mut out_reader = BufReader::new(stdout).lines();

    tokio::spawn(async move {
        child
            .wait()
            .await
            .expect("child process encountered an error");
    });

    stdin
        .write_all("protocol=inv4\nhost=\nusername= \n\n".as_bytes())
        .await
        .expect("could not write to stdin");

    eprintln!("Seed Phrase or Private Key ↓");

    drop(stdin);

    let mut credential = String::new();

    while let Some(line) = out_reader.next_line().await? {
        if line.trim().starts_with("password=") {
            credential = line.trim_start_matches("password=").to_string();
        }
    }

    if credential.is_empty() {
        return Err("No credential".into());
    }

    let signer = &PairSigner::<DefaultConfig, sp_keyring::sr25519::sr25519::Pair>::new(
        sp_keyring::sr25519::sr25519::Pair::from_string(&credential, None).unwrap(),
    );

    // Separate source, destination and the force flag
    let mut refspec_iter = ref_arg.split(':');

    let first_half = refspec_iter
        .next()
        .ok_or_else(|| eprintln!("Could not read source ref from refspec: {:?}", ref_arg))
        .unwrap();

    let force = first_half.starts_with('+');

    let src = if force {
        eprintln!("THIS PUSH WILL BE FORCED");
        &first_half[1..]
    } else {
        first_half
    };

    let dst = refspec_iter
        .next()
        .ok_or_else(|| eprintln!("Could not read destination ref from refspec: {:?}", ref_arg))
        .unwrap();

    // Upload the object tree
    match remote_repo
        .push_ref_from_str(src, dst, force, &mut repo, &mut ipfs, chain, signer, ips_id)
        .await
    {
        Ok(multi_object_id) => {
            let (new_repo_data_id, old_repo_data_id) = remote_repo
                .mint_return_new_old_id(&mut ipfs, chain, signer, ips_id)
                .await?;

            if let Some(old_id) = old_repo_data_id {
                eprintln!("Removing old Repo Data with IPF ID: {}", old_id);

                chain
                    .remove_nft(&old_id, ips_id, subasset_id, signer)
                    .await?;
            }

            eprintln!(
                "Appending new objects and repo data to repository under IPS ID: {}",
                ips_id
            );

            chain
                .append_nfts(
                    (&multi_object_id, &new_repo_data_id),
                    ips_id,
                    subasset_id,
                    signer,
                )
                .await?;

            eprintln!("New objects successfully appended to on-chain repository!");

            println!("ok {}", dst);
        }
        Err(e) => {
            println!("error {} \"{}\"", dst, e);
        }
    }

    println!();
    Ok(())
}

async fn fetch(
    remote_repo: &RepoData,
    chain: &Chain,
    ips_id: u32,
    mut repo: Repository,
    mut ipfs: IpfsClient,
    sha: &str,
    name: &str,
) -> BoxResult<()> {
    remote_repo
        .fetch_to_ref_from_str(sha, name, &mut repo, &mut ipfs, chain, ips_id)
        .await?;

    println!();

    Ok(())
}

fn capabilities() -> BoxResult<()> {
    println!("push");
    println!("fetch\n");
    Ok(())
}

fn list(remote_repo: &RepoData) -> BoxResult<()> {
    for (name, git_hash) in &remote_repo.refs {
        let output = format!("{} {}", git_hash, name);
        println!("{}", output);
    }
    println!();

    Ok(())
}
