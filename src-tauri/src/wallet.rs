//! The Truthcoin wallet, which lives in the node this app runs: its recovery words, balance, receiving (including
//! the address BitWindow takes for a deposit from eCash) and withdrawals to eCash. The app never spends from the eCash
//! wallet: deposits are made in BitWindow.
//!
//! The node keeps its seed unencrypted in its own data folder, and has no call that gives the words back. So the
//! words are shown once, at setup, and the app keeps no copy: three of them are asked back before the wallet is made.
//! A seed can't be replaced once set (the node refuses), so nothing here can overwrite a wallet.

use crate::markets;
use crate::node::enforcer;
use crate::state::{NewWords, St};
use rand::seq::SliceRandom;
use serde::Serialize;
use serde_json::{json, Value};
use zeroize::Zeroizing;

const NO_SEED: &str = "does not have a seed";

/// Does the node's wallet have its seed yet?
pub async fn has_seed(rpc: &crate::rpc::Rpc) -> Result<bool, String> {
    match rpc.private::<Value>("get_voter_address", json!([])).await {
        Ok(_) => Ok(true),
        Err(crate::rpc::RpcError::Rpc { message, .. }) if message.contains(NO_SEED) => Ok(false),
        Err(e) => Err(e.to_string()),
    }
}

#[derive(Serialize)]
pub struct WalletStatus {
    pub has_seed: bool,
    /// What the wallet holds and can use, once what is moving settles: its coins, change coming back from its own
    /// transfers, and the coins waiting trades hold less what those trades cost. Withdrawals on their way to eCash are
    /// not in it (UX review B1).
    pub total_sats: u64,
    /// Ready to use now.
    pub available_sats: u64,
    /// Coins big enough to pay for a trade. Each waiting trade holds one, so this is how many can wait at once.
    pub coins: usize,
    /// Coins spent by trades waiting for their block (they come back, as change and shares, in that block).
    pub in_pending_trades_sats: u64,
    pub pending_trades: usize,
    /// Change from this wallet's own transfers, not yet in a block.
    pub incoming_sats: u64,
    /// On its way to eCash: withdrawals waiting for their bundle to pay out.
    pub withdrawing_sats: u64,
    /// Withdrawals to eCash this app made, until they pay out (or are hidden).
    pub withdrawals: Vec<WithdrawalView>,
    /// The cost of the waiting trades, already taken out of `total_sats`.
    pub pending_cost_sats: u64,
}

#[derive(Serialize, serde::Deserialize, Clone)]
pub struct Withdrawal {
    pub time: u64,
    pub amount_sats: u64,
    pub address: String,
    pub txid: String,
    #[serde(default)]
    pub hidden: bool,
}

#[derive(Serialize, Clone)]
pub struct WithdrawalView {
    pub time: u64,
    pub amount_sats: u64,
    pub address: String,
    pub txid: String,
    /// "waiting" (for the next bundle), "bundled" (in the bundle eCash miners are voting on) or "sent" (out of this
    /// node's hands: it pays out once miners approve it; BitWindow shows the payment).
    pub stage: String,
}

/// The value of an output's content: plain coins, and a withdrawal's value, from the node's JSON.
fn content_value(c: &Value) -> (u64, u64) {
    (c["Value"].as_u64().unwrap_or(0), c["Withdrawal"]["value_sats"].as_u64().unwrap_or(0))
}

/// Coins and withdrawals among the wallet's outputs.
async fn holdings_in_coins(rpc: &crate::rpc::Rpc) -> Result<(u64, u64, u64), String> {
    let utxos: Vec<Value> = rpc.private("get_wallet_utxos", json!([])).await?;
    let (mut coins, mut withdrawing) = (0u64, 0u64);
    for u in &utxos {
        let (c, w) = content_value(&u["output"]["content"]);
        coins = coins.saturating_add(c);
        withdrawing = withdrawing.saturating_add(w);
    }
    let unconfirmed: Vec<Value> = rpc.private("my_unconfirmed_utxos", json!([])).await.unwrap_or_default();
    let incoming = unconfirmed
        .iter()
        .map(|u| content_value(if u["output"].is_null() { u } else { &u["output"]["content"] }).0)
        .fold(0u64, |a, b| a.saturating_add(b));
    Ok((coins, incoming, withdrawing))
}

