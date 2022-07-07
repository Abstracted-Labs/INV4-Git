#![allow(clippy::too_many_arguments, clippy::enum_variant_names)]
use std::{
    fs::{remove_file, write, File},
    io::Write,
    path::{Path, PathBuf},
    process::exit,
    str::FromStr,
};

use cid::Cid;
use codec::{Decode, Encode};
use futures::TryStreamExt;
use invarch::runtime_types::{
    invarch_runtime::Call, pallet_inv4::pallet::AnyId, pallet_inv4::pallet::Call as IpsCall,
};
use ipfs_api::{IpfsApi, IpfsClient};
use sp_keyring::{sr25519::sr25519::Pair, AccountKeyring::Alice};
use subxt::{
    sp_core::H256, sp_runtime::AccountId32, subxt, ClientBuilder, DefaultConfig, PairSigner,
    PolkadotExtrinsicParams,
};

use git2::Repository;
use temp_dir::TempDir;

use crate::{
    error,
    primitives::{BoxResult, RefsFile, Settings},
    util::{
        create_bundle_all, create_bundle_target_ref, generate_cid, log, pull_from_bundle, show_ref,
    },
};

#[subxt(runtime_metadata_path = "invarch_metadata.scale")]
pub mod invarch {}

pub struct GitArch {
    pub signer: PairSigner<DefaultConfig, Pair>,
    pub signer_account: AccountId32,
    pub api: invarch::RuntimeApi<DefaultConfig, PolkadotExtrinsicParams<DefaultConfig>>,
    pub ipfs_client: IpfsClient,
}

impl GitArch {
    pub async fn new() -> BoxResult<Self> {
        Ok(Self {
            signer: PairSigner::<DefaultConfig, Pair>::new(Alice.pair()),
            signer_account: Alice.to_account_id(),
            api: ClientBuilder::new()
                .set_url("ws://127.0.0.1:9944")
                .build()
                .await?
                .to_runtime_api(),
            ipfs_client: IpfsClient::default(),
        })
    }

    pub async fn list(&self, settings: &Settings, path: &str) -> BoxResult<String> {
        let temp_dir = TempDir::new()?;

        std::fs::create_dir_all(path).unwrap();

        let ips_id = settings.root.ips_id;
        Repository::init(path)?;

        let ips_info = self
            .api
            .storage()
            .inv4()
            .ip_storage(&ips_id, None)
            .await?
            .ok_or(format!("Ips {ips_id} does not exist"))?;

        let ips_data = ips_info.data.0;

        if let Ok((refs_hash, _)) = self.find_refs_file(&ips_data).await {
            let refs_cid = generate_cid(refs_hash)?.to_string();
            let refs_content = self
                .ipfs_client
                .cat(&refs_cid)
                .map_ok(|c| c.to_vec())
                .try_concat()
                .await?;

            let refs = RefsFile::decode(&mut refs_content.as_slice()).unwrap();

            for anyid in &ips_data {
                if let AnyId::IpfId(ipf_id) = anyid {
                    let ipf_info = self
                        .api
                        .storage()
                        .ipf()
                        .ipf_storage(&ipf_id, None)
                        .await?
                        .ok_or("Internal error: IPF listed from IPS does not exist")?;

                    if refs
                        .refs
                        .contains(&String::from_utf8(ipf_info.metadata.0.clone()).unwrap())
                    {
                        let content = self
                            .ipfs_client
                            .cat(&generate_cid(ipf_info.data)?.to_string())
                            .map_ok(|c| c.to_vec())
                            .try_concat()
                            .await?;

                        let bundle_path =
                            temp_dir.child(&String::from_utf8(ipf_info.metadata.0).unwrap());

                        write(&bundle_path, content)?;

                        pull_from_bundle(Path::new(path), &bundle_path).unwrap();
                    }
                }
            }
        } else {
            return Ok(String::default());
        };
        let refs = show_ref(Path::new(path))?.trim().to_string();

        Ok(refs)
    }

    pub async fn fetch(&self, settings: &Settings) -> BoxResult<()> {
        let temp_dir = TempDir::new()?;

        let ips_id = settings.root.ips_id;

        let ip_set_info = self
            .api
            .storage()
            .inv4()
            .ip_storage(&ips_id, None)
            .await?
            .ok_or(format!("IPS {} does not exist", ips_id))?;

        if let Ok(remote_bundle) = self.find_refs_file(&ip_set_info.data.0).await {
            let cid = generate_cid(remote_bundle.0)?.to_string();

            let content = self
                .ipfs_client
                .cat(&cid)
                .map_ok(|c| c.to_vec())
                .try_concat()
                .await?;

            let bundle_path = temp_dir.child("inv4.bundle");

            write(&bundle_path, content)?;
            pull_from_bundle(Path::new("."), &bundle_path)?;

            //    let mut file = File::create("gitarch.bundle")?;
            //    file.write_all(&content)?;

            //    pull_from_bundle(
            //        Path::new("."),
            //         &PathBuf::from_str("gitarch.bundle").unwrap(),
            //     )?;

            //  remove_file("gitarch.bundle")?;
            exit(0);
        }
        Ok(())
    }

