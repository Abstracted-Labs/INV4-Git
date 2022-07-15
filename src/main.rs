#![allow(clippy::too_many_arguments)]

use git2::Repository;
use invarch::runtime_types::{
    invarch_runtime::Call, pallet_inv4::pallet::AnyId, pallet_inv4::pallet::Call as IpsCall,
};
use ipfs_api::IpfsClient;
use log::debug;
use primitives::{BoxResult, RepoData};
use sp_keyring::AccountKeyring::Alice;
use std::{
    env::{args, current_dir, var},
    fs::create_dir_all,
    io::{self},
    path::{Path, PathBuf},
    process::Stdio,
};
use subxt::sp_core::Pair;
use subxt::subxt;
use subxt::{ClientBuilder, DefaultConfig, PairSigner, PolkadotExtrinsicParams};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

mod primitives;
mod util;

#[subxt(runtime_metadata_path = "invarch_metadata.scale")]
pub mod invarch {}

pub async fn set_repo(
    ips_id: u32,
    api: invarch::RuntimeApi<DefaultConfig, PolkadotExtrinsicParams<DefaultConfig>>,
) -> BoxResult<RepoData> {
    let mut ipfs_client = IpfsClient::default();
    let data = api
        .storage()
        .inv4()
        .ip_storage(&ips_id, None)
        .await?
        .ok_or(format!("Ips {ips_id} does not exist"))?
        .data
        .0;

    for file in data {
        if let AnyId::IpfId(id) = file {
            let ipf_info = api
                .storage()
                .ipf()
                .ipf_storage(&id, None)
                .await?
                .ok_or("Internal error: IPF listed from IPS does not exist")?;
            if String::from_utf8(ipf_info.metadata.0.clone())? == *"RepoData" {
                return RepoData::from_ipfs(ipf_info.data, &mut ipfs_client).await;
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
    let (alias, raw_url) = {
        let mut args = args();
        args.next();
        (
            args.next().ok_or("Missing alias argument.")?,
            args.next().ok_or("Missing url argument.")?,
        )
    };

    let (ips_id, subasset_id) = {
        let mut url = Path::new(&raw_url).components();
        url.next();
        (
            url.next()
                .ok_or("Missing IPS id. Expected: 'inv4://>ips_id<'")?
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
    // let mut in_writer = BufWriter::new(stdin);

    // Ensure the child process is spawned in the runtime so it can
    // make progress on its own while we await for any output.
    tokio::spawn(async move {
        let status = child
            .wait()
            .await
            .expect("child process encountered an error");

        println!("child status was: {}", status);
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
        error!("No credential")
    }

    let signer = &PairSigner::<DefaultConfig, sp_keyring::sr25519::sr25519::Pair>::new(
        sp_keyring::sr25519::sr25519::Pair::from_string(&credential, None).unwrap(),
    );

    let api: invarch::RuntimeApi<DefaultConfig, PolkadotExtrinsicParams<DefaultConfig>> =
        ClientBuilder::new()
            .set_url("ws://127.0.0.1:9944")
            .build()
            .await?
            .to_runtime_api();

    let git_dir = PathBuf::from(var("GIT_DIR")?);
    create_dir_all(
        current_dir()?
            .join(&git_dir)
            .join("remote-inv4")
            .join(&alias),
    )?;

    let mut remote_repo = set_repo(ips_id, api.clone()).await?;
    debug!("RepoData: {:#?}", remote_repo);

    loop {
        let repo = Repository::open_from_env().unwrap();
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        if input.is_empty() {
            return Ok(());
        }

        debug!("{}", &input.clone());

        let mut args = input.split_ascii_whitespace();

        match (args.next(), args.next(), args.next()) {
            (Some("push"), Some(ref_arg), None) => {
                push(
                    &api,
                    signer,
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
                    &api,
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
    api: &invarch::RuntimeApi<DefaultConfig, PolkadotExtrinsicParams<DefaultConfig>>,
    signer: &PairSigner<DefaultConfig, sp_keyring::sr25519::sr25519::Pair>,
    remote_repo: &mut RepoData,
    ips_id: u32,
    subasset_id: Option<u32>,
    mut repo: Repository,
    mut ipfs: IpfsClient,
    ref_arg: &str,
) -> BoxResult<()> {
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
        .push_ref_from_str(src, dst, force, &mut repo, &mut ipfs, api, signer, ips_id)
        .await
    {
        Ok(mut ipf_id_list) => {
            let (new_repo_data, old_repo_data) = remote_repo
                .mint_return_new_old_id(&mut ipfs, api, signer, ips_id)
                .await?;

            if let Some(old_id) = old_repo_data {
                eprintln!("Removing old Repo Data with IPF ID: {}", old_id);

                let remove_call = Call::INV4(IpsCall::remove {
                    ips_id,
                    assets: vec![(AnyId::IpfId(old_id), Alice.to_account_id())],
                    new_metadata: None,
                });

                api.tx()
                    .inv4()
                    .operate_multisig(false, (ips_id, subasset_id), remove_call)?
                    .sign_and_submit_default(signer)
                    .await?;
            }

            ipf_id_list.push(new_repo_data);

            eprintln!(
                "Appending new objects and repo data to repository under IPS ID: {}",
                ips_id
            );

            let append_call = Call::INV4(IpsCall::append {
                ips_id,
                assets: ipf_id_list.into_iter().map(AnyId::IpfId).collect(),
                new_metadata: None,
            });

            api.tx()
                .inv4()
                .operate_multisig(true, (ips_id, subasset_id), append_call)?
                .sign_and_submit_default(signer)
                .await?;

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
    api: &invarch::RuntimeApi<DefaultConfig, PolkadotExtrinsicParams<DefaultConfig>>,
    ips_id: u32,
    mut repo: Repository,
    mut ipfs: IpfsClient,
    sha: &str,
    name: &str,
) -> BoxResult<()> {
    remote_repo
        .fetch_to_ref_from_str(sha, name, &mut repo, &mut ipfs, api, ips_id)
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
