//! Markets and trading, shared by the desktop's screens and the phone link.
//!
//! What the node taught us (truthcoin_dc v0.19.0 and v0.20.0, checked on a private chain):
//! - A buy's cap (`max_cost`) must cover the quote **and** the 1,000-sat miner fee: v0.19.0's RPC accepted a cap that
//!   only covered the quote, and its block builder then skipped the trade every block, with the coin it spent tied
//!   up; v0.20.0 refuses such a cap. So every cap here is at least quote + miner fee + a margin for the price moving.
//! - A trade spends a whole coin and has no outputs until its block: while it waits the balance can look empty.
//! - Shares land at an address the node picks, so positions are read for every wallet address, and a sell names the
//!   address that holds the shares.
//! - A quote is a guide; the price is fixed when the block is built. Screens show "about X, at most Y".

use crate::rpc::Rpc;
use crate::trades::{Side, Status, Trade, Trades};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// The miner fee every trade pays, in sats (truthcoin_dc v0.19.0 and v0.20.0).
pub const MINER_FEE: u64 = 1_000;
/// The largest trade the app places, in share units (1 sat each if they win): 1,000 coins.
pub const MAX_SHARES: u64 = 100_000_000_000;

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct MarketSummary {
    pub market_id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub outcome_count: u32,
    pub state: String,
    pub volume_sats: u64,
    pub created_at_height: u64,
}

pub async fn list(rpc: &Rpc) -> Result<Vec<MarketSummary>, String> {
    Ok(rpc.public("market_list", json!([])).await?)
}

/// `market_get`, as the node gives it (null when there's no such market).
pub async fn get(rpc: &Rpc, id: &str) -> Result<Value, String> {
    if !valid_market_id(id) {
        return Err("not a market id".into());
    }
    let v: Value = rpc.public("market_get", json!([id])).await?;
    if v.is_null() {
        return Err("no such market".into());
    }
    Ok(v)
}