    pub async fn push(&self, settings: &Settings) -> BoxResult<()> {
        let ips_id = settings.root.ips_id;

        // First let's find out which references, if any, we already have on chain.
        let ips = self
            .api
            .storage()
            .inv4()
            .ip_storage(&ips_id, None)
            .await?
            .ok_or(format!("IPS {ips_id} does not exist"))?;

        let repo = Repository::open("./").unwrap();

        let local_refs = RefsFile::from_reference_names(repo.references().unwrap().names(), &repo);

        if let Ok(refs_file) = self.find_refs_file_owned(ips.data.0).await {
            let cid = generate_cid(refs_file.0)?.to_string();

            let content = self
                .ipfs_client
                .cat(&cid)
                .map_ok(|c| c.to_vec())
                .try_concat()
                .await?;

            let remote_refs = RefsFile::decode(&mut content.as_slice()).unwrap();

            let latest_ref_from_remote = remote_refs.get_latest_ref();

            if latest_ref_from_remote == local_refs.get_latest_ref() {
                panic!("remote and local already in sync");
            } else {
                create_bundle_target_ref(Path::new("inv4.bundle"), latest_ref_from_remote.clone())
                    .unwrap();

                let remove_call = Call::INV4(IpsCall::remove {
                    ips_id,
                    assets: vec![(refs_file.1, self.signer_account.clone())],
                    new_metadata: None,
                });

                let transaction = self
                    .api
                    .tx()
                    .inv4()
                    .operate_multisig(false, (ips_id, settings.root.subasset_id), remove_call)?
                    .sign_and_submit_default(&self.signer)
                    .await?;

                eprintln!("Submitted IPS remove: {transaction:?}");
            }
        } else {
            create_bundle_all(Path::new("inv4.bundle")).unwrap();
        }

        eprintln!("past create bundle section");

        let bundle_file = File::open("inv4.bundle")?;

        let mut new_refs_file = std::fs::OpenOptions::new()
            .write(true)
            .create(true) // This is needed to append to file
            .open("refs")
            .unwrap();

        new_refs_file
            .write_all(local_refs.encode().as_slice())
            .unwrap();

        // Send file to IPFS
        let bundle_cid =
            &Cid::try_from(self.ipfs_client.add(bundle_file).await?.hash)?.to_bytes()[2..];
        let refs_cid =
            &Cid::try_from(self.ipfs_client.add(File::open("refs")?).await?.hash)?.to_bytes()[2..];

        eprintln!("got cids");

        let refs_ipf_id: u64 = self.api.storage().ipf().next_ipf_id(None).await?;
        let transaction = self
            .api
            .tx()
            .ipf()
            .mint(b"refs".to_vec(), H256::from_slice(refs_cid))?
            .sign_and_submit_default(&self.signer)
            .await?;

        eprintln!("Submitted IPF Mint for file \"refs\": {transaction:?}");

        let bundle_ipf_id: u64 = self.api.storage().ipf().next_ipf_id(None).await?;
        let transaction = self
            .api
            .tx()
            .ipf()
            .mint(
                local_refs.get_latest_ref().into_bytes(),
                H256::from_slice(bundle_cid),
            )?
            .sign_and_submit_default(&self.signer)
            .await?;

        eprintln!("Submitted IPF Mint for file \"bundle\": {transaction:?}");

        let append_call = Call::INV4(IpsCall::append {
            ips_id,
            assets: vec![AnyId::IpfId(refs_ipf_id), AnyId::IpfId(refs_ipf_id + 1)],
            new_metadata: None,
        });

        let transaction = self
            .api
            .tx()
            .inv4()
            .operate_multisig(true, (ips_id, settings.root.subasset_id), append_call)?
            .sign_and_submit_default(&self.signer)
            .await?;

        eprintln!("Submitter IPS append: {transaction:?}");

        Ok(())
    }

    async fn find_refs_file<'a>(
        &self,
        files: &'a Vec<AnyId<u32, u64, (u32, u32), u32>>,
    ) -> BoxResult<(H256, &'a AnyId<u32, u64, (u32, u32), u32>)> {
        for file in files {
            if let AnyId::IpfId(id) = file {
                let ipf_info = self
                    .api
                    .storage()
                    .ipf()
                    .ipf_storage(&id, None)
                    .await?
                    .ok_or("Internal error: IPF listed from IPS does not exist")?;
                if String::from_utf8(ipf_info.metadata.0.clone())? == *"refs" {
                    return Ok((ipf_info.data, file));
                }
            }
        }
        error!("bundle not found")
    }

    async fn find_refs_file_owned(
        &self,
        files: Vec<AnyId<u32, u64, (u32, u32), u32>>,
    ) -> BoxResult<(H256, AnyId<u32, u64, (u32, u32), u32>)> {
        for file in files {
            if let AnyId::IpfId(id) = file {
                let ipf_info = self
                    .api
                    .storage()
                    .ipf()
                    .ipf_storage(&id, None)
                    .await?
                    .ok_or("Internal error: IPF listed from IPS does not exist")?;
                if String::from_utf8(ipf_info.metadata.0.clone())? == *"refs" {
                    return Ok((ipf_info.data, file));
                }
            }
        }
        error!("bundle not found")
    }
}
