#![allow(clippy::too_many_arguments)]
use sp_keyring::{sr25519::sr25519::Pair, AccountKeyring::Alice};
use std::{cmp::Ordering, error::Error};
use subxt::{subxt, DefaultConfig, DefaultExtra, PairSigner};

use crate::primitives::{GitRef, Settings};

#[subxt(runtime_metadata_path = "invarch_metadata.scale")]
pub mod invarch {}

pub struct GitArchClient {
    pub signer: PairSigner<DefaultConfig, DefaultExtra<DefaultConfig>, Pair>,
}

impl Default for GitArchClient {
    fn default() -> Self {
        Self {
            signer: PairSigner::<DefaultConfig, DefaultExtra<DefaultConfig>, Pair>::new(
                Alice.pair(),
            ),
        }
    }
}

impl GitArchClient {
    async fn _fetch(&self, _settings: Settings, _git_ref: GitRef) -> Result<(), Box<dyn Error>> {
        todo!()
    }
    async fn _push(&self, _settings: Settings, _local_ref: GitRef) -> Result<(), Box<dyn Error>> {
        todo!()
    }
}
