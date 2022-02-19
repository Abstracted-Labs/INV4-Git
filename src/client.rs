#![allow(clippy::too_many_arguments)]
use sp_keyring::{sr25519::sr25519::Pair, AccountKeyring::Alice};
use std::{cmp::Ordering, error::Error};
use subxt::{subxt, DefaultConfig, DefaultExtra, PairSigner};

use crate::primitives::{GitRef, Settings};

#[subxt(runtime_metadata_path = "invarch_metadata.scale")]
pub mod invarch {}

pub type Id = invarch::runtime_types::polkadot_parachain::primitives::Id;

// TEMP: Workaround for parachain ID issue #315 on paritytech/subxt
impl PartialEq for Id {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for Id {}

impl PartialOrd for Id {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.0.partial_cmp(&other.0)
    }
}

impl Ord for Id {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0)
    }
}

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
