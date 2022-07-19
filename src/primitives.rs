use crate::tinkernet;
use cid::{multihash::MultihashGeneric, CidGeneric};
use codec::{Decode, Encode};
use futures::TryStreamExt;
use git2::{Blob, Commit, Object, ObjectType, Odb, Oid, Repository, Tag, Tree};
use ipfs_api::{IpfsApi, IpfsClient};
use log::debug;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    error::Error,
    fmt::Display,
    io::Cursor,
};
use subxt::{sp_core::H256, ClientBuilder, DefaultConfig, PairSigner, PolkadotExtrinsicParams};
use twox_hash::xxh3;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Config {
    pub chain_endpoint: String,
}

/// A magic value used to signal that a hash is a submodule tip (to be obtained by git on its own).
pub static SUBMODULE_TIP_MARKER: &str = "submodule-tip";

pub type BoxResult<T> = Result<T, Box<dyn Error>>;

#[derive(Clone)]
pub enum Chain {
    Tinkernet {
        api: tinkernet::RuntimeApi<DefaultConfig, PolkadotExtrinsicParams<DefaultConfig>>,
    },
}

pub trait INV4AnyId {
    fn nft_id(&self) -> Option<NFTIdByChain>;
}

impl INV4AnyId for tinkernet::runtime_types::pallet_inv4::pallet::AnyId<u32, u64, (u32, u32), u32> {
    fn nft_id(&self) -> Option<NFTIdByChain> {
        if let tinkernet::runtime_types::pallet_inv4::pallet::AnyId::IpfId(id) = self {
            Some(NFTIdByChain::Tinkernet(*id))
        } else {
            None
        }
    }
}

pub trait NFTInfo {
    fn data(&self) -> Result<CidGeneric<32>, Box<dyn Error>>;
    fn metadata(&self) -> Vec<u8>;
}

impl NFTInfo
    for tinkernet::runtime_types::invarch_primitives::IpfInfo<
        subxt::sp_runtime::AccountId32,
        subxt::sp_core::H256,
        tinkernet::runtime_types::frame_support::storage::bounded_vec::BoundedVec<u8>,
    >
{
    fn data(&self) -> Result<CidGeneric<32>, Box<dyn Error>> {
        Ok(CidGeneric::new_v0(MultihashGeneric::<32>::from_bytes(
            hex::decode(format!("{:?}", self.data.0).replace("0x", "1220"))?.as_slice(),
        )?)?)
    }

    fn metadata(&self) -> Vec<u8> {
        self.metadata.0.clone()
    }
}

#[derive(Clone)]
pub enum NFTIdByChain {
    Tinkernet(u64),
}

impl Display for NFTIdByChain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::Tinkernet(id) => id,
            }
        )
    }
}

impl Chain {
    pub async fn from_str(s: String, config: Config) -> Result<Self, Box<dyn Error>> {
        match s.to_lowercase().as_str() {
            "tinkernet" => Ok(Self::Tinkernet {
                api: ClientBuilder::new()
                    .set_url(config.chain_endpoint)
                    .build()
                    .await?
                    .to_runtime_api(),
            }),
            _ => Err("Not a supported chain".into()),
        }
    }

    pub async fn ips_data(
        &self,
        ips_id: u32,
    ) -> Result<impl IntoIterator<Item = impl INV4AnyId>, Box<dyn Error>> {
        Ok(match self {
            Chain::Tinkernet { api } => {
                api.storage()
                    .inv4()
                    .ip_storage(&ips_id, None)
                    .await?
                    .ok_or(format!("IPS {ips_id} does not exist"))?
                    .data
                    .0
            }
        })
    }

    pub async fn nft_info(&self, nft_id: &NFTIdByChain) -> Result<impl NFTInfo, Box<dyn Error>> {
        Ok(match (self, nft_id) {
            (Chain::Tinkernet { api }, NFTIdByChain::Tinkernet(id)) => api
                .storage()
                .ipf()
                .ipf_storage(&id, None)
                .await?
                .ok_or("Internal error: IPF listed from IPS does not exist")?,
        })
    }