fn withdrawals_path(dir: &std::path::Path) -> std::path::PathBuf {
    dir.join("withdrawals.json")
}

fn all_withdrawals(dir: &std::path::Path) -> Vec<Withdrawal> {
    crate::files::read_json(&withdrawals_path(dir)).ok().flatten().unwrap_or_default()
}

/// The app's withdrawals, shown for 14 days unless hidden (UX re-check N2), each with how far it has got: still an
/// output of this wallet (waiting for the next bundle), in the bundle eCash miners are voting on, or past that.
async fn withdrawals_view(dir: &std::path::Path, rpc: &crate::rpc::Rpc) -> Vec<WithdrawalView> {
    let now = crate::activity::unix_now();
    let l: Vec<Withdrawal> =
        all_withdrawals(dir).into_iter().filter(|w| !w.hidden && w.time + 14 * 86400 > now).collect();
    if l.is_empty() {
        return vec![];
    }
    let utxos: Value = rpc.private("get_wallet_utxos", json!([])).await.unwrap_or(Value::Null);
    let bundle: Value = rpc.public("pending_withdrawal_bundle", json!([])).await.unwrap_or(Value::Null);
    let (mine, bundled) = (utxos.to_string(), bundle.to_string());
    l.into_iter()
        .map(|w| {
            let stage = if mine.contains(&w.address) {
                "waiting"
            } else if bundled.contains(&w.address) {
                "bundled"
            } else {
                "sent"
            };
            WithdrawalView { time: w.time, amount_sats: w.amount_sats, address: w.address, txid: w.txid, stage: stage.into() }
        })
        .collect()
}

