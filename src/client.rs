#![allow(clippy::too_many_arguments, clippy::enum_variant_names)]
use std::{
    fs::{remove_file, write, File},
    io::Write,
    path::Path,
    process::exit,
};

use cid::Cid;
use futures::TryStreamExt;
use invarch::runtime_types::{
    invarch_primitives::AnyId, invarch_runtime::Call, pallet_ips::pallet::Call as IpsCall,
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
    primitives::{BoxResult, Settings},
    util::{create_bundle, generate_cid, pull_from_bundle, show_ref},
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

    pub async fn list(&self, settings: &Settings) -> BoxResult<String> {
        let temp_dir = TempDir::new()?;

        let ips_id = settings.root.ips_id;
        Repository::init(temp_dir.path())?;

        let remote_bundle: H256 = {
            let ips_info = self
                .api
                .storage()
                .ips()
                .ips_storage(&ips_id, None)
                .await?
                .ok_or(format!("Ips {ips_id} does not exist"))?;
            if let Ok((bundle, _)) = self.find_bundle(ips_info.data.0).await {
                bundle
            } else {
                return Ok(String::default());
            }
        };

        let cid = generate_cid(remote_bundle)?.to_string();

        let content = self
            .ipfs_client
            .cat(&cid)
            .map_ok(|c| c.to_vec())
            .try_concat()
            .await?;

        let bundle_path = temp_dir.child("gitarch.bundle");

        write(&bundle_path, content)?;
        pull_from_bundle(temp_dir.path())?;
        let refs = show_ref(temp_dir.path())?.trim().to_string();

        Ok(refs)
    }

    pub async fn fetch(&self, settings: &Settings) -> BoxResult<()> {
        let ips_id = settings.root.ips_id;

        let ip_set_info = self
            .api
            .storage()
            .ips()
            .ips_storage(&ips_id, None)
            .await?
            .ok_or(format!("IPS {} does not exist", ips_id))?;

        if let Ok(remote_bundle) = self.find_bundle(ip_set_info.data.0).await {
            let cid = generate_cid(remote_bundle.0)?.to_string();

            let content = self
                .ipfs_client
                .cat(&cid)
                .map_ok(|c| c.to_vec())
                .try_concat()
                .await?;

            let mut file = File::create("gitarch.bundle")?;
            file.write_all(&content)?;

            pull_from_bundle(Path::new("."))?;

            remove_file("gitarch.bundle")?;
            exit(0);
        }
        Ok(())
    }

    pub async fn push(&self, settings: &Settings) -> BoxResult<()> {
        let ips_id = settings.root.ips_id;
        let subasset_id = settings.root.subasset_id;
        // Predict next IPF_ID
        let bundle_ipf_id: u64 = self.api.storage().ipf().next_ipf_id(None).await?;

        create_bundle(Path::new("gitarch.bundle"))?;
        let file = File::open("gitarch.bundle")?;

        // Send file to IPFS
        let cid = &Cid::try_from(self.ipfs_client.add(file).await?.hash)?.to_bytes()[2..];

        let transaction = self
            .api
            .tx()
            .ipf()
            .mint(b"gitarch.bundle".to_vec(), H256::from_slice(cid))?
            .sign_and_submit_default(&self.signer)
            .await?;

        eprintln!("Submitted IPF Mint for file \"gitarch.bundle\": {transaction:?}");

        // Remove old bundle from IPS
        let ips_info = self
            .api
            .storage()
            .ips()
            .ips_storage(&ips_id, None)
            .await?
            .ok_or(format!("IPS {ips_id} does not exist"))?;

        if let Ok(old_bundle) = self.find_bundle(ips_info.data.0).await {
            let remove_call = Call::Ips(IpsCall::remove {
                ips_id,
                assets: vec![(old_bundle.1, self.signer_account.clone())],
                new_metadata: None,
            });

            let transaction = self
                .api
                .tx()
                .ipt()
                .operate_multisig(false, (ips_id, subasset_id), remove_call)?
                .sign_and_submit_default(&self.signer)
                .await?;

            eprintln!("Submitted IPS remove: {transaction:?}");
        }

        // Move new bundle file to IPS
        let append_call = Call::Ips(IpsCall::append {
            ips_id,
            assets: vec![AnyId::IpfId(bundle_ipf_id)],
            new_metadata: None,
        });

        let transaction = self
            .api
            .tx()
            .ipt()
            .operate_multisig(true, (ips_id, subasset_id), append_call)?
            .sign_and_submit_default(&self.signer)
            .await?;

        eprintln!("Submitter IPS append: {transaction:?}");

        Ok(())
    }

    async fn find_bundle(&self, files: Vec<AnyId<u64, u64>>) -> BoxResult<(H256, AnyId<u64, u64>)> {
        for file in files {
            if let AnyId::IpfId(id) = file {
                let ipf_info = self
                    .api
                    .storage()
                    .ipf()
                    .ipf_storage(&id, None)
                    .await?
                    .ok_or("Internal error: IPF listed from IPS does not exist")?;
                if String::from_utf8(ipf_info.metadata.0.clone())? == *"gitarch.bundle" {
                    return Ok((ipf_info.data, file));
                }
            }
        }
        error!("bundle not found")
    }
}