    pub async fn mint_nft(
        &self,
        metadata: Vec<u8>,
        data: CidGeneric<32>,
        signer: &PairSigner<DefaultConfig, sp_keyring::sr25519::sr25519::Pair>,
    ) -> Result<NFTIdByChain, Box<dyn Error>> {
        Ok(match self {
            Chain::Tinkernet { api } => {
                let events = api
                    .tx()
                    .ipf()
                    .mint(metadata, H256::from_slice(&data.to_bytes()[2..]))?
                    .sign_and_submit_then_watch_default(signer)
                    .await?
                    .wait_for_in_block()
                    .await?;

                let ipf_id = events
                    .fetch_events()
                    .await?
                    .find_first::<tinkernet::ipf::events::Minted>()?
                    .unwrap()
                    .1;

                events.wait_for_success().await?;

                NFTIdByChain::Tinkernet(ipf_id)
            }
        })
    }

    pub async fn remove_nft(
        &self,
        nft_id: &NFTIdByChain,
        ips_id: u32,
        subasset_id: Option<u32>,
        signer: &PairSigner<DefaultConfig, sp_keyring::sr25519::sr25519::Pair>,
    ) -> Result<(), Box<dyn Error>> {
        Ok(match (self, nft_id) {
            (Chain::Tinkernet { api }, NFTIdByChain::Tinkernet(nft_id)) => {
                let remove_call = tinkernet::runtime_types::invarch_runtime::Call::INV4(
                    tinkernet::runtime_types::pallet_inv4::pallet::Call::remove {
                        ips_id,
                        assets: vec![(
                            tinkernet::runtime_types::pallet_inv4::pallet::AnyId::IpfId(*nft_id),
                            signer.account_id().clone(),
                        )],
                        new_metadata: None,
                    },
                );

                api.tx()
                    .inv4()
                    .operate_multisig(false, (ips_id, subasset_id), remove_call)?
                    .sign_and_submit_default(signer)
                    .await?;
            }
        })
    }

    pub async fn append_nfts(
        &self,
        nft_ids: (&NFTIdByChain, &NFTIdByChain),
        ips_id: u32,
        subasset_id: Option<u32>,
        signer: &PairSigner<DefaultConfig, sp_keyring::sr25519::sr25519::Pair>,
    ) -> Result<(), Box<dyn Error>> {
        Ok(match (self, nft_ids) {
            (
                Chain::Tinkernet { api },
                (NFTIdByChain::Tinkernet(multi_object), NFTIdByChain::Tinkernet(repo_data)),
            ) => {
                let append_call = tinkernet::runtime_types::invarch_runtime::Call::INV4(
                    tinkernet::runtime_types::pallet_inv4::pallet::Call::append {
                        ips_id,
                        assets: vec![
                            tinkernet::runtime_types::pallet_inv4::pallet::AnyId::IpfId(
                                *multi_object,
                            ),
                            tinkernet::runtime_types::pallet_inv4::pallet::AnyId::IpfId(*repo_data),
                        ],
                        new_metadata: None,
                    },
                );

                api.tx()
                    .inv4()
                    .operate_multisig(true, (ips_id, subasset_id), append_call)?
                    .sign_and_submit_default(signer)
                    .await?;
            }
        })
    }
}

#[derive(Clone, Debug, Encode, Decode)]
pub struct MultiObject {
    pub hash: String,
    pub git_hashes: Vec<String>,
    pub objects: BTreeMap<String, GitObject>,
}

impl MultiObject {
    pub fn add(&mut self, object: GitObject) {
        let hash = object.git_hash.clone();
        self.objects.insert(hash.clone(), object);
        self.git_hashes.push(hash);
    }

