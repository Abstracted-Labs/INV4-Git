use crate::{
    error,
    invarch::{self, runtime_types::pallet_inv4::pallet::AnyId},
    util::generate_cid,
};
use cid::Cid;
use codec::{Decode, Encode};
use futures::TryStreamExt;
use git2::{Blob, Commit, Object, ObjectType, Odb, Oid, Repository, Tag, Tree};
use ipfs_api::{IpfsApi, IpfsClient};
use log::debug;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    error::Error,
    io::Cursor,
};
use subxt::{sp_core::H256, DefaultConfig, PairSigner, PolkadotExtrinsicParams};
use twox_hash::xxh3;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Config {
    pub chain_endpoint: String,
}

/// A magic value used to signal that a hash is a submodule tip (to be obtained by git on its own).
pub static SUBMODULE_TIP_MARKER: &str = "submodule-tip";

pub type BoxResult<T> = Result<T, Box<dyn Error>>;

/// Holds all git objects in a given repository???
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
        chain_api: &invarch::RuntimeApi<DefaultConfig, PolkadotExtrinsicParams<DefaultConfig>>,
        ips_id: u32,
    ) -> Result<Self, Box<dyn Error>> {
        let ips_info = chain_api
            .storage()
            .inv4()
            .ip_storage(&ips_id, None)
            .await?
            .ok_or(format!("IPS {ips_id} does not exist"))?;

        for file in ips_info.data.0 {
            if let AnyId::IpfId(id) = file {
                let ipf_info = chain_api
                    .storage()
                    .ipf()
                    .ipf_storage(&id, None)
                    .await?
                    .ok_or("Internal error: IPF listed from IPS does not exist")?;
                if String::from_utf8(ipf_info.metadata.0.clone())? == *hash {
                    return Ok(Self::decode(
                        &mut ipfs
                            .cat(&generate_cid(ipf_info.data.0.into())?.to_string())
                            .map_ok(|c| c.to_vec())
                            .try_concat()
                            .await?
                            .as_slice(),
                    )?);
                }
            }
        }
        error!("git_hash ipf not found")
    }
}

/// Represents a git object. Types are Commit, Tag, Tree, & Blob
/// Ex in filesystem: .git/objects/4b/62c9e0f3c6550c17af27daa0b24a194e113374
#[derive(Clone, Debug, Encode, Decode)]
pub struct GitObject {
    /// The git hash of the underlying git object
    pub git_hash: String,
    /// A link to the raw form of the object
    pub data: Vec<u8>,
    /// Object-type-specific metadata
    pub metadata: GitObjectMetadata,
}

// Valid git object types
#[derive(Clone, Debug, Encode, Decode)]
pub enum GitObjectMetadata {
    /// References tree and its parent commit
    Commit {
        parent_git_hashes: BTreeSet<String>,
        tree_git_hash: String,
    },
    /// References a specific commit
    Tag { target_git_hash: String },
    /// References blobs and/or other trees
    Tree { entry_git_hashes: BTreeSet<String> },
    /// The actual files of the repo i.e. .html, .js, .pdf, etc.
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

/// Top level repository data
#[derive(Encode, Decode, Debug, Clone)]
pub struct RepoData {
    /// All refs this repository knows; a {branch name -> sha1 (commit hash???)} map
    /// i.e. branches
    pub refs: BTreeMap<String, String>,
    /// All objects this repository contains; a {sha1 (commit hash???) -> MultiObject hash} map
    pub objects: BTreeMap<String, String>,
}

impl RepoData {
    pub async fn from_ipfs(ipfs_hash: H256, ipfs: &mut IpfsClient) -> Result<Self, Box<dyn Error>> {
        let refs_cid = generate_cid(ipfs_hash)?.to_string();
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
        chain_api: &invarch::RuntimeApi<DefaultConfig, PolkadotExtrinsicParams<DefaultConfig>>,
        signer: &PairSigner<DefaultConfig, sp_keyring::sr25519::sr25519::Pair>,
        ips_id: u32,
    ) -> Result<u64, Box<dyn Error>> {
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
            error!("ref_srd empty")
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
                    chain_api,
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

        let ipf_id = self
            .push_git_objects(&objs_for_push, repo, ipfs, chain_api, signer)
            .await?;

        for submod_oid in submodules_for_push {
            self.objects
                .insert(submod_oid.to_string(), SUBMODULE_TIP_MARKER.to_owned());
        }

        self.refs
            .insert(ref_dst.to_owned(), format!("{}", obj.id()));
        Ok(ipf_id)
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
        chain_api: &invarch::RuntimeApi<DefaultConfig, PolkadotExtrinsicParams<DefaultConfig>>,
        ips_id: u32,
    ) -> Result<(), Box<dyn Error>> {
        debug!("Fetching {} for {}", git_hash, ref_name);

        let git_hash_oid = Oid::from_str(git_hash)?;
        let mut oids_for_fetch = HashSet::new();

        self.enumerate_for_fetch(
            git_hash_oid,
            &mut oids_for_fetch,
            repo,
            ipfs,
            chain_api,
            ips_id,
        )
        .await?;

        self.fetch_git_objects(&oids_for_fetch, repo, ipfs, chain_api, ips_id)
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
        chain_api: &invarch::RuntimeApi<DefaultConfig, PolkadotExtrinsicParams<DefaultConfig>>,
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
                MultiObject::chain_get(multi_object_hash, ipfs, chain_api, ips_id).await?;

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
        chain_api: &invarch::RuntimeApi<DefaultConfig, PolkadotExtrinsicParams<DefaultConfig>>,
        signer: &PairSigner<DefaultConfig, sp_keyring::sr25519::sr25519::Pair>,
    ) -> Result<u64, Box<dyn Error>> {
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
        // Actually push data to IPFS and get the unique hash back
        // First 2 bytes are multihash metadata and are excluded b/c not part of the actual hash (digest)
        let ipfs_hash = &Cid::try_from(ipfs.add(Cursor::new(multi_object.encode())).await?.hash)?
            .to_bytes()[2..];

        debug!("Sending MultiObject to the chain");
        let events = chain_api
            .tx()
            .ipf()
            .mint(
                multi_object.hash.as_bytes().to_vec(),
                H256::from_slice(ipfs_hash),
            )?
            .sign_and_submit_then_watch_default(signer)
            .await?
            .wait_for_in_block()
            .await?;

        let ipf_id = events
            .fetch_events()
            .await?
            .find_first::<invarch::ipf::events::Minted>()?
            .unwrap()
            .1;

        events.wait_for_success().await?;

        eprintln!("Minted Git Objects on-chain with IPF ID: {}", ipf_id);

        Ok(ipf_id)
    }