/// Hide a withdrawal from Home (it paid out, or the user has seen enough of it).
#[tauri::command]
pub fn withdrawal_hide(st: St<'_>, txid: String) -> Result<(), String> {
    let mut l = all_withdrawals(&st.dir);
    for w in l.iter_mut().filter(|w| w.txid == txid) {
        w.hidden = true;
    }
    crate::files::write_json(&withdrawals_path(&st.dir), &l).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn wallet_status(st: St<'_>) -> Result<WalletStatus, String> {
    let rpc = st.node.rpc_or_err()?;
    let has_seed = has_seed(&rpc).await?;
    if has_seed {
        mark_ready(&st.dir);
    }
    let b = balance(&rpc, &st.trades).await?;
    let coins = if has_seed { coins(&rpc).await.unwrap_or(0) } else { 0 };
    let withdrawals = withdrawals_view(&st.dir, &rpc).await;
    Ok(WalletStatus { has_seed, coins, withdrawals, ..b })
}

/// The balance as people read it (UX review B1), for the desktop and the phone.
pub async fn balance(rpc: &crate::rpc::Rpc, trades: &crate::trades::Trades) -> Result<WalletStatus, String> {
    let b: Value = rpc.private("balance", json!([])).await?;
    let _ = markets::refresh(rpc, trades).await;
    let tied = markets::tied_up(rpc, trades).await;
    let open: Vec<crate::trades::Trade> =
        trades.all().into_iter().filter(|t| t.status == crate::trades::Status::Pending).collect();
    // What the waiting trades will cost (a buy, its quote and miner fee); a sell's proceeds come later, as coins.
    let cost: u64 = open
        .iter()
        .filter(|t| t.side == crate::trades::Side::Buy)
        .map(|t| t.quoted_sats.saturating_add(markets::MINER_FEE))
        .fold(0, |a, b| a.saturating_add(b));
    let (coins, incoming, withdrawing) = holdings_in_coins(rpc).await?;
    Ok(WalletStatus {
        has_seed: true,
        coins: 0,
        total_sats: coins.saturating_add(incoming).saturating_add(tied.saturating_sub(cost)),
        available_sats: b["available_sats"].as_u64().unwrap_or(0),
        in_pending_trades_sats: tied,
        pending_trades: open.len(),
        incoming_sats: incoming,
        withdrawing_sats: withdrawing,
        withdrawals: vec![],
        pending_cost_sats: cost,
    })
}

/// The smallest coin counted as one a trade can use.
pub const USEFUL_COIN: u64 = 10_000;

/// How many coins of at least USEFUL_COIN the wallet has.
pub async fn coins(rpc: &crate::rpc::Rpc) -> Result<usize, String> {
    let u: Vec<Value> = rpc.private("get_wallet_utxos", json!([])).await?;
    Ok(u.iter().filter(|c| content_value(&c["output"]["content"]).0 >= USEFUL_COIN).count())
}

/// Split what the wallet has into `parts` equal coins at new addresses of its own, so that many trades can wait for a
/// block at once (a waiting trade holds a whole coin). One transfer, with a 1,000-sat fee.
pub async fn split(rpc: &crate::rpc::Rpc, parts: u32) -> Result<Value, String> {
    if !(2..=8).contains(&parts) {
        return Err("Split into 2 to 8 coins".into());
    }
    let b: Value = rpc.private("balance", json!([])).await?;
    let available = b["available_sats"].as_u64().unwrap_or(0);
    let fee = 1_000;
    let each = available.saturating_sub(fee * 3) / parts as u64;
    if each < USEFUL_COIN {
        return Err("There isn't enough to split".into());
    }
    let mut dests = serde_json::Map::new();
    for _ in 0..parts {
        let a: String = rpc.private("get_new_address", json!([])).await?;
        dests.insert(a, json!(each));
    }
    Ok(rpc.private("create_transfer_many", json!([dests, fee])).await?)
}

#[tauri::command]
pub async fn wallet_split(st: St<'_>, parts: u32) -> Result<Value, String> {
    let r = split(&st.node.rpc_or_err()?, parts).await?;
    crate::activity::note(&st.dir, &format!("split the wallet into {parts} coins"));
    Ok(r)
}

/// The app's folder remembers that its wallet was set up (so Setup isn't shown again while the node is down). Kept in
/// the data folder, not the window's storage: a new folder starts at Setup.
const READY: &str = "wallet-ready";

pub fn mark_ready(dir: &std::path::Path) {
    let p = dir.join(READY);
    if !p.exists() {
        let _ = crate::files::write_private(&p, b"1\n");
    }
}

pub fn is_ready(dir: &std::path::Path) -> bool {
    dir.join(READY).exists()
}

#[derive(Serialize)]
pub struct WordsToWrite {
    pub words: Vec<String>,
    /// Which words (1-based) the user will be asked for.
    pub ask: Vec<usize>,
}

/// New recovery words from the node, held in memory (never on disk) until three come back.
#[tauri::command]
pub async fn wallet_new_words(st: St<'_>) -> Result<WordsToWrite, String> {
    let rpc = st.node.rpc_or_err()?;
    if has_seed(&rpc).await? {
        return Err("This wallet already has its words".into());
    }
    let w: Zeroizing<String> = Zeroizing::new(rpc.private("generate_mnemonic", json!([])).await?);
    let words: Vec<String> = w.split_whitespace().map(String::from).collect();
    if words.len() < 12 {
        return Err("the node gave too few words".into());
    }
    let mut idx: Vec<usize> = (1..=words.len()).collect();
    idx.shuffle(&mut rand::thread_rng());
    let mut ask: Vec<usize> = idx.into_iter().take(3).collect();
    ask.sort();
    *st.new_words.lock().unwrap() = Some(NewWords { words: w, ask: ask.clone(), made: std::time::Instant::now() });
    Ok(WordsToWrite { words, ask })
}

/// The three asked words; if they match, the wallet is made from the words and the words are forgotten.
#[tauri::command]
pub async fn wallet_confirm_words(st: St<'_>, answers: Vec<String>) -> Result<(), String> {
    let rpc = st.node.rpc_or_err()?;
    let words = {
        let g = st.new_words.lock().unwrap();
        let nw = g.as_ref().ok_or("Start again: no words are waiting")?;
        if nw.made.elapsed().as_secs() > 3600 {
            drop(g);
            *st.new_words.lock().unwrap() = None; // forgotten (review N4)
            return Err("These words are over an hour old: start again".into());
        }
        let list: Vec<&str> = nw.words.split_whitespace().collect();
        if answers.len() != nw.ask.len() {
            return Err("Give all three words".into());
        }
        for (i, a) in nw.ask.iter().zip(&answers) {
            if list[i - 1] != a.trim().to_lowercase() {
                return Err(format!("Word {i} isn't right. Check what you wrote down."));
            }
        }
        nw.words.clone()
    };
    let _: Value = rpc.private("set_seed_from_mnemonic", json!([words.as_str()])).await?;
    *st.new_words.lock().unwrap() = None;
    crate::activity::note(&st.dir, "wallet made from new words");
    Ok(())
}

/// Bring a wallet back from its words.
#[tauri::command]
pub async fn wallet_restore(st: St<'_>, words: String) -> Result<(), String> {
    let rpc = st.node.rpc_or_err()?;
    let words = Zeroizing::new(words);
    let n = restore(&rpc, &words).await?;
    crate::activity::note(&st.dir, &format!("wallet restored from words ({n} addresses looked at)"));
    Ok(())
}

/// Addresses looked at in a row with no history before a restore stops looking.
pub const GAP: usize = 20;

/// Set the seed from `words`, then find the wallet's coins. The node's wallet only watches addresses it has handed
/// out, and a restored wallet has handed out none: so addresses are handed out 20 at a time and the chain asked
/// whether each was ever used, until 20 in a row weren't; then the wallet rescans. Returns how many were looked at.
pub async fn restore(rpc: &crate::rpc::Rpc, words: &str) -> Result<usize, String> {
    if has_seed(rpc).await? {
        return Err("This wallet already has its words; it can't be replaced".into());
    }
    let words = Zeroizing::new(words.split_whitespace().map(|w| w.to_lowercase()).collect::<Vec<_>>().join(" "));
    let n = words.split(' ').count();
    if ![12, 15, 18, 21, 24].contains(&n) {
        return Err(format!("Recovery words come in 12 or 24; this is {n}"));
    }
    let _: Value = rpc.private("set_seed_from_mnemonic", json!([words.as_str()])).await?;
    let _: Value = rpc.private("get_voter_address", json!([])).await?;
    let mut looked = 0;
    let mut unused_run = 0;
    while unused_run < GAP && looked < 5_000 {
        let mut batch = vec![];
        for _ in 0..GAP {
            batch.push(rpc.private::<String>("get_new_address", json!([])).await?);
        }
        looked += batch.len();
        let utxos: Value = rpc.public("get_utxos", json!([batch])).await?;
        let stxos: Value = rpc.public("get_stxos", json!([batch])).await?;
        let mut used = std::collections::HashSet::new();
        collect_addresses(&utxos, &mut used);
        collect_addresses(&stxos, &mut used);
        for a in &batch {
            if used.contains(a.as_str()) {
                unused_run = 0;
            } else {
                unused_run += 1;
            }
        }
    }
    let _: Value = rpc.private_slow("refresh_wallet", json!([])).await?;
    Ok(looked)
}

/// Every string under an "address" key, anywhere in `v`.
fn collect_addresses<'a>(v: &'a Value, out: &mut std::collections::HashSet<&'a str>) {
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                if k == "address" {
                    if let Some(s) = x.as_str() {
                        out.insert(s);
                    }
                }
                collect_addresses(x, out);
            }
        }
        Value::Array(a) => a.iter().for_each(|x| collect_addresses(x, out)),
        _ => {}
    }
}