    pub async fn chain_get(
        hash: String,
        ipfs: &mut IpfsClient,
        chain: &Chain,
        ips_id: u32,
    ) -> Result<Self, Box<dyn Error>> {
        let ips_data = chain.ips_data(ips_id).await?;

        for file in ips_data {
            if let Some(nft_id) = file.nft_id() {
                let ipf_info = chain.nft_info(&nft_id).await?;
                if String::from_utf8(ipf_info.metadata())? == *hash {
                    return Ok(Self::decode(
                        &mut ipfs
                            .cat(&ipf_info.data()?.to_string())
                            .map_ok(|c| c.to_vec())
                            .try_concat()
                            .await?
                            .as_slice(),
                    )?);
                }
            }
        }
        Err("git_hash ipf not found".into())
    }
}

#[derive(Clone, Debug, Encode, Decode)]
pub struct GitObject {
    /// The git hash of the underlying git object
    pub git_hash: String,
    /// A link to the raw form of the object
    pub data: Vec<u8>,
    /// Object-type-specific metadata
    pub metadata: GitObjectMetadata,
}

#[derive(Clone, Debug, Encode, Decode)]
pub enum GitObjectMetadata {
    #[allow(missing_docs)]
    Commit {
        parent_git_hashes: BTreeSet<String>,
        tree_git_hash: String,
    },
    #[allow(missing_docs)]
    Tag { target_git_hash: String },
    #[allow(missing_docs)]
    Tree { entry_git_hashes: BTreeSet<String> },
    #[allow(missing_docs)]
    Blob,
}

impl GitObject {
    pub fn from_git_blob(blob: &Blob, odb: &Odb) -> Result<Self, Box<dyn Error>> {
        let odb_obj = odb.read(blob.id())?;

        Ok(Self {
            git_hash: blob.id().to_string(),
            data: odb_obj.data().to_vec(),
            metadata: GitObjectMetadata::Blob,
        })
    }

    pub fn from_git_commit(commit: &Commit, odb: &Odb) -> Result<Self, Box<dyn Error>> {
        let odb_obj = odb.read(commit.id())?;

        let parent_git_hashes: BTreeSet<String> = commit
            .parent_ids()
            .map(|parent_id| format!("{}", parent_id))
            .collect();

        let tree_git_hash = format!("{}", commit.tree()?.id());

        Ok(Self {
            git_hash: commit.id().to_string(),
            data: odb_obj.data().to_vec(),
            metadata: GitObjectMetadata::Commit {
                parent_git_hashes,
                tree_git_hash,
            },
        })
    }

    pub fn from_git_tag(tag: &Tag, odb: &Odb) -> Result<Self, Box<dyn Error>> {
        let odb_obj = odb.read(tag.id())?;

        Ok(Self {
            git_hash: tag.id().to_string(),
            data: odb_obj.data().to_vec(),
            metadata: GitObjectMetadata::Tag {
                target_git_hash: format!("{}", tag.target_id()),
            },
        })
    }

    pub fn from_git_tree(tree: &Tree, odb: &Odb) -> Result<Self, Box<dyn Error>> {
        let odb_obj = odb.read(tree.id())?;

        let entry_git_hashes: BTreeSet<String> =
            tree.iter().map(|entry| format!("{}", entry.id())).collect();

        Ok(Self {
            git_hash: tree.id().to_string(),
            data: odb_obj.data().to_vec(),
            metadata: GitObjectMetadata::Tree { entry_git_hashes },
        })
    }
}

#[derive(Encode, Decode, Debug, Clone)]
pub struct RepoData {
    /// All refs this repository knows; a {name -> sha1} map
    pub refs: BTreeMap<String, String>,
    /// All objects this repository contains; a {sha1 -> MultiObject hash} map
    pub objects: BTreeMap<String, String>,
}

impl RepoData {
    pub async fn from_ipfs(
        ipfs_cid: CidGeneric<32>,
        ipfs: &mut IpfsClient,
    ) -> Result<Self, Box<dyn Error>> {
        let refs_cid = ipfs_cid.to_string();
        let refs_content = ipfs
            .cat(&refs_cid)
            .map_ok(|c| c.to_vec())
            .try_concat()
            .await?;

        Ok(Self::decode(&mut refs_content.as_slice())?)
    }

