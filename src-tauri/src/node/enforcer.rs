//! The enforcer's gRPC (BIP300/301, L2L's bip300301_enforcer, which BitWindow runs). The app asks it three things:
//! is it there and where is its chain, how much its wallet holds, and to make a deposit to this app's Truthcoin
//! address (BIP300 M5). The messages are written here from the published field numbers.

use std::time::Duration;
use tonic::codegen::http::uri::PathAndQuery;

#[derive(Clone, PartialEq, prost::Message)]
pub struct Empty {}

#[derive(Clone, PartialEq, prost::Message)]
pub struct StringValue {
    #[prost(string, tag = "1")]
    pub value: String,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct UInt32Value {
    #[prost(uint32, tag = "1")]
    pub value: u32,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct UInt64Value {
    #[prost(uint64, tag = "1")]
    pub value: u64,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ReverseHex {
    #[prost(message, optional, tag = "1")]
    pub hex: Option<StringValue>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct GetChainInfoResponse {
    #[prost(int32, tag = "1")]
    pub network: i32,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct BlockHeaderInfo {
    #[prost(message, optional, tag = "1")]
    pub block_hash: Option<ReverseHex>,
    #[prost(uint32, tag = "3")]
    pub height: u32,
    #[prost(uint64, tag = "5")]
    pub timestamp: u64,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct GetChainTipResponse {
    #[prost(message, optional, tag = "1")]
    pub block_header_info: Option<BlockHeaderInfo>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct GetBalanceResponse {
    #[prost(uint64, tag = "1")]
    pub confirmed_sats: u64,
    #[prost(uint64, tag = "2")]
    pub pending_sats: u64,
    #[prost(bool, tag = "3")]
    pub has_synced: bool,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct CreateDepositTransactionRequest {
    #[prost(message, optional, tag = "1")]
    pub sidechain_id: Option<UInt32Value>,
    #[prost(message, optional, tag = "2")]
    pub address: Option<StringValue>,
    #[prost(message, optional, tag = "3")]
    pub value_sats: Option<UInt64Value>,
    #[prost(message, optional, tag = "4")]
    pub fee_sats: Option<UInt64Value>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct CreateDepositTransactionResponse {
    #[prost(message, optional, tag = "1")]
    pub txid: Option<ReverseHex>,
}

/// Truthcoin's sidechain slot.
pub const TRUTHCOIN_SLOT: u32 = 13;

/// eCash beta's fork block: a release build runs only on a chain that has exactly this block at this height.
pub const BETA_FORK_HEIGHT: u32 = 967_680;
pub const BETA_FORK_HASH: &str = "00000000000000030101ba5cfea54b22becc79f95dc6040beb76e01dd9d04042";

#[derive(Clone, PartialEq, prost::Message)]
pub struct GetBlockHeaderInfoRequest {
    #[prost(message, optional, tag = "1")]
    pub block_hash: Option<ReverseHex>,
    #[prost(uint32, optional, tag = "2")]
    pub max_ancestors: Option<u32>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct GetBlockHeaderInfoResponse {
    #[prost(message, repeated, tag = "1")]
    pub header_infos: Vec<BlockHeaderInfo>,
}

/// Is the enforcer following eCash beta? Ok(()) when its chain has the beta fork block at its height; otherwise why
/// not, in words for the screen.
pub async fn check_ecash_beta(addr: &str) -> Result<(), String> {
    let (tip, network) = chain(addr).await?;
    // eCash beta reports itself as mainnet (a fork of Bitcoin's chain); anything else is another chain outright.
    if network != "mainnet" {
        return Err(format!("This release runs only on eCash beta, and the enforcer is on a {network} chain."));
    }
    let req = GetBlockHeaderInfoRequest {
        block_hash: Some(ReverseHex { hex: Some(StringValue { value: BETA_FORK_HASH.into() }) }),
        max_ancestors: Some(0),
    };
    let r: Result<GetBlockHeaderInfoResponse, String> =
        unary(addr, "/cusf.mainchain.v1.ValidatorService/GetBlockHeaderInfo", req, Duration::from_secs(10)).await;
    let found = r.ok().and_then(|r| r.header_infos.into_iter().next());
    match found {
        Some(h) if h.height == BETA_FORK_HEIGHT => Ok(()),
        _ if tip < BETA_FORK_HEIGHT => Err(format!(
            "The enforcer is at eCash block {tip}; this release runs only on eCash beta, from block {BETA_FORK_HEIGHT}. \
             Wait for it to catch up."
        )),
        _ => Err("This release runs only on eCash beta, and the enforcer is following another chain.".into()),
    }
}

async fn unary<Req, Resp>(addr: &str, path: &'static str, req: Req, timeout: Duration) -> Result<Resp, String>
where
    Req: prost::Message + Send + Sync + 'static,
    Resp: prost::Message + Default + Send + Sync + 'static,
{
    let ch = tonic::transport::Endpoint::from_shared(format!("http://{addr}"))
        .map_err(|e| e.to_string())?
        .connect_timeout(Duration::from_secs(4))
        .timeout(timeout)
        .connect()
        .await
        .map_err(|_| format!("nothing answers at {addr}"))?;
    let mut g = tonic::client::Grpc::new(ch);
    g.ready().await.map_err(|e| e.to_string())?;
    let codec = tonic::codec::ProstCodec::<Req, Resp>::default();
    g.unary(tonic::Request::new(req), PathAndQuery::from_static(path), codec)
        .await
        .map(|r| r.into_inner())
        .map_err(|s| if s.message().is_empty() { format!("{:?}", s.code()) } else { s.message().to_string() })
}

pub fn network_name(n: i32) -> &'static str {
    match n {
        2 => "mainnet",
        3 => "regtest",
        4 => "signet",
        5 => "testnet",
        _ => "unknown",
    }
}

/// The enforcer's chain tip height, and its network.
pub async fn chain(addr: &str) -> Result<(u32, String), String> {
    let tip: GetChainTipResponse =
        unary(addr, "/cusf.mainchain.v1.ValidatorService/GetChainTip", Empty {}, Duration::from_secs(8)).await?;
    let info: GetChainInfoResponse =
        unary(addr, "/cusf.mainchain.v1.ValidatorService/GetChainInfo", Empty {}, Duration::from_secs(8)).await?;
    Ok((tip.block_header_info.map(|b| b.height).unwrap_or(0), network_name(info.network).to_string()))
}

/// The enforcer wallet's balance (BitWindow's eCash wallet): confirmed and pending sats, and whether it has synced.
pub async fn balance(addr: &str) -> Result<GetBalanceResponse, String> {
    unary(addr, "/cusf.mainchain.v1.WalletService/GetBalance", Empty {}, Duration::from_secs(20)).await
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct CreateNewAddressResponse {
    #[prost(string, tag = "1")]
    pub address: String,
}

/// A new receiving address of the enforcer's wallet (BitWindow's eCash wallet).
pub async fn new_address(addr: &str) -> Result<String, String> {
    let r: CreateNewAddressResponse =
        unary(addr, "/cusf.mainchain.v1.WalletService/CreateNewAddress", Empty {}, Duration::from_secs(20)).await?;
    if r.address.is_empty() {
        return Err("the enforcer gave no address".into());
    }
    Ok(r.address)
}

/// Deposit `value_sats` from the enforcer's wallet to `address` (a Truthcoin address of this wallet, not the
/// `s13_…` deposit form), paying `fee_sats` on eCash. Returns the eCash txid.
pub async fn deposit(addr: &str, address: &str, value_sats: u64, fee_sats: u64) -> Result<String, String> {
    let req = CreateDepositTransactionRequest {
        sidechain_id: Some(UInt32Value { value: TRUTHCOIN_SLOT }),
        address: Some(StringValue { value: address.to_string() }),
        value_sats: Some(UInt64Value { value: value_sats }),
        fee_sats: Some(UInt64Value { value: fee_sats }),
    };
    let r: CreateDepositTransactionResponse =
        unary(addr, "/cusf.mainchain.v1.WalletService/CreateDepositTransaction", req, Duration::from_secs(60)).await?;
    Ok(r.txid.and_then(|t| t.hex).map(|h| h.value).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    /// Against a real enforcer: BETA_ENFORCER (on eCash beta: passes) and OTHER_ENFORCER (a dev chain: refused).
    /// `BETA_ENFORCER=host:50051 OTHER_ENFORCER=127.0.0.1:23403 cargo test -j2 beta_check -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn beta_check() {
        if let Some(a) = std::env::var_os("BETA_ENFORCER") {
            let r = super::check_ecash_beta(a.to_str().unwrap()).await;
            eprintln!("beta enforcer: {r:?}");
            assert_eq!(r, Ok(()));
        }
        if let Some(a) = std::env::var_os("OTHER_ENFORCER") {
            let r = super::check_ecash_beta(a.to_str().unwrap()).await;
            eprintln!("other enforcer: {r:?}");
            assert!(r.is_err());
        }
    }
}
