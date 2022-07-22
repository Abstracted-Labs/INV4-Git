#![allow(clippy::too_many_arguments)]

use dirs::config_dir;
use git2::Repository;
use invarch::runtime_types::{
    invarch_runtime::Call, pallet_inv4::pallet::AnyId, pallet_inv4::pallet::Call as IpsCall,
};
use ipfs_api::IpfsClient;
use log::debug;
use primitives::{BoxResult, Config, RepoData};
use sp_keyring::AccountKeyring::Alice;
use std::{
    env::args,
    io::{self, Read, Write},
    path::Path,
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

/// Gets Repo Data file from IP Set and returns a `RepoData` struct created from the file.
/// If no file, return `RepoData` struct with defaults
pub async fn get_repo(
    ips_id: u32,
    api: invarch::RuntimeApi<DefaultConfig, PolkadotExtrinsicParams<DefaultConfig>>,
) -> BoxResult<RepoData> {
    // Setup IPFS client
    let mut ipfs_client = IpfsClient::default();

    // Get assets (IPFs, NFTs, nested IPS, NFT collections) under `ips_id` IP Set
    let data = api
        .storage()
        .inv4() // pallet
        .ip_storage(&ips_id, None) // storage entity
        .await?
        .ok_or(format!("Ips {ips_id} does not exist"))?
        .data
        .0;

    // Find "RepoData" IPF and return `RepoData` struct if it exists
    for file in data {
        if let AnyId::IpfId(id) = file {
            let ipf_info = api
                .storage()
                .ipf() // pallet
                .ipf_storage(&id, None) // storage entity
                .await?
                .ok_or("Internal error: IPF listed from IPS does not exist")?;
            if String::from_utf8(ipf_info.metadata.0.clone())? == *"RepoData" {
                return RepoData::from_ipfs(ipf_info.data, &mut ipfs_client).await;
            }
        }
    }

    // Return default `RepoData` if file doesn't exist
    Ok(RepoData {
        refs: Default::default(),
        objects: Default::default(),
    })
}

/// Git will call this helper program because it does not natively support git-remote-inv4
#[tokio::main]
async fn main() -> BoxResult<()> {
    // Setup config file

    // Get URL passed from Git.
    let (_, raw_url) = {
        let mut args = args();
        args.next();
        (
            args.next().ok_or("Missing alias argument.")?,
            args.next().ok_or("Missing url argument.")?,
        )
    };

    // Parse `ips_id` and sub token ID (optional) from URL
    let (ips_id, subasset_id) = {
        let mut url = Path::new(&raw_url).components();
        url.next();
        (
            // Get IPS ID
            url.next()
                .ok_or("Missing IPS id. Expected: 'inv4://>ips_id<'")?
                .as_os_str()
                .to_str()
                .ok_or("Input was not UTF-8")?
                .parse::<u32>()?,
            // Get optional sub token ID
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

    let mut config_file_path =
        config_dir().expect("Operating system's configs directory not found");
    config_file_path.push("INV4-Git/config.toml");

    std::fs::create_dir_all(config_file_path.parent().unwrap()).unwrap();

    // Deserialize `config.toml` into `Config` struct
    let config: Config = if config_file_path.exists() {
        // Read contents of config.toml file into `contents` buffer
        let mut contents = String::new();
        std::fs::File::options()
            .write(true)
            .read(true)
            .create(false)
            .open(config_file_path.clone())?
            .read_to_string(&mut contents)?;

        toml::from_str(&contents)?
    } else {
        // Default Substrate RPC server address
        let c = Config {
            chain_endpoint: String::from("ws://127.0.0.1:9944"),
        };

        let mut f = std::fs::File::create(config_file_path)?;

        f.write_all(toml::to_string(&c)?.as_bytes())?;

        c
    };

    // Create subxt client connected to specified chain
    let api: invarch::RuntimeApi<DefaultConfig, PolkadotExtrinsicParams<DefaultConfig>> =
        ClientBuilder::new()
            .set_url(config.chain_endpoint)
            .build()
            .await?
            .to_runtime_api();

    // Get IPS RepoData
    let mut remote_repo = get_repo(ips_id, api.clone()).await?;
    debug!("RepoData: {:#?}", remote_repo);

    loop {
        let repo = Repository::open_from_env().unwrap();

        // Read next input command
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
    remote_repo: &mut RepoData,
    ips_id: u32,
    subasset_id: Option<u32>,
    mut repo: Repository,
    mut ipfs: IpfsClient,
    ref_arg: &str,
) -> BoxResult<()> {
    // Requesting credentials from the user in the terminal
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
        error!("No credential")
    }

    // Generate key pair from command line entered seed phrase
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
        .push_ref_from_str(src, dst, force, &mut repo, &mut ipfs, api, signer, ips_id)
        .await
    {
        Ok(pack_ipf_id) => {
            let (new_repo_data, old_repo_data) = remote_repo
                .mint_return_new_old_id(&mut ipfs, api, signer, ips_id)
                .await?;

            // If IP Set has a pre-existing RepoData file, remove it from the IP Set
            if let Some(old_id) = old_repo_data {
                eprintln!("Removing old Repo Data with IPF ID: {}", old_id);

                let remove_call = Call::INV4(IpsCall::remove {
                    ips_id,
                    assets: vec![(AnyId::IpfId(old_id), Alice.to_account_id())],
                    new_metadata: None,
                });

                // Sign and submit the `remove_call` extrinsic to remove the old RepoData IPF from the IPS
                api.tx()
                    .inv4()
                    .operate_multisig(false, (ips_id, subasset_id), remove_call)?
                    .sign_and_submit_default(signer)
                    .await?;
            }

            eprintln!(
                "Appending new objects and repo data to repository under IPS ID: {}",
                ips_id
            );

            let append_call = Call::INV4(IpsCall::append {
                ips_id,
                assets: vec![AnyId::IpfId(pack_ipf_id), AnyId::IpfId(new_repo_data)], //ipf_id_list.into_iter().map(AnyId::IpfId).collect(),
                new_metadata: None,
            });

            // Sign and submit the `append_call` extrinsic to add the new RepoData IPF to the IPS
            api.tx()
                .inv4()
                .operate_multisig(true, (ips_id, subasset_id), append_call)?
                .sign_and_submit_then_watch_default(signer)
                .await?
                .wait_for_in_block()
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