    pub async fn push_ref_from_str(
        &mut self,
        ref_src: &str,
        ref_dst: &str,
        force: bool,
        repo: &mut Repository,
        ipfs: &mut IpfsClient,
        chain: &Chain,
        signer: &PairSigner<DefaultConfig, sp_keyring::sr25519::sr25519::Pair>,
        ips_id: u32,
    ) -> Result<NFTIdByChain, Box<dyn Error>> {
        // Deleting `ref_dst` was requested
        if ref_src.is_empty() {
            debug!("Removing ref {} from index", ref_dst);
            if self.refs.remove(ref_dst).is_none() {
                debug!(
                    "Nothing to delete, ref {} not part of the index ref set",
                    ref_dst
                );
                debug!("Available refs:\n{:#?}", self.refs);
            }
            return Err("ref_srd empty".into());
        }
        let reference = repo.find_reference(ref_src)?.resolve()?;

        // Differentiate between annotated tags and their commit representation
        let obj = reference
            .peel(ObjectType::Tag)
            .unwrap_or(reference.peel(ObjectType::Commit)?);

        debug!(
            "{:?} dereferenced to {:?} {}",
            reference.shorthand(),
            obj.kind(),
            obj.id()
        );

        if force {
            eprintln!("This push will be forced");
        } else {
            eprintln!("Checking for work ahead of us...");

            if let Some(dst_git_hash) = self.refs.get(ref_dst) {
                let mut missing_objects = HashSet::new();
                self.enumerate_for_fetch(
                    dst_git_hash.parse()?,
                    &mut missing_objects,
                    repo,
                    ipfs,
                    chain,
                    ips_id,
                )
                .await?;

                if !missing_objects.is_empty() {
                    eprintln!(
                        "There's {} objects in {} not present locally. Please fetch first or force-push.",
                        missing_objects.len(),
                        ref_dst
                        );

                    debug!("Missing objects:\n{:#?}", missing_objects);
                    return Err("There's objects in the index not present in the local repo - a pull is needed".into());
                }
            }
        }

        let mut objs_for_push = HashSet::new();
        let mut submodules_for_push = HashSet::new();

        self.enumerate_for_push(
            &obj.clone(),
            &mut objs_for_push,
            &mut submodules_for_push,
            repo,
        )?;

        let nft_id = self
            .push_git_objects(&objs_for_push, repo, ipfs, chain, signer)
            .await?;

        for submod_oid in submodules_for_push {
            self.objects
                .insert(submod_oid.to_string(), SUBMODULE_TIP_MARKER.to_owned());
        }

        self.refs
            .insert(ref_dst.to_owned(), format!("{}", obj.id()));
        Ok(nft_id)
    }