    /// Download git objects in `oids` from IPFS and instantiate them in `repo`.
    pub async fn fetch_git_objects(
        &self,
        oids: &HashSet<Oid>,
        repo: &mut Repository,
        ipfs: &mut IpfsClient,
        chain_api: &invarch::RuntimeApi<DefaultConfig, PolkadotExtrinsicParams<DefaultConfig>>,
        ips_id: u32,
    ) -> Result<(), Box<dyn Error>> {
        let mut fetched_objects = BTreeMap::new();

        let objects_deduped = {
            let mut o = self.objects.values().collect::<Vec<&String>>();
            o.sort();
            o.dedup();
            o
        };

        for object_hash in objects_deduped {
            let mut multi_object =
                MultiObject::chain_get(object_hash.clone(), ipfs, chain_api, ips_id).await?;

            fetched_objects.append(&mut multi_object.objects)
        }

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

    /// Mint new/updated RepoData file. 
    /// Returns IPF ID of new file and Option holding ID of potential pre-existing file
    pub async fn mint_return_new_old_id(
        &self,
        ipfs: &mut IpfsClient,
        chain_api: &invarch::RuntimeApi<DefaultConfig, PolkadotExtrinsicParams<DefaultConfig>>,
        signer: &PairSigner<DefaultConfig, sp_keyring::sr25519::sr25519::Pair>,
        ips_id: u32,
    ) -> Result<(u64, Option<u64>), Box<dyn Error>> {
        // Mint `RepoData` instance as a new IPF
        let events = chain_api
            .tx()
            .ipf()
            .mint(
                b"RepoData".to_vec(),
                H256::from_slice(
                    &Cid::try_from(ipfs.add(Cursor::new(self.encode())).await?.hash)?.to_bytes()
                        [2..],
                ),
            )?
            .sign_and_submit_then_watch_default(signer)
            .await?
            .wait_for_in_block()
            .await?;

        // Get ID of new IPF just minted
        let new_ipf_id = events
            .fetch_events()
            .await?
            .find_first::<invarch::ipf::events::Minted>()?
            .unwrap()
            .1;

        events.wait_for_success().await?;

        eprintln!("Minted Repo Data on-chain with IPF ID: {}", new_ipf_id);

        // Get IPS info
        let ips_info = chain_api
            .storage()
            .inv4()
            .ip_storage(&ips_id, None)
            .await?
            .ok_or(format!("IPS {ips_id} does not exist"))?;

        // Check if IPS has a pre-existing RepoData file
        for file in ips_info.data.0 {
            if let AnyId::IpfId(id) = file {
                let ipf_info = chain_api
                    .storage()
                    .ipf()
                    .ipf_storage(&id, None)
                    .await?
                    .ok_or("Internal error: IPF listed from IPS does not exist")?;
                if String::from_utf8(ipf_info.metadata.0.clone())? == *"RepoData" {
                    return Ok((new_ipf_id, Some(id)));
                }
            }
        }

        // IPS doesn't have a pre-existing RepoData file
        Ok((new_ipf_id, None))
    }
}
