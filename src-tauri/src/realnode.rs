//! The app's own code against a real chain: dev/stack.sh with TC_NO_NODE=1 (eCash regtest and the enforcer, as
//! BitWindow gives a user), and the app's own Truthcoin node on top. Ignored by default; run with
//!
//!   TC_STACK_ENV=<the stack.env TC_NO_NODE=1 dev/stack.sh printed> cargo test -j2 realnode -- --ignored --nocapture
//!
//! It walks a market's whole life through the app's functions: start the node, set the wallet's seed, deposit from
//! the enforcer's wallet, create a market, quote, buy, see the position, sell, a stuck trade cancelled, a phone's trade
//! within and over its limit, and the wallet restored from its words on a second node.

use crate::markets;
use crate::node::{enforcer, Node, RunState};
use crate::phone::{Device, Phone};
use crate::trades::{Side, Status, Trade, Trades};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

struct Stack {
    env: PathBuf,
    enforcer: String,
}

fn stack() -> Option<Stack> {
    let env = PathBuf::from(std::env::var_os("TC_STACK_ENV")?);
    let t = std::fs::read_to_string(&env).ok()?;
    let enforcer = t.lines().find_map(|l| l.strip_prefix("EA="))?.to_string();
    Some(Stack { env, enforcer })
}

impl Stack {
    /// A new address of the enforcer's eCash wallet.
    fn l1_address(&self) -> String {
        let out = std::process::Command::new("bash")
            .arg("-c")
            .arg(format!("source '{}' && enf WalletService/CreateNewAddress", self.env.display()))
            .output()
            .unwrap();
        let v: Value = serde_json::from_slice(&out.stdout).expect("CreateNewAddress answers JSON");
        v["address"].as_str().unwrap().to_string()
    }

    fn l1(&self, n: u32) {
        let ok = std::process::Command::new("bash")
            .arg("-c")
            .arg(format!("source '{}' && l1 {n}", self.env.display()))
            .status()
            .unwrap()
            .success();
        assert!(ok, "l1 {n}");
    }
}

fn node_on(dir: &Path, st: &Stack) -> Arc<Node> {
    let node = Arc::new(Node::new(dir.to_path_buf(), reqwest::Client::builder().no_proxy().build().unwrap()));
    {
        let mut s = node.settings.lock().unwrap();
        s.network = "regtest".into();
        s.enforcer = st.enforcer.clone();
        s.rpc_port = crate::node::free_port().unwrap();
        s.zmq_port = crate::node::free_port().unwrap();
        s.p2p_addr = format!("127.0.0.1:{}", crate::node::free_port().unwrap());
        s.node_binary = Some(PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../dev/bin/truthcoin-0.19.0-x86_64-unknown-linux-gnu"
        )));
        s.node_args = vec!["--decision-config-testing".into(), "10".into()];
    }
    node
}

/// One Truthcoin block: the node's `mine` (BMM through the enforcer's wallet), then the eCash block that carries it.
async fn bmm(node: &Node, st: &Stack, n: u32) {
    for _ in 0..n {
        let rpc = node.rpc().unwrap();
        let m = tokio::spawn(async move { rpc.private::<Value>("mine", json!([null])).await });
        tokio::time::sleep(Duration::from_secs(1)).await;
        st.l1(1);
        let _ = m.await;
    }
}

async fn balance(node: &Node) -> u64 {
    let b: Value = node.rpc().unwrap().private("bitcoin_balance", json!([])).await.unwrap();
    b["total_sats"].as_u64().unwrap()
}