    pub fn enumerate_for_push(
        &self,
        obj: &Object,
        push_todo: &mut HashSet<Oid>,
        submodules: &mut HashSet<Oid>,
        repo: &Repository,
    ) -> Result<(), Box<dyn Error>> {
        // Object tree traversal state
        let mut stack = vec![obj.clone()];

        let mut obj_cnt = 1;
        while let Some(obj) = stack.pop() {
            if self.objects.contains_key(&obj.id().to_string()) {
                debug!("Object {} already in RepoData", obj.id());
                continue;
            }

            if push_todo.contains(&obj.id()) {
                debug!("Object {} already in state", obj.id());
                continue;
            }

            let obj_type = obj.kind().ok_or_else(|| {
                let msg = format!("Cannot determine type of object {}", obj.id());
                debug!("{}", msg);
                msg
            })?;

            push_todo.insert(obj.id());

            match obj_type {
                ObjectType::Commit => {
                    let commit = obj
                        .as_commit()
                        .ok_or_else(|| eprintln!("Could not view {:?} as a commit", obj))
                        .unwrap();
                    debug!("[{}] Counting commit {:?}", obj_cnt, commit);

                    let tree_obj = obj.peel(ObjectType::Tree)?;
                    debug!("Commit {}: Handling tree {}", commit.id(), tree_obj.id());

                    stack.push(tree_obj);

                    for parent in commit.parents() {
                        debug!(
                            "Commit {}: Pushing parent commit {}",
                            commit.id(),
                            parent.id()
                        );
                        stack.push(parent.into_object());
                    }
                }
                ObjectType::Tree => {
                    let tree = obj
                        .as_tree()
                        .ok_or_else(|| eprintln!("Could not view {:?} as a tree", obj))
                        .unwrap();
                    debug!("[{}] Counting tree {:?}", obj_cnt, tree);

                    for entry in tree.into_iter() {
                        // Weed out submodules (Implicitly known as commit children of tree objects)
                        if let Some(ObjectType::Commit) = entry.kind() {
                            debug!("Skipping submodule at {}", entry.id());

                            submodules.insert(entry.id());

                            continue;
                        }

                        debug!(
                            "Tree {}: Pushing tree entry {} ({:?})",
                            tree.id(),
                            entry.id(),
                            entry.kind()
                        );

                        stack.push(entry.to_object(repo)?);
                    }
                }
                ObjectType::Blob => {
                    let blob = obj
                        .as_blob()
                        .ok_or_else(|| eprintln!("Could not view {:?} as a blob", obj))
                        .unwrap();
                    debug!("[{}] Counting blob {:?}", obj_cnt, blob);
                }
                ObjectType::Tag => {
                    let tag = obj
                        .as_tag()
                        .ok_or_else(|| eprintln!("Could not view {:?} as a tag", obj))
                        .unwrap();
                    debug!("[{}] Counting tag {:?}", obj_cnt, tag);

                    stack.push(tag.target()?);
                }
                other => {
                    return Err(format!("Don't know how to traverse a {}", other).into());
                }
            }

            obj_cnt += 1;
        }
        Ok(())
    }

    pub async fn fetch_to_ref_from_str(
        &self,
        git_hash: &str,
        ref_name: &str,
        repo: &mut Repository,
        ipfs: &mut IpfsClient,
        chain: &Chain,
        ips_id: u32,
    ) -> Result<(), Box<dyn Error>> {
        debug!("Fetching {} for {}", git_hash, ref_name);

        let git_hash_oid = Oid::from_str(git_hash)?;
        let mut oids_for_fetch = HashSet::new();

        self.enumerate_for_fetch(git_hash_oid, &mut oids_for_fetch, repo, ipfs, chain, ips_id)
            .await?;

        self.fetch_git_objects(&oids_for_fetch, repo, ipfs, chain, ips_id)
            .await?;

        match repo.odb()?.read_header(git_hash_oid)?.1 {
            ObjectType::Commit if ref_name.starts_with("refs/tags") => {
                debug!("Not setting ref for lightweight tag {}", ref_name);
            }
            ObjectType::Commit => {
                repo.reference(ref_name, git_hash_oid, true, "inv4-git fetch")?;
            }
            // Somehow git is upset when we set tag refs for it
            ObjectType::Tag => {
                debug!("Not setting ref for tag {}", ref_name);
            }
            other_type => {
                let msg = format!("New tip turned out to be a {} after fetch", other_type);
                debug!("{}", msg);
                return Err(msg.into());
            }
        }

        debug!("Fetched {} for {} OK.", git_hash, ref_name);
        Ok(())
    }

