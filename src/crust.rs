use crate::compression;
use crate::primitives::BoxResult;
use serde::{Deserialize, Serialize};
use subxt::ext::sp_core::sr25519::Pair as Sr25519Pair;
use subxt::ext::sp_core::Pair;
use subxt::{tx::PairSigner, PolkadotConfig};

#[derive(Debug, Serialize, Deserialize)]
pub struct ResponseAdd {
    #[serde(alias = "Hash")]
    hash: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RequestPin {
    cid: String,
    name: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ResponsePin {
    status: String,
}

pub async fn send_to_crust(
    signer: &PairSigner<PolkadotConfig, Sr25519Pair>,
    data: Vec<u8>,
) -> BoxResult<String> {
    let signature = hex::encode(
        signer
            .signer()
            .sign(signer.account_id().to_string().as_bytes())
            .0,
    );
    let base64 = base64::encode(format!(
        "sub-{}:0x{}",
        signer.account_id().to_string(),
        signature
    ));

    let client = reqwest::Client::new();

    let cid = client
        .post("https://crustwebsites.net/api/v0/add")
        .header("Authorization", format!("Basic {}", base64))
        .multipart(
            reqwest::multipart::Form::new().part("file", reqwest::multipart::Part::bytes(data)),
        )
        .send()
        .await?
        .json::<ResponseAdd>()
        .await?
        .hash;

    if client
        .post("https://pin.crustcode.com/psa/pins")
        .header("Authorization", format!("Bearer {}", base64))
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&RequestPin {
            name: cid.clone(),
            cid: cid.clone(),
        })?)
        .send()
        .await?
        .json::<ResponsePin>()
        .await?
        .status
        != "queued"
    {
        return Err("error".into());
    }

    Ok(cid)
}

pub async fn get_from_crust(cid: String) -> BoxResult<Vec<u8>> {
    let client = reqwest::Client::new();

    let data = client
        .get(format!("https://crustwebsites.net/ipfs/{}", cid))
        .send()
        .await?
        .bytes()
        .await?
        .to_vec();

    Ok(data)
}
