use cid::CidGeneric;
use multihash::MultihashGeneric;
use subxt::sp_core::H256;

use crate::primitives::BoxResult;

#[macro_export]
macro_rules! error {
    ($x:expr) => {{
        return Err($x.into());
    }};
}

pub fn generate_cid(hash: H256) -> BoxResult<ipfs::Cid> {
    Ok(CidGeneric::new_v0(MultihashGeneric::from_bytes(
        hex::decode(format!("{:?}", hash).replace("0x", "1220"))?,
    )?)?)
}
