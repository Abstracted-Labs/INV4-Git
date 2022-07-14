#![warn(clippy::pedantic)]
#![allow(clippy::unnecessary_wraps)] // Allowed while TODOs in functions exists
use crate::client::invarch::runtime_types::{
    invarch_runtime::Call, pallet_inv4::pallet::AnyId, pallet_inv4::pallet::Call as IpsCall,
};
use git2::Repository;
use ipfs_api::IpfsClient;
use primitives::{BoxResult, Key, RepoData, Settings};
use sp_keyring::{sr25519::sr25519::Pair, AccountKeyring::Alice};
use std::{
    env::{args, current_dir, var},
    fs::create_dir_all,
    io::stdin,
    path::{Path, PathBuf},
};
use subxt::{ClientBuilder, DefaultConfig, PairSigner, PolkadotExtrinsicParams};

mod client;
mod primitives;
mod util;

pub async fn set_repo(
    ips_id: u32,
    api: crate::client::invarch::RuntimeApi<DefaultConfig, PolkadotExtrinsicParams<DefaultConfig>>,
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

    let api: crate::client::invarch::RuntimeApi<
        DefaultConfig,
        PolkadotExtrinsicParams<DefaultConfig>,
    > = ClientBuilder::new()
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

    let settings = Settings {
        git_dir,
        remote_alias: alias,
        root: Key {
            ips_id,
            subasset_id,
        },
    };

    let mut remote_repo = set_repo(settings.root.ips_id, api.clone()).await?;
    eprintln!("RepoData: {:#?}", remote_repo);

    loop {
        let repo = Repository::open_from_env().unwrap();
        let mut input = String::new();
        stdin().read_line(&mut input)?;

        if input.is_empty() {
            return Ok(());
        }

        eprintln!("{}", &input.clone());

        let mut args = input.split_ascii_whitespace();

        match (args.next(), args.next(), args.next()) {
            (Some("push"), Some(ref_arg), None) => {
                push(
                    &api,
                    &mut remote_repo,
                    &settings,
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
                    &settings,
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
    api: &crate::client::invarch::RuntimeApi<DefaultConfig, PolkadotExtrinsicParams<DefaultConfig>>,
    remote_repo: &mut RepoData,
    settings: &Settings,
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
    eprintln!("Parsed src: {}", src);

    let dst = refspec_iter
        .next()
        .ok_or_else(|| eprintln!("Could not read destination ref from refspec: {:?}", ref_arg))
        .unwrap();
    eprintln!("Parsed dst: {}", dst);

    // Upload the object tree
    match remote_repo
        .push_ref_from_str(
            src,
            dst,
            force,
            &mut repo,
            &mut ipfs,
            &api,
            settings.root.ips_id,
        )
        .await
    {
        Ok(mut ipf_id_list) => {
            let (new_repo_data, old_repo_data) = remote_repo
                .mint_return_new_old_id(&mut ipfs, &api, settings.root.ips_id)
                .await?;

            if let Some(old_id) = old_repo_data {
                eprintln!("Found old RepoData, removing it!");

                let remove_call = Call::INV4(IpsCall::remove {
                    ips_id: settings.root.ips_id,
                    assets: vec![(AnyId::IpfId(old_id), Alice.to_account_id())],
                    new_metadata: None,
                });

                api.tx()
                    .inv4()
                    .operate_multisig(
                        false,
                        (settings.root.ips_id, settings.root.subasset_id),
                        remove_call,
                    )?
                    .sign_and_submit_default(&PairSigner::<DefaultConfig, Pair>::new(Alice.pair()))
                    .await?;
            }

            ipf_id_list.push(new_repo_data);

            eprintln!("Appending new data!");

            let append_call = Call::INV4(IpsCall::append {
                ips_id: settings.root.ips_id,
                assets: ipf_id_list
                    .into_iter()
                    .map(|ipf_id| AnyId::IpfId(ipf_id))
                    .collect(),
                new_metadata: None,
            });

            api.tx()
                .inv4()
                .operate_multisig(
                    true,
                    (settings.root.ips_id, settings.root.subasset_id),
                    append_call,
                )?
                .sign_and_submit_default(&PairSigner::<DefaultConfig, Pair>::new(Alice.pair()))
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
    api: &crate::client::invarch::RuntimeApi<DefaultConfig, PolkadotExtrinsicParams<DefaultConfig>>,
    settings: &Settings,
    mut repo: Repository,
    mut ipfs: IpfsClient,
    sha: &str,
    name: &str,
) -> BoxResult<()> {
    remote_repo
        .fetch_to_ref_from_str(sha, name, &mut repo, &mut ipfs, api, settings.root.ips_id)
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
        eprintln!("{}", output);
        println!("{}", output);
    }
    println!();

    Ok(())
}
