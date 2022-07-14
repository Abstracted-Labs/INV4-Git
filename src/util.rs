use cid::{multihash::MultihashGeneric, CidGeneric};
use subxt::sp_core::H256;

use crate::primitives::BoxResult;

#[macro_export]
macro_rules! error {
    ($x:expr) => {{
        return Err($x.into());
    }};
}

pub fn generate_cid(hash: H256) -> BoxResult<CidGeneric<32>> {
    Ok(CidGeneric::new_v0(MultihashGeneric::<32>::from_bytes(
        hex::decode(format!("{:?}", hash).replace("0x", "1220"))?.as_slice(),
    )?)?)
}