#[tokio::test]
#[ignore]
async fn realnode_a_market_through_the_app() {
    let Some(st) = stack() else {
        eprintln!("TC_STACK_ENV not set: skipped");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    crate::state::set_app_dir(dir.path().to_path_buf());
    let node = node_on(dir.path(), &st);
    node.start().await.expect("the app's node starts");
    assert_eq!(node.state(), RunState::Running);
    let rpc = node.rpc().unwrap();

    // The wallet: no seed, then new words.
    assert!(!crate::wallet::has_seed(&rpc).await.unwrap());
    let words: String = rpc.private("generate_mnemonic", json!([])).await.unwrap();
    let _: Value = rpc.private("set_seed_from_mnemonic", json!([words])).await.unwrap();
    assert!(crate::wallet::has_seed(&rpc).await.unwrap());
    let other: String = rpc.private("generate_mnemonic", json!([])).await.unwrap();
    let again: Result<Value, _> = rpc.private("set_seed_from_mnemonic", json!([other])).await;
    assert!(again.is_err(), "a seed can't be replaced by other words");

    // Deposit 2 coins from the enforcer's wallet, as the Deposit screen does.
    let r = crate::wallet::receive(&rpc).await.unwrap();
    assert!(r.deposit_address.starts_with("s13_"));
    let txid = enforcer::deposit(&st.enforcer, &r.address, 200_000_000, 100_000).await.expect("deposit");
    assert_eq!(txid.len(), 64);
    st.l1(1);
    bmm(&node, &st, 2).await;
    let b = balance(&node).await;
    eprintln!("balance after the deposit: {b}");
    assert!(b >= 199_000_000, "deposit arrived");
    // One coin: split it into four, so several trades can wait at once.
    assert_eq!(crate::wallet::coins(&rpc).await.unwrap(), 1);
    crate::wallet::split(&rpc, 4).await.expect("split");
    bmm(&node, &st, 1).await;
    let c = crate::wallet::coins(&rpc).await.unwrap();
    eprintln!("coins after the split: {c}, balance {}", balance(&node).await);
    assert!(c >= 4);

    // Create a market for a period ahead.
    let info: Value = rpc.public("decision_status", json!([])).await.unwrap();
    let period = info["current_period"].as_u64().unwrap() as u32 + 2;
    let m = crate::create::NewMarket {
        title: "Will the app's test pass?".into(),
        description: "A market made by the realnode test.".into(),
        kind: "binary".into(),
        question: "Test passes".into(),
        rules: "Yes if it does.".into(),
        period,
        no_label: Some("No".into()),
        yes_label: Some("Yes".into()),
        options: None,
        min: None,
        max: None,
        increment: None,
        beta: 1_000_000.0,
        trading_fee: 0.01,
        tags: vec!["test".into()],
    };
    let made = crate::create::make(&rpc, &m, 100_000).await.expect("market created");
    let mid = made["market_id"].as_str().unwrap().to_string();
    bmm(&node, &st, 1).await;
    let mk = markets::get(&rpc, &mid).await.unwrap();
    assert_eq!(mk["state"], "trading");

    // Quote and buy 100,000 Yes.
    let trades = Arc::new(Trades::load(dir.path()));
    let q = markets::quote(&rpc, &mid, 1, 100_000, Side::Buy).await.unwrap();
    eprintln!("quote: {} sats, cap {}", q.sats, q.limit_sats);
    assert!(q.sats < 100_000 && q.limit_sats >= q.sats + markets::MINER_FEE + 1_000);
    // A cap without room for the miner fee is refused before it can get stuck.
    assert!(markets::place(&rpc, &trades, "t0", "desktop", &mid, 1, 100_000, Side::Buy, q.sats).await.is_err());
    let t = markets::place(&rpc, &trades, "t1", "desktop", &mid, 1, 100_000, Side::Buy, q.limit_sats).await.unwrap();
    assert_eq!(t.status, Status::Pending);
    assert!(markets::tied_up(&rpc, &trades).await > 0, "the pending trade holds a coin");
    bmm(&node, &st, 1).await;
    markets::refresh(&rpc, &trades).await.unwrap();
    assert_eq!(trades.get("t1").unwrap().status, Status::Done);
    let h = markets::holdings(&rpc, &trades).await.unwrap();
    let yes = h.iter().find(|x| x.market_id == mid && x.outcome == 1).expect("a Yes position");
    assert_eq!(yes.shares, 100_000);
    assert_eq!(yes.outcome_label, "Yes");
    assert_eq!(yes.paid_sats, Some(t.quoted_sats + markets::MINER_FEE), "paid includes the miner fee");

    // Sell 40,000.
    let qs = markets::quote(&rpc, &mid, 1, 40_000, Side::Sell).await.unwrap();
    assert!(qs.seller_address.is_some());
    markets::place(&rpc, &trades, "t2", "desktop", &mid, 1, 40_000, Side::Sell, qs.limit_sats).await.unwrap();
    bmm(&node, &st, 1).await;
    markets::refresh(&rpc, &trades).await.unwrap();
    assert_eq!(trades.get("t2").unwrap().status, Status::Done);
    let h = markets::holdings(&rpc, &trades).await.unwrap();
    assert_eq!(h.iter().find(|x| x.market_id == mid && x.outcome == 1).unwrap().shares, 60_000);

    // A trade whose cap has no room for the miner fee (made straight through the node, as another program could) is
    // skipped block after block; Cancel takes it out and gives the coin back.
    let before = balance(&node).await;
    let q2 = markets::quote(&rpc, &mid, 0, 10_000, Side::Buy).await.unwrap();
    let v: Value = rpc
        .private("market_buy", json!([{"market_id": mid, "outcome_index": 0, "shares_amount": 10_000, "max_cost": q2.sats}]))
        .await
        .unwrap();
    trades
        .add(Trade {
            id: "t3".into(),
            time: crate::activity::unix_now(),
            market_id: mid.clone(),
            market_title: String::new(),
            outcome: 0,
            outcome_label: "No".into(),
            side: Side::Buy,
            shares: 10_000,
            quoted_sats: q2.sats,
            limit_sats: q2.sats,
            txid: v["txid"].as_str().map(String::from),
            status: Status::Pending,
            source: "desktop".into(),
            height: 0,
            error: None,
            charge_sats: q2.sats,
        })
        .unwrap();
    bmm(&node, &st, 2).await;
    markets::refresh(&rpc, &trades).await.unwrap();
    assert_eq!(trades.get("t3").unwrap().status, Status::Pending, "skipped: still pending");
    markets::cancel(&rpc, &trades, "t3").await.unwrap();
    assert_eq!(trades.get("t3").unwrap().status, Status::Cancelled);
    assert_eq!(balance(&node).await, before, "the coin is back");

    // A phone with a 20,000-sat limit: a small buy goes through, a big one is held, then allowed on the desktop.
    let phone = Arc::new(Phone::new(dir.path(), node.clone(), trades.clone()).unwrap());
    let dev = Device {
        name: "Test phone".into(),
        p: crate::phone::crypto::pub_b64u(&crate::phone::crypto::random_secret().public_key()),
        np: "ab".repeat(32),
        limit_sats: 20_000,
        paired_at: 0,
        last_seen: 0,
    };
    phone.add_device_for_test(dev.clone());
    let pq = phone.read(&dev, "quote", &json!({"id": mid, "outcome": 1, "shares": 5_000, "side": "buy"})).await.unwrap();
    let small = phone
        .trade(&dev, "aa".repeat(16).as_str(), &json!({"id": mid, "outcome": 1, "shares": 5_000, "side": "buy", "limit": pq["limit"]}))
        .await;
    assert_eq!(small["ok"]["status"], "pending", "{small}");
    // A cap far above the price is refused; one at the suggested cap, over the limit, is held.
    let bq = phone.read(&dev, "quote", &json!({"id": mid, "outcome": 1, "shares": 50_000, "side": "buy"})).await.unwrap();
    let far = phone
        .trade(&dev, "ba".repeat(16).as_str(), &json!({"id": mid, "outcome": 1, "shares": 50_000, "side": "buy",
                                                       "limit": bq["limit"].as_u64().unwrap() * 3}))
        .await;
    assert!(far["err"].is_string(), "{far}");
    let big = phone
        .trade(&dev, "bb".repeat(16).as_str(), &json!({"id": mid, "outcome": 1, "shares": 50_000, "side": "buy", "limit": bq["limit"]}))
        .await;
    assert!(big["held"].is_object(), "{big}");
    assert_eq!(phone.held().len(), 1);
    let allowed = phone.held_answer(&"bb".repeat(16), true).await.unwrap();
    assert_eq!(allowed["ok"]["status"], "pending", "{allowed}");
    bmm(&node, &st, 1).await;
    let pos = phone.read(&dev, "positions", &json!({})).await.unwrap();
    eprintln!("phone positions: {pos}");
    let shares: u64 = pos["positions"].as_array().unwrap().iter().filter(|p| p["outcome"] == 1).map(|p| p["shares"].as_u64().unwrap()).sum();
    assert_eq!(shares, 60_000 + 5_000 + 50_000);
    let bal = phone.read(&dev, "balance", &json!({})).await.unwrap();
    assert!(bal["total"].as_u64().unwrap() > 0);
    // The phone's limit is spent: a sell counts at its value, so it is held too.
    let sq = phone.read(&dev, "quote", &json!({"id": mid, "outcome": 1, "shares": 10_000, "side": "sell"})).await.unwrap();
    let sell = phone
        .trade(&dev, "cc".repeat(16).as_str(), &json!({"id": mid, "outcome": 1, "shares": 10_000, "side": "sell", "limit": sq["limit"]}))
        .await;
    eprintln!("phone sell: {sell}");
    assert!(sell["held"].is_object(), "{sell}");
    let refused = phone.held_answer(&"cc".repeat(16), false).await.unwrap();
    assert!(refused["err"].is_string());
    assert!(phone.held_answer(&"cc".repeat(16), true).await.is_err(), "a held trade is answered once");

    // Withdraw 0.1 coin to an eCash address (as the Withdraw panel does): it joins the pending withdrawal bundle.
    let to = st.l1_address();
    let w: Value = rpc.private("withdraw", json!([to, 10_000_000u64, 1_000u64, 1_000u64])).await.expect("withdraw");
    eprintln!("withdraw: {w}");
    bmm(&node, &st, 1).await;
    let bundle: Value = rpc.public("pending_withdrawal_bundle", json!([])).await.unwrap_or(Value::Null);
    eprintln!("pending bundle: {}", bundle.to_string().chars().take(200).collect::<String>());

    // Restore: a second node from the same words finds the same coins.
    let total = balance(&node).await;
    node.stop().await;
    assert_eq!(node.state(), RunState::Stopped);
    let dir2 = tempfile::tempdir().unwrap();
    let node2 = node_on(dir2.path(), &st);
    node2.start().await.expect("a second node starts");
    let rpc2 = node2.rpc().unwrap();
    // It catches up through the enforcer and its own block archive? A fresh node has only the eCash side: it needs
    // the Truthcoin blocks from a peer, which this stack doesn't have once the first node stopped. So restart the
    // first node beside it and connect them.
    node.start().await.unwrap();
    let p2p = node.settings().p2p_addr;
    let _: Result<Value, _> = rpc2.private("connect_peer", json!([p2p])).await;
    for _ in 0..60 {
        let h1: u64 = node.rpc().unwrap().public("getblockcount", json!([])).await.unwrap();
        let h2: u64 = rpc2.public("getblockcount", json!([])).await.unwrap_or(0);
        if h2 >= h1 {
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    let looked = crate::wallet::restore(&rpc2, &words).await.expect("restore");
    let restored = balance(&node2).await;
    eprintln!("restore looked at {looked} addresses");
    eprintln!("restored balance {restored} (original {total})");
    assert_eq!(restored, total, "the words bring the coins back");
    node2.stop().await;
    node.stop().await;
}
