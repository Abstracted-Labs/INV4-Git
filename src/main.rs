#![allow(clippy::too_many_arguments)]

use dirs::config_dir;
use git2::{CredentialHelper, Repository};
use ipfs_api::IpfsClient;
use log::debug;
use primitives::{BoxResult, Config, RepoData};
use std::{
    env::args,
    io::{self, BufRead, Read},
    path::Path,
    process::Stdio,
};
use subxt::{ext::sp_core::sr25519::Pair as Sr25519Pair, subxt};
use subxt::{ext::sp_core::Pair, tx::PairSigner};
use subxt::{OnlineClient, PolkadotConfig};
use tinkernet::runtime_types::{
    pallet_inv4::pallet::AnyId, pallet_inv4::pallet::Call as INV4Call,
    pallet_utility::pallet::Call as UtilityCall, tinkernet_runtime::Call,
};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use magic_crypt::new_magic_crypt;
use magic_crypt::MagicCryptTrait;

mod compression;
mod primitives;
mod util;

#[cfg(feature = "crust")]
mod crust;

#[subxt(runtime_metadata_path = "tinkernet_metadata.scale")]
pub mod tinkernet {}

pub async fn get_repo(ips_id: u32, api: OnlineClient<PolkadotConfig>) -> BoxResult<RepoData> {
    let mut ipfs_client = IpfsClient::default();
    let ips_storage_address = tinkernet::storage().inv4().ip_storage(&ips_id);

    let data = api
        .storage()
        .fetch(&ips_storage_address, None)
        .await?
        .expect("Couldn't find this repository on-chain")
        .data
        .0;

    for file in data {
        if let AnyId::IpfId(id) = file {
            let ipf_storage_address = tinkernet::storage().ipf().ipf_storage(&id);

            let ipf_info = api
                .storage()
                .fetch(&ipf_storage_address, None)
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
    let raw_url = {
        let mut args = args();
        args.next();
        args.next();

        args.next().ok_or("Missing url argument.")?
    };
    git(raw_url).await
}

#[cfg(target_family = "unix")]
fn read_input() -> std::io::Result<String> {
    let mut string = String::new();
    let tty = std::fs::File::open("/dev/tty")?;
    let mut reader = io::BufReader::new(tty);
    reader.read_line(&mut string)?;
    Ok(string.trim().to_string())
}

#[cfg(target_family = "windows")]
fn read_input() -> std::io::Result<String> {
    let mut string = String::new();
    let handle = unsafe {
        CreateFileA(
            b"CONIN$\x00".as_ptr() as *const i8,
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null_mut(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    };

    if handle == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error());
    }

    let mut stream = BufReader::new(unsafe { std::fs::File::from_raw_handle(handle) });

    let reader_return = reader.read_line(&mut string);

    // Newline for windows which otherwise prints on the same line.
    // println!();

    if reader_return.is_err() {
        return Err(reader_return.unwrap_err());
    }

    Ok(string)
}

async fn auth_flow() -> BoxResult<String> {
    let mut cred_helper = CredentialHelper::new("https://inv4-tinkernet");
    cred_helper.config(&git2::Config::open_default().unwrap());
    let creds = cred_helper.execute();

    Ok(if let Some((username, encrypted_seed)) = creds {
        let mut password =
            rpassword::prompt_password(format!("Enter password for {}: ", username))?;

        password = password.trim().to_string();

        let mcrypt = new_magic_crypt!(password, 256);

        mcrypt.decrypt_base64_to_string(&encrypted_seed).unwrap()
    } else {
        let mut seed = rpassword::prompt_password("Enter your private key/seed phrase: ")?;

        let mut password = rpassword::prompt_password("Create a password: ")?;

        eprint!("Give this account a nickname: ");
        let name = read_input()?;

        let mut cmd = Command::new("git");
        cmd.arg("credential");
        cmd.arg("approve");
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::null());

        let mut child = cmd.spawn().expect("failed to spawn command");

        let mut stdin = child
            .stdin
            .take()
            .expect("child did not have a handle to stdin");

        seed = seed.trim().to_string();
        password = password.trim().to_string();

        let mcrypt = new_magic_crypt!(password, 256);
        let encrypted_seed = mcrypt.encrypt_str_to_base64(&seed);

        stdin
            .write_all(
                format!(
                    "protocol=https\nhost=inv4-tinkernet\nusername={}\npassword={}\n\n",
                    &name, &encrypted_seed
                )
                .as_bytes(),
            )
            .await
            .expect("could not write to stdin");

        drop(stdin);

        child.wait_with_output().await.unwrap();

        seed
    })
}

async fn git(raw_url: String) -> BoxResult<()> {
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
        Config {
            chain_endpoint: String::from("wss://tinker.invarch.network:443"),
        }
    };

    let api = OnlineClient::<PolkadotConfig>::from_url(config.chain_endpoint).await?;

    let mut remote_repo = get_repo(ips_id, api.clone()).await?;
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
    api: &OnlineClient<PolkadotConfig>,
    remote_repo: &mut RepoData,
    ips_id: u32,
    subasset_id: Option<u32>,
    mut repo: Repository,
    mut ipfs: IpfsClient,
    ref_arg: &str,
) -> BoxResult<()> {
    let seed = auth_flow().await.unwrap();

    let pair = Sr25519Pair::from_string(&seed, None).expect("Invalid credentials");
    let signer = PairSigner::new(pair);

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
        .push_ref_from_str(src, dst, force, &mut repo, &mut ipfs, api, &signer, ips_id)
        .await
    {
        Ok(pack_ipf_id) => {
            let (new_repo_data, old_repo_data) = remote_repo
                .mint_return_new_old_id(&mut ipfs, api, &signer, ips_id)
                .await?;

            let mut calls: Vec<Call> = vec![];

            if let Some(old_id) = old_repo_data {
                eprintln!("Removing old Repo Data with IPF ID: {}", old_id);

                calls.push(Call::INV4(INV4Call::remove {
                    ips_id,
                    original_caller: Some(signer.account_id().clone()),
                    assets: vec![(AnyId::IpfId(old_id), signer.account_id().clone())],
                    new_metadata: None,
                }));
            }

            eprintln!(
                "Appending new objects and repo data to repository under IPS ID: {}",
                ips_id
            );

            calls.push(Call::INV4(INV4Call::append {
                ips_id,
                original_caller: Some(signer.account_id().clone()),
                assets: vec![AnyId::IpfId(pack_ipf_id), AnyId::IpfId(new_repo_data)], //ipf_id_list.into_iter().map(AnyId::IpfId).collect(),
                new_metadata: None,
            }));

            let batch_call = Call::Utility(UtilityCall::batch_all { calls });

            let multisig_batch_tx = tinkernet::tx().inv4().operate_multisig(
                true,
                (ips_id, subasset_id),
                Some(b"{\"protocol\":\"inv4-git\",\"type\":\"push\"}".to_vec()),
                batch_call,
            );

            api.tx()
                .sign_and_submit_then_watch_default(&multisig_batch_tx, &signer)
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
    api: &OnlineClient<PolkadotConfig>,
    ips_id: u32,
    mut repo: Repository,
    mut ipfs: IpfsClient,
    sha: &str,
    name: &str,
) -> BoxResult<()> {
    remote_repo
        .fetch_to_ref_from_str(sha, name, &mut repo, &mut ipfs, api, ips_id)
        .await?;

    tokio::io::stdout().write_all(b"\n").await?;

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