#[derive(Serialize)]
pub struct Receive {
    pub address: String,
    /// The form eCash wallets (BitWindow) take for a deposit to this address.
    pub deposit_address: String,
}

#[tauri::command]
pub async fn wallet_receive(st: St<'_>) -> Result<Receive, String> {
    let rpc = st.node.rpc_or_err()?;
    receive(&rpc).await
}

pub async fn receive(rpc: &crate::rpc::Rpc) -> Result<Receive, String> {
    let address: String = rpc.private("get_new_address", json!([])).await?;
    let deposit_address = deposit_form(&address);
    Ok(Receive { address, deposit_address })
}

/// The sidechain deposit address eCash wallets take (BitWindow's form): `s13_<address>_` and the first 6 hex
/// characters of that string's SHA-256. (The node's `format_deposit_address` gives the address back unchanged.)
pub fn deposit_form(address: &str) -> String {
    use sha2::{Digest, Sha256};
    let s = format!("s{}_{address}_", enforcer::TRUTHCOIN_SLOT);
    let h = hex::encode(Sha256::digest(s.as_bytes()));
    format!("{s}{}", &h[..6])
}

#[cfg(test)]
mod tests {
    #[test]
    fn deposit_form() {
        let d = super::deposit_form("o51yVMf5cJZr7A5nWWBBzcLLDJt");
        assert!(d.starts_with("s13_o51yVMf5cJZr7A5nWWBBzcLLDJt_") && d.len() == "s13_o51yVMf5cJZr7A5nWWBBzcLLDJt_".len() + 6);
    }