    pub async fn enumerate_for_fetch(
        &self,
        oid: Oid,
        fetch_todo: &mut HashSet<Oid>,
        repo: &Repository,
        ipfs: &mut IpfsClient,
        chain: &Chain,
        ips_id: u32,
    ) -> Result<(), Box<dyn Error>> {
        let mut stack = vec![oid];

        while let Some(oid) = stack.pop() {
            if repo.odb()?.read_header(oid).is_ok() {
                debug!("Object {} already present locally!", oid);
                continue;
            }

            if fetch_todo.contains(&oid) {
                debug!("Object {} already present in state!", oid);
                continue;
            }

            let multi_object_hash = self
                .objects
                .get(&format!("{}", oid))
                .ok_or_else(|| {
                    let msg = format!("Could not find object {} in the index", oid);
                    debug!("{}", msg);
                    msg
                })?
                .clone();

            if multi_object_hash == SUBMODULE_TIP_MARKER {
                debug!("Ommitting submodule {}", oid.to_string());
                return Ok(());
            }

            fetch_todo.insert(oid);

            let multi_object =
                MultiObject::chain_get(multi_object_hash, ipfs, chain, ips_id).await?;

            match multi_object
                .objects
                .get(&oid.to_string())
                .expect("Oid not found in MultiObject")
                .clone()
                .metadata
            {
                GitObjectMetadata::Commit {
                    parent_git_hashes,
                    tree_git_hash,
                } => {
                    stack.push(Oid::from_str(&tree_git_hash)?);

                    for parent_git_hash in parent_git_hashes {
                        stack.push(Oid::from_str(&parent_git_hash)?);
                    }
                }
                GitObjectMetadata::Tag { target_git_hash } => {
                    stack.push(Oid::from_str(&target_git_hash)?);
                }
                GitObjectMetadata::Tree { entry_git_hashes } => {
                    for entry_git_hash in entry_git_hashes {
                        stack.push(Oid::from_str(&entry_git_hash)?);
                    }
                }
                GitObjectMetadata::Blob => {}
            }
        }

        Ok(())
    }

    pub async fn push_git_objects(
        &mut self,
        oids: &HashSet<Oid>,
        repo: &Repository,
        ipfs: &mut IpfsClient,
        chain: &Chain,
        signer: &PairSigner<DefaultConfig, sp_keyring::sr25519::sr25519::Pair>,
    ) -> Result<NFTIdByChain, Box<dyn Error>> {
        eprintln!("Minting 2 IPFs");

        let mut multi_object = MultiObject {
            hash: String::new(),
            git_hashes: vec![],
            objects: BTreeMap::new(),
        };

        for oid in oids {
            let obj = repo.find_object(*oid, None)?;
            debug!("Current object: {:?} at {}", obj.kind(), obj.id());

            if self.objects.contains_key(&obj.id().to_string()) {
                debug!("push_objects: Object {} already in RepoData", obj.id());
                continue;
            }

            let obj_type = obj.kind().ok_or_else(|| {
                let msg = format!("Cannot determine type of object {}", obj.id());
                debug!("{}", msg);
                msg
            })?;

            match obj_type {
                ObjectType::Commit => {
                    let commit = obj
                        .as_commit()
                        .ok_or_else(|| eprintln!("Could not view {:?} as a commit", obj))
                        .unwrap();
                    debug!("Pushing commit {:?}", commit);

                    multi_object.add(GitObject::from_git_commit(commit, &repo.odb()?)?);
                }
                ObjectType::Tree => {
                    let tree = obj
                        .as_tree()
                        .ok_or_else(|| eprintln!("Could not view {:?} as a tree", obj))
                        .unwrap();
                    debug!("Pushing tree {:?}", tree);

                    multi_object.add(GitObject::from_git_tree(tree, &repo.odb()?)?);
                }
                ObjectType::Blob => {
                    let blob = obj
                        .as_blob()
                        .ok_or_else(|| eprintln!("Could not view {:?} as a blob", obj))
                        .unwrap();
                    debug!("Pushing blob {:?}", blob);

                    multi_object.add(GitObject::from_git_blob(blob, &repo.odb()?)?);
                }
                ObjectType::Tag => {
                    let tag = obj
                        .as_tag()
                        .ok_or_else(|| eprintln!("Could not view {:?} as a tag", obj))
                        .unwrap();
                    debug!("Pushing tag {:?}", tag);

                    multi_object.add(GitObject::from_git_tag(tag, &repo.odb()?)?);
                }
                other => {
                    return Err(format!("Don't know how to traverse a {}", other).into());
                }
            }
        }

        multi_object.hash = xxh3::hash64(multi_object.git_hashes.encode().as_slice()).to_string();

        for oid in multi_object.git_hashes.clone() {
            self.objects.insert(oid, multi_object.hash.clone());
        }

        debug!("Pushing MultiObject to IPFS");
        let cid =
            CidGeneric::<32>::try_from(ipfs.add(Cursor::new(multi_object.encode())).await?.hash)?;

        debug!("Sending MultiObject to the chain");
        let ipf_id = chain
            .mint_nft(multi_object.hash.as_bytes().to_vec(), cid, signer)
            .await?;

        eprintln!("Minted Git Objects on-chain with IPF ID: {}", ipf_id);

        Ok(ipf_id)
    }