pub fn valid_market_id(id: &str) -> bool {
    id.len() == 12 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

/// An outcome's label and price from `market_get`.
pub fn outcome(m: &Value, index: u32) -> Option<(String, f64)> {
    m["outcomes"].as_array()?.iter().find(|o| o["outcome_index"].as_u64() == Some(index as u64)).map(|o| {
        (short_label(m, o["label"].as_str().unwrap_or("")), o["price"].as_f64().unwrap_or(0.0))
    })
}

/// "Rain by period 7: Yes" reads "Yes" in a one-question market.
fn short_label(m: &Value, label: &str) -> String {
    if m["dimensions"].as_array().map(|d| d.len()) == Some(1) {
        if let Some((_, tail)) = label.rsplit_once(": ") {
            return tail.to_string();
        }
    }
    label.to_string()
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct Holding {
    pub market_id: String,
    pub market_title: String,
    pub market_state: String,
    pub outcome: u32,
    pub outcome_label: String,
    pub shares: u64,
    /// Shares per wallet address (a sell takes them from one address at a time).
    pub by_address: Vec<(String, u64)>,
    pub price: f64,
    /// What the shares are worth at today's price, in sats.
    pub value_sats: u64,
    /// What the app's own record says was paid for shares of this outcome still held (buys minus sells, miner fees
    /// included, never below zero), when it has one.
    pub paid_sats: Option<u64>,
}

/// The last positions read, for a few seconds: each read costs one node call per wallet address (review L6).
static HOLDINGS: std::sync::Mutex<Option<(std::time::Instant, Vec<Holding>)>> = std::sync::Mutex::new(None);

pub fn forget_holdings() {
    *HOLDINGS.lock().unwrap() = None;
}

/// Every position the wallet holds, across all its addresses.
pub async fn holdings(rpc: &Rpc, trades: &Trades) -> Result<Vec<Holding>, String> {
    if let Some((t, h)) = HOLDINGS.lock().unwrap().as_ref() {
        if t.elapsed() < std::time::Duration::from_secs(5) {
            return Ok(h.clone());
        }
    }
    let h = read_holdings(rpc, trades).await?;
    *HOLDINGS.lock().unwrap() = Some((std::time::Instant::now(), h.clone()));
    Ok(h)
}

async fn read_holdings(rpc: &Rpc, trades: &Trades) -> Result<Vec<Holding>, String> {
    let _ = refresh(rpc, trades).await;
    let addrs: Vec<String> = rpc.private("get_wallet_addresses", json!([])).await?;
    let mut by: BTreeMap<(String, u32), Holding> = BTreeMap::new();
    for a in &addrs {
        let h: Value = rpc.public("market_positions", json!([a, null])).await?;
        for p in h["positions"].as_array().into_iter().flatten() {
            let shares = p["shares"].as_f64().unwrap_or(0.0).max(0.0) as u64;
            if shares == 0 {
                continue;
            }
            let id = p["market_id"].as_str().unwrap_or("").to_string();
            let oi = p["outcome_index"].as_u64().unwrap_or(0) as u32;
            let e = by.entry((id.clone(), oi)).or_insert_with(|| Holding { market_id: id, outcome: oi, ..Default::default() });
            e.shares += shares;
            e.by_address.push((a.clone(), shares));
        }
    }
    let mut cache: BTreeMap<String, Value> = BTreeMap::new();
    let mut out = vec![];
    for ((id, oi), mut h) in by {
        if !cache.contains_key(&id) {
            cache.insert(id.clone(), get(rpc, &id).await.unwrap_or(Value::Null));
        }
        let m = &cache[&id];
        h.market_title = m["title"].as_str().unwrap_or("").to_string();
        h.market_state = m["state"].as_str().unwrap_or("").to_string();
        if let Some((label, price)) = outcome(m, oi) {
            h.outcome_label = label;
            h.price = price;
        }
        h.value_sats = (h.shares as f64 * h.price).round() as u64;
        h.by_address.sort_by(|a, b| b.1.cmp(&a.1));
        h.paid_sats = paid(trades, &id, oi);
        out.push(h);
    }
    Ok(out)
}

/// Net paid for an outcome by the app's own record of completed trades.
fn paid(trades: &Trades, market: &str, outcome: u32) -> Option<u64> {
    let mut any = false;
    let mut net: i128 = 0;
    for t in trades.all().iter().filter(|t| t.market_id == market && t.outcome == outcome && t.status == Status::Done) {
        any = true;
        // The miner fee comes out of the wallet's coin: a buy costs it on top, a sell brings that much less.
        match t.side {
            Side::Buy => net += t.quoted_sats as i128 + MINER_FEE as i128,
            Side::Sell => net -= t.quoted_sats as i128 - MINER_FEE as i128,
        }
    }
    any.then(|| net.max(0) as u64)
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Quote {
    pub side: Side,
    pub market_id: String,
    pub market_title: String,
    pub outcome: u32,
    pub outcome_label: String,
    pub shares: u64,
    /// Buy: what it costs, trading fee included. Sell: what you get, trading fee taken off.
    pub sats: u64,
    pub trading_fee_sats: u64,
    pub miner_fee_sats: u64,
    pub price_now: f64,
    pub price_after: f64,
    /// The cap the trade will carry: at most this (buy, miner fee included), at least this (sell).
    pub limit_sats: u64,
    /// Sell: the address the shares come from.
    pub seller_address: Option<String>,
}

/// The margin a suggested cap leaves for the price moving before the block: 2%, at least 1,000 sats.
pub fn margin(sats: u64) -> u64 {
    (sats / 50).max(1_000)
}

pub async fn quote(rpc: &Rpc, market_id: &str, outcome_index: u32, shares: u64, side: Side) -> Result<Quote, String> {
    if shares == 0 || shares > MAX_SHARES {
        return Err("Pick an amount of shares above zero".into());
    }
    let m = get(rpc, market_id).await?;
    if m["state"].as_str() != Some("trading") {
        return Err(format!("This market isn't trading (it is {})", m["state"].as_str().unwrap_or("unknown")));
    }
    let (label, price_now) = outcome(&m, outcome_index).ok_or("This market has no such outcome")?;
    let title = m["title"].as_str().unwrap_or("").to_string();
    match side {
        Side::Buy => {
            let q: Value = rpc
                .private(
                    "market_buy",
                    json!([{"market_id": market_id, "outcome_index": outcome_index, "shares_amount": shares, "dry_run": true}]),
                )
                .await?;
            let cost = q["cost_sats"].as_u64().ok_or("the node's quote has no cost")?;
            let fee = q["trading_fee_sats"].as_u64().unwrap_or(0);
            let after = q["new_price"].as_f64().unwrap_or(price_now);
            // A share pays at most 1 sat, so shares can't cost more than their number (plus the fee), buying can't
            // lower the price, and the fee is the market's rate (at least 1,000 sats). A quote that says otherwise
            // isn't trusted (review N2).
            if cost.saturating_sub(fee) > shares || after + 1e-9 < price_now || !fee_ok(&m, cost, fee) {
                return Err("The node's quote doesn't add up; not trading on it".into());
            }
            Ok(Quote {
                side,
                market_id: market_id.into(),
                market_title: title,
                outcome: outcome_index,
                outcome_label: label,
                shares,
                sats: cost,
                trading_fee_sats: fee,
                miner_fee_sats: MINER_FEE,
                price_now,
                price_after: after,
                limit_sats: cost.saturating_add(MINER_FEE).saturating_add(margin(cost)),
                seller_address: None,
            })
        }
        Side::Sell => {
            let holder = holder_of(rpc, market_id, outcome_index, shares).await?;
            let q: Value = rpc
                .private(
                    "market_sell",
                    json!([{"market_id": market_id, "outcome_index": outcome_index, "shares_amount": shares,
                            "seller_address": holder, "dry_run": true}]),
                )
                .await?;
            let net = q["net_proceeds_sats"].as_u64().ok_or("the node's quote has no proceeds")?;
            let gross = q["proceeds_sats"].as_u64().unwrap_or(net);
            let fee = q["trading_fee_sats"].as_u64().unwrap_or(0);
            let after = q["new_price"].as_f64().unwrap_or(price_now);
            if gross > shares || net > gross || after > price_now + 1e-9 || !fee_ok(&m, gross, fee) {
                return Err("The node's quote doesn't add up; not trading on it".into());
            }
            Ok(Quote {
                side,
                market_id: market_id.into(),
                market_title: title,
                outcome: outcome_index,
                outcome_label: label,
                shares,
                sats: net,
                trading_fee_sats: fee,
                miner_fee_sats: MINER_FEE,
                price_now,
                price_after: after,
                // At least half of what it brings (re-check N4: a small sell's floor came out at 0), and never above
                // what place() accepts.
                limit_sats: net
                    .saturating_sub(MINER_FEE + margin(net))
                    .max((net / 2).min(net.saturating_sub(MINER_FEE))),
                seller_address: Some(holder),
            })
        }
    }
}

/// Is a quoted trading fee the market's? Its rate (at most 10%) of the amount, and at least 1,000 sats.
fn fee_ok(m: &Value, amount: u64, fee: u64) -> bool {
    let rate = m["trading_fee_rate"].as_f64().unwrap_or(0.0);
    (0.0..=0.1).contains(&rate) && fee <= ((rate * amount as f64).ceil() as u64 + 1).max(1_000)
}

/// Did the node refuse a trade before sending it? In truthcoin_dc v0.19.0 and v0.20.0, `market_buy` and `market_sell`
/// fail with these messages before anything goes out; only the last step (`sign_and_send`) can fail after the trade
/// reached the mempool and peers. Anything else is treated as "may have gone through" (review L1).
pub fn refused_before_sending(code: i64, message: &str) -> bool {
    if (-32700..=-32600).contains(&code) {
        return true; // JSON-RPC parse, request, method and params errors: nothing ran
    }
    let m = message.to_lowercase();
    [
        "market not found",
        "calculation failed",
        "calculation error",
        "max_cost is required",
        "exceeds maximum cost",
        "below minimum",
        "min_proceeds",
        "insufficient shares",
        "wallet has no addresses",
        "chain has no tip",
        "not enough funds",
        "invalid market",
        "market is not",
        // submit_transaction's own validation, before the mempool (re-review R1)
        "invalid transaction",
        "utxo double spent",
        "value in is less than value out",
        "tx-pow",
    ]
    .iter()
    .any(|p| m.contains(p))
}

/// The wallet address holding the most shares of an outcome, if it holds at least `shares`.
async fn holder_of(rpc: &Rpc, market_id: &str, outcome_index: u32, shares: u64) -> Result<String, String> {
    let addrs: Vec<String> = rpc.private("get_wallet_addresses", json!([])).await?;
    let mut best: Option<(String, u64)> = None;
    for a in addrs {
        let h: Value = rpc.public("market_positions", json!([a, market_id])).await?;
        let n: u64 = h["positions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|p| p["market_id"].as_str() == Some(market_id) && p["outcome_index"].as_u64() == Some(outcome_index as u64))
            .map(|p| p["shares"].as_f64().unwrap_or(0.0).max(0.0) as u64)
            .sum();
        if n > best.as_ref().map(|b| b.1).unwrap_or(0) {
            best = Some((a, n));
        }
    }
    match best {
        None => Err("You hold no shares of this outcome".into()),
        Some((_, n)) if n < shares => Err(format!("At most {n} shares can be sold in one trade (they sit at one address)")),
        Some((a, _)) => Ok(a),
    }
}

/// "15,217": amounts in messages read as on the screens.
pub fn fmt_sats(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Why a trade wasn't placed: refused (nothing went to the node, or the node said no) or unsure (it went, and no answer
/// came back: it may be in the node's mempool).
#[derive(Debug, Clone, PartialEq)]
pub enum PlaceError {
    Refused(String),
    Unsure(String),
}

impl std::fmt::Display for PlaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlaceError::Refused(m) | PlaceError::Unsure(m) => f.write_str(m),
        }
    }
}

impl From<PlaceError> for String {
    fn from(e: PlaceError) -> String {
        e.to_string()
    }
}

/// Place a trade on a fresh quote. `limit_sats` is the caller's cap (at most, for a buy; at least, for a sell). The
/// trade is written to the log before it goes to the node.
pub async fn place(
    rpc: &Rpc,
    trades: &Trades,
    id: &str,
    source: &str,
    market_id: &str,
    outcome_index: u32,
    shares: u64,
    side: Side,
    limit_sats: u64,
) -> Result<Trade, PlaceError> {
    let q = quote(rpc, market_id, outcome_index, shares, side.clone()).await.map_err(PlaceError::Refused)?;
    match side {
        Side::Buy if limit_sats < q.sats + MINER_FEE => {
            return Err(PlaceError::Refused(format!(
                "The price moved: it's now about {} sats with the miner fee, over your cap of {}. Get a new price.",
                fmt_sats(q.sats + MINER_FEE),
                fmt_sats(limit_sats)
            )))
        }
        Side::Sell if limit_sats > q.sats.saturating_sub(MINER_FEE) => {
            return Err(PlaceError::Refused(format!(
                "The price moved: selling now brings about {} sats after the miner fee, under your minimum of {}. Get a new price.",
                fmt_sats(q.sats.saturating_sub(MINER_FEE)),
                fmt_sats(limit_sats.saturating_sub(MINER_FEE))
            )))
        }
        _ => {}
    }
    let height: u64 = rpc.public("getblockcount", json!([])).await.unwrap_or(0);
    let t = Trade {
        id: id.to_string(),
        time: crate::activity::unix_now(),
        market_id: market_id.into(),
        market_title: q.market_title.clone(),
        outcome: outcome_index,
        outcome_label: q.outcome_label.clone(),
        side: side.clone(),
        shares,
        quoted_sats: q.sats,
        limit_sats,
        txid: None,
        status: Status::Sending,
        source: source.into(),
        height,
        error: None,
        // What it counts against a phone's limit: a buy its cap, a sell its face value, a sat a share (re-review R3).
        charge_sats: match side {
            Side::Buy => limit_sats,
            Side::Sell => shares,
        },
    };
    trades.add(t.clone()).map_err(PlaceError::Refused)?;
    forget_holdings();
    let r: Result<Value, _> = match side {
        Side::Buy => {
            rpc.private(
                "market_buy",
                json!([{"market_id": market_id, "outcome_index": outcome_index, "shares_amount": shares,
                        "max_cost": limit_sats}]),
            )
            .await
        }
        Side::Sell => {
            rpc.private(
                "market_sell",
                json!([{"market_id": market_id, "outcome_index": outcome_index, "shares_amount": shares,
                        "seller_address": q.seller_address, "min_proceeds": limit_sats}]),
            )
            .await
        }
    };
    const UNSURE: &str = "Not confirmed: check Positions before trying again";
    let sent = match r {
        Ok(v) => {
            let txid = v["txid"].as_str().map(String::from);
            let mut sent = t.clone();
            sent.txid = txid.clone();
            sent.status = Status::Pending;
            // The trade went out: a failure to write that down mustn't turn it into an error (review L11).
            if let Err(e) = trades.update(id, |t| {
                t.txid = txid.clone();
                t.status = Status::Pending;
            }) {
                crate::activity::note(crate::state::app_dir(), &format!("trade {id} placed, but not recorded: {e}"));
            }
            sent
        }
        // The node refused it before sending: nothing went out. Any other error, a timeout or a lost connection is
        // different: the trade may have gone, so it stays "sending" and counts.
        Err(crate::rpc::RpcError::Rpc { code, message }) if refused_before_sending(code, &message) => {
            let _ = trades.update(id, |t| {
                t.status = Status::Failed;
                t.error = Some(message.clone());
            });
            return Err(PlaceError::Refused(message));
        }
        Err(e) => return Err(PlaceError::Unsure(format!("{UNSURE} ({e})"))),
    };
    crate::activity::note(
        crate::state::app_dir(),
        &format!("{source} {:?} {shares} shares of outcome {outcome_index} in {market_id}", side),
    );
    Ok(trades.get(id).unwrap_or(sent))
}

/// How many blocks a cancelled or dropped trade is still looked for: another node may have had it (review L13).
const RECHECK_BLOCKS: u64 = 20;

/// Bring the log's trades up to date: in the mempool, in a block, or gone.
pub async fn refresh(rpc: &Rpc, trades: &Trades) -> Result<(), String> {
    let height: u64 = rpc.public("getblockcount", json!([])).await.unwrap_or(0);
    let open: Vec<Trade> = trades
        .all()
        .into_iter()
        .filter(|t| match t.status {
            Status::Pending | Status::Sending => true,
            Status::Cancelled | Status::Dropped => t.height + RECHECK_BLOCKS >= height,
            _ => false,
        })
        .filter(|t| t.txid.is_some())
        .collect();
    if open.is_empty() {
        return Ok(());
    }
    let pool: Vec<Value> = rpc.public("list_mempool", json!([])).await?;
    let in_pool: Vec<&str> = pool.iter().filter_map(|t| t["txid"].as_str()).collect();
    for t in open {
        let Some(txid) = t.txid.clone() else { continue };
        let status = if in_pool.contains(&txid.as_str()) {
            if t.status == Status::Cancelled {
                continue; // cancelled here, but back from a peer: still cancelled until it lands or goes
            }
            Status::Pending
        } else {
            // On a node error, keep what is known.
            let Ok(info) = rpc.public::<Value>("get_transaction_info", json!([txid])).await else { continue };
            if !info["txin"].is_null() {
                Status::Done
            } else if t.status == Status::Cancelled {
                continue;
            } else {
                Status::Dropped
            }
        };
        if status != t.status {
            trades.update(&t.id, |t| t.status = status)?;
            forget_holdings();
        }
    }
    Ok(())
}

/// Take a stuck trade out of the node's mempool and give its coin back to the wallet.
pub async fn cancel(rpc: &Rpc, trades: &Trades, id: &str) -> Result<(), String> {
    let t = trades.get(id).ok_or("no such trade")?;
    let txid = t.txid.clone().ok_or("this trade has no transaction to cancel")?;
    if t.status != Status::Pending {
        return Err("only a pending trade can be cancelled".into());
    }
    let _: Value = rpc.private("remove_from_mempool", json!([txid])).await?;
    let _: Value = rpc.private_slow("refresh_wallet", json!([])).await?;
    trades.update(id, |t| t.status = Status::Cancelled)?;
    forget_holdings();
    crate::activity::note(crate::state::app_dir(), &format!("cancelled pending trade {id}"));
    Ok(())
}

/// What pending trades hold: each spends a whole coin until its block (`fee_sats` of a trade is that whole input).
pub async fn tied_up(rpc: &Rpc, trades: &Trades) -> u64 {
    let mut sum = 0;
    for t in trades.all().iter().filter(|t| t.status == Status::Pending) {
        if let Some(txid) = &t.txid {
            let info: Value = rpc.public("get_transaction_info", json!([txid])).await.unwrap_or(Value::Null);
            if info["txin"].is_null() {
                sum += info["fee_sats"].as_u64().unwrap_or(0);
            }
        }
    }
    sum
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_known_pre_send_errors_count_as_refused() {
        assert!(refused_before_sending(-1, "Share cost 60000 exceeds maximum cost 54295 (slippage protection)"));
        assert!(refused_before_sending(-1, "not enough funds"));
        assert!(refused_before_sending(-32602, "Invalid params"));
        assert!(refused_before_sending(-1, "invalid transaction: Market not in trading state (Voting)"));
        assert!(refused_before_sending(-1, "utxo double spent"));
        assert!(!refused_before_sending(-1, "database error: MDB_MAP_FULL"), "a failure after sending");
        assert!(!refused_before_sending(-1, ""));
    }

    #[test]
    fn thousands() {
        assert_eq!(fmt_sats(0), "0");
        assert_eq!(fmt_sats(999), "999");
        assert_eq!(fmt_sats(15217), "15,217");
        assert_eq!(fmt_sats(100000000), "100,000,000");
    }

    #[test]
    fn the_fee_must_be_the_markets() {
        let m = json!({"trading_fee_rate": 0.01});
        assert!(fee_ok(&m, 52_250, 1_000));
        assert!(fee_ok(&m, 1_000_000, 10_000));
        assert!(!fee_ok(&m, 1_000_000, 50_000));
        assert!(!fee_ok(&json!({"trading_fee_rate": 0.5}), 1_000, 10));
    }

    #[test]
    fn caps_leave_room_for_the_miner_fee() {
        assert_eq!(margin(10_000), 1_000);
        assert_eq!(margin(1_000_000), 20_000);
        // A buy's suggested cap covers the quote, the miner fee and the margin.
        let cost = 52_250;
        assert!(cost + MINER_FEE + margin(cost) >= cost + MINER_FEE + 1_000);
    }

    #[test]
    fn labels_are_short_in_one_question_markets() {
        let m = json!({"dimensions": [{}], "outcomes": [{"outcome_index": 1, "label": "Rain by period 7: Yes", "price": 0.6}]});
        assert_eq!(outcome(&m, 1), Some(("Yes".to_string(), 0.6)));
        let m2 = json!({"dimensions": [{}, {}], "outcomes": [{"outcome_index": 0, "label": "A: No, B: Yes", "price": 0.2}]});
        assert_eq!(outcome(&m2, 0).unwrap().0, "A: No, B: Yes");
        assert!(valid_market_id("41bbe0197f4a"));
        assert!(!valid_market_id("41bbe0197f4"));
        assert!(!valid_market_id("41bbe0197f4z"));
    }
}