    #[test]
    fn coins_and_withdrawals_read_from_the_node() {
        use serde_json::json;
        let v = super::content_value;
        assert_eq!(v(&json!({"Value": 5000})), (5000, 0));
        let w = json!({"Withdrawal": {"value_sats": 7000, "main_fee_sats": 1000, "main_address": "tb1q"}});
        assert_eq!(v(&w), (0, 7000));
        assert_eq!(v(&json!({"MarketFunds": {"market_id": [0, 0, 0, 0, 0, 1], "amount": 9000, "is_fee": false}})), (0, 0));
    }
}

/// A new address of BitWindow's eCash wallet (the enforcer's), to withdraw to.
#[tauri::command]
pub async fn ecash_address(st: St<'_>) -> Result<String, String> {
    let e = st.node.settings().enforcer;
    // Over a network, the enforcer's unencrypted answer could be someone else's address (review U1).
    if !crate::commands::is_loopback(&e) {
        return Err("Only with an enforcer on this computer: copy an address from BitWindow instead".into());
    }
    enforcer::new_address(&e).await
}

/// Withdraw to an eCash address. It joins the next withdrawal bundle, which eCash miners vote on over many blocks.
#[tauri::command]
pub async fn withdraw(
    st: St<'_>,
    address: String,
    amount_sats: u64,
    fee_sats: u64,
    mainchain_fee_sats: u64,
) -> Result<Value, String> {
    let rpc = st.node.rpc_or_err()?;
    let address = address.trim().to_string();
    if address.is_empty() || address.len() > 100 || !address.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err("That isn't an eCash address".into());
    }
    if amount_sats == 0 {
        return Err("Pick an amount".into());
    }
    // Fees no bigger than the amount, nor than 1,000,000 sats (review N3).
    if fee_sats > amount_sats.min(1_000_000) || mainchain_fee_sats > amount_sats.min(1_000_000) {
        return Err("Each fee must be under the amount, and at most 1,000,000 sats".into());
    }
    let r: Value = rpc.private("create_withdrawal", json!([address, amount_sats, fee_sats, mainchain_fee_sats])).await?;
    crate::activity::note(&st.dir, &format!("withdrawal of {amount_sats} sats to eCash {}", crate::activity::mask(&address)));
    let mut l = all_withdrawals(&st.dir);
    l.push(Withdrawal {
        time: crate::activity::unix_now(),
        amount_sats,
        address: address.clone(),
        txid: r.as_str().unwrap_or_default().to_string(),
        hidden: false,
    });
    let _ = crate::files::write_json(&withdrawals_path(&st.dir), &l);
    Ok(r)
}