    /// Download git objects in `oids` from IPFS and instantiate them in `repo`.
    pub async fn fetch_git_objects(
        &self,
        oids: &HashSet<Oid>,
        repo: &mut Repository,
        ipfs: &mut IpfsClient,
        chain: &Chain,
        ips_id: u32,
    ) -> Result<(), Box<dyn Error>> {
        let mut fetched_objects = BTreeMap::new();

        let objects_deduped = {
            let mut o = self.objects.values().collect::<Vec<&String>>();
            o.sort();
            o.dedup();
            o
        };

        eprintln!("past here");

        for object_hash in objects_deduped {
            let mut multi_object =
                MultiObject::chain_get(object_hash.clone(), ipfs, chain, ips_id).await?;

            fetched_objects.append(&mut multi_object.objects)
        }

        eprintln!("past this");

        for (i, &oid) in oids.iter().enumerate() {
            debug!("[{}/{}] Fetching object {}", i + 1, oids.len(), oid);

            let git_object = fetched_objects
                .get(&format!("{}", oid))
                .ok_or_else(|| {
                    let msg = format!("Could not find object {} in the index", oid);
                    debug!("{}", msg);
                    msg
                })?
                .clone();

            if repo.odb()?.read_header(oid).is_ok() {
                debug!("fetch objects: Object {} already present locally!", oid);
                continue;
            }

            let written_oid = repo.odb()?.write(
                match git_object.metadata {
                    GitObjectMetadata::Blob => ObjectType::Blob,
                    GitObjectMetadata::Commit { .. } => ObjectType::Commit,
                    GitObjectMetadata::Tag { .. } => ObjectType::Tag,
                    GitObjectMetadata::Tree { .. } => ObjectType::Tree,
                },
                &git_object.data,
            )?;
            if written_oid != oid {
                let msg = format!(
                    "Object tree inconsistency detected: fetched {}, but write result hashes to {}",
                    oid, written_oid
                );
                debug!("{}", msg);
                return Err(msg.into());
            }
            debug!("Fetched object {}", written_oid);
        }
        Ok(())
    }

    pub async fn mint_return_new_old_id(
        &self,
        ipfs: &mut IpfsClient,
        chain: &Chain,
        signer: &PairSigner<DefaultConfig, sp_keyring::sr25519::sr25519::Pair>,
        ips_id: u32,
    ) -> Result<(NFTIdByChain, Option<NFTIdByChain>), Box<dyn Error>> {
        let new_nft_id = chain
            .mint_nft(
                b"RepoData".to_vec(),
                CidGeneric::<32>::try_from(ipfs.add(Cursor::new(self.encode())).await?.hash)?,
                signer,
            )
            .await?;

        eprintln!("Minted Repo Data on-chain with IPF ID: {}", new_nft_id);

        let ips_data = chain.ips_data(ips_id).await?;

        for file in ips_data {
            if let Some(old_nft_id) = file.nft_id() {
                let nft_info = chain.nft_info(&old_nft_id).await?;
                if String::from_utf8(nft_info.metadata())? == *"RepoData" {
                    return Ok((new_nft_id, Some(old_nft_id)));
                }
            }
        }

        Ok((new_nft_id, None))
    }
}
