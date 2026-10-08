//! The app's own code against a real chain: dev/stack.sh with TC_NO_NODE=1 (eCash regtest and the enforcer, as
//! BitWindow gives a user), and the app's own Truthcoin node on top. Ignored by default; run with
//!
//!   TC_STACK_ENV=<the stack.env TC_NO_NODE=1 dev/stack.sh printed> cargo test -j2 realnode -- --ignored --nocapture
//!
//! with the node the app pins, from dev/bin (`truthcoin-<version>-x86_64-unknown-linux-gnu`, L2L's release), or another
//! one named by TC_NODE_BIN.
//!
//! It walks a market's whole life through the app's functions: start the node, set the wallet's seed, deposit from
//! the enforcer's wallet, create a market, quote, buy, see the position, sell, a waiting trade cancelled, a phone's trade
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
        s.p2p_addr = format!("127.0.0.1:{}", crate::node::free_port().unwrap());
        s.node_binary = Some(std::env::var_os("TC_NODE_BIN").map(PathBuf::from).unwrap_or_else(|| {
            dev_bin(&format!("truthcoin-{}-x86_64-unknown-linux-gnu", crate::node::pins::NODE_VERSION))
        }));
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

fn dev_bin(name: &str) -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../dev/bin")).join(name)
}

async fn balance(node: &Node) -> u64 {
    let b: Value = node.rpc().unwrap().private("balance", json!([])).await.unwrap();
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

    // Deposit 2 coins from the enforcer's wallet, as BitWindow does (the app itself never spends from it).
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
    // A BMM request can miss the eCash block a second later: mine again, up to 3 times, before calling it missing.
    for _ in 0..3 {
        bmm(&node, &st, 1).await;
        if crate::wallet::coins(&rpc).await.unwrap() >= 4 {
            break;
        }
    }
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

    // Cancel: it takes a waiting trade out and gives the coin back. The node refuses a cap with no room for the miner
    // fee, and prices a quote after the trades already waiting, so one node can't strand its own trade (a miner that
    // saw trades in another order still can, and Cancel is for that): here a trade is cancelled before its block.
    let before = balance(&node).await;
    let q2 = markets::quote(&rpc, &mid, 0, 10_000, Side::Buy).await.unwrap();
    let refused: Result<Value, _> = rpc
        .private("market_buy", json!([{"market_id": mid, "outcome_index": 0, "shares_amount": 10_000, "max_cost": q2.sats}]))
        .await;
    assert!(refused.as_ref().is_err_and(|e| e.to_string().contains("miner fee")), "{refused:?}");
    let cap = q2.sats + markets::MINER_FEE;
    let v: Value = rpc
        .private("market_buy", json!([{"market_id": mid, "outcome_index": 0, "shares_amount": 10_000, "max_cost": cap}]))
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
    markets::refresh(&rpc, &trades).await.unwrap();
    assert_eq!(trades.get("t3").unwrap().status, Status::Pending, "waiting");
    let (pending, tied) = (balance(&node).await, markets::tied_up(&rpc, &trades).await);
    eprintln!("waiting trade: balance {pending}, tied up {tied}");
    assert!(tied > 0, "the waiting trade holds a coin");
    markets::cancel(&rpc, &trades, "t3").await.unwrap();
    assert_eq!(trades.get("t3").unwrap().status, Status::Cancelled);
    assert_eq!(markets::tied_up(&rpc, &trades).await, 0);
    assert_eq!(balance(&node).await, before, "the coin is back");
    // And it stays out: the next block doesn't carry it.
    bmm(&node, &st, 1).await;
    markets::refresh(&rpc, &trades).await.unwrap();
    assert_eq!(trades.get("t3").unwrap().status, Status::Cancelled);

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
    for _ in 0..3 {
        bmm(&node, &st, 1).await;
        if rpc.public::<Vec<Value>>("list_mempool", json!([])).await.is_ok_and(|m| m.is_empty()) {
            break;
        }
    }
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
    let w: Value = rpc.private("create_withdrawal", json!([to, 10_000_000u64, 1_000u64, 1_000u64])).await.expect("withdraw");
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

/// One folder from Truthcoin 0.19 to 0.20: 0.19 (L2L's v0.19.0, from dev/bin, run here only to leave what it leaves)
/// makes a wallet and gets a coin; 0.20 can't read that, so the app sets it aside before its first start, and the same
/// words then make a wallet with other addresses.
#[tokio::test]
#[ignore]
async fn realnode_from_019_to_020() {
    let Some(st) = stack() else {
        eprintln!("TC_STACK_ENV not set: skipped");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    crate::state::set_app_dir(dir.path().to_path_buf());
    let node = node_on(dir.path(), &st);
    let (ep, port) = (st.enforcer.rsplit_once(':').unwrap().1.to_string(), crate::node::free_port().unwrap());
    let mut old = std::process::Command::new(dev_bin("truthcoin-0.19.0-x86_64-unknown-linux-gnu"))
        .args(["--headless", "--network", "regtest", "--datadir"])
        .arg(dir.path().join("node"))
        .args(["--mainchain-grpc-host", "127.0.0.1", "--mainchain-grpc-port", &ep])
        .args(["--rpc-host", "127.0.0.1", "--rpc-port", &port.to_string(), "--net-addr", "127.0.0.1:0"])
        .args(["--zmq-addr", "127.0.0.1:0", "--decision-config-testing", "10", "--log-level", "info"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("0.19 runs");
    let rpc = crate::rpc::Rpc::new(reqwest::Client::builder().no_proxy().build().unwrap(), port, "127.0.0.1", port);
    for _ in 0..120 {
        if rpc.public::<u64>("getblockcount", json!([])).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let words: String = rpc.private("generate_mnemonic", json!([])).await.unwrap();
    let _: Value = rpc.private("set_seed_from_mnemonic", json!([words])).await.unwrap();
    crate::wallet::mark_ready(dir.path());
    let a19: String = rpc.private("get_new_address", json!([])).await.unwrap();
    enforcer::deposit(&st.enforcer, &a19, 100_000_000, 100_000).await.expect("deposit");
    st.l1(1);
    for _ in 0..4 {
        let r = rpc.clone();
        let m = tokio::spawn(async move { r.private::<Value>("mine", json!([null])).await });
        tokio::time::sleep(Duration::from_secs(1)).await;
        st.l1(1);
        let _ = m.await;
        let b: Value = rpc.private("bitcoin_balance", json!([])).await.unwrap();
        if b["total_sats"].as_u64().unwrap_or(0) > 0 {
            break;
        }
    }
    let b: Value = rpc.private("bitcoin_balance", json!([])).await.unwrap();
    eprintln!("0.19: {} sats at {a19}", b["total_sats"]);
    assert!(b["total_sats"].as_u64().unwrap() >= 99_000_000);
    std::fs::write(dir.path().join("trades.json"), b"[]").unwrap();
    let _: Result<Value, _> = rpc.private("stop", json!([])).await;
    let _ = old.wait();

    // The app's node, 0.20, on the same folder.
    let told = Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let t = told.clone();
        let _ = node.on_set_aside.set(Box::new(move || t.store(true, std::sync::atomic::Ordering::SeqCst)));
    }
    node.start().await.expect("0.20 starts once 0.19's data is set aside");
    let aside = node.set_aside_path().expect("0.19's data set aside");
    eprintln!("set aside: {:?}", std::fs::read_dir(&aside).unwrap().map(|e| e.unwrap().file_name()).collect::<Vec<_>>());
    assert!(told.load(std::sync::atomic::Ordering::SeqCst), "the app was told");
    assert!(aside.join("wallet.mdb").exists() && aside.join("wallet-ready").is_file() && aside.join("trades.json").is_file());
    assert!(!dir.path().join("wallet-ready").exists());
    let rpc = node.rpc().unwrap();
    assert!(!crate::wallet::has_seed(&rpc).await.unwrap(), "0.20 starts without a wallet");

    // The same words: a wallet again, at other addresses.
    let looked = crate::wallet::restore(&rpc, &words).await.expect("restore on 0.20");
    let addrs: Vec<String> = rpc.private("get_wallet_addresses", json!([])).await.unwrap();
    eprintln!("0.20 restore looked at {looked} addresses; 0.19's {a19} among them: {}", addrs.contains(&a19));
    assert!(!addrs.contains(&a19), "0.20 derives other addresses from the same words");
    bmm(&node, &st, 1).await;
    eprintln!("0.20 balance from the same words: {}", balance(&node).await);

    // A deposit to a 0.20 address arrives, and the app reads it.
    let r2 = crate::wallet::receive(&rpc).await.unwrap();
    enforcer::deposit(&st.enforcer, &r2.address, 50_000_000, 100_000).await.expect("deposit to 0.20");
    st.l1(1);
    for _ in 0..3 {
        bmm(&node, &st, 1).await;
        if balance(&node).await > 0 {
            break;
        }
    }
    let after = balance(&node).await;
    let coins = crate::wallet::coins(&rpc).await.unwrap();
    eprintln!("0.20: {after} sats, {coins} coin(s), at {}", r2.address);
    assert!(after >= 49_000_000 && coins >= 1);
    node.stop().await;
    // Starting again leaves everything where it is.
    node.start().await.expect("0.20 starts again");
    assert!(crate::wallet::has_seed(&node.rpc().unwrap()).await.unwrap());
    node.stop().await;

    // Obliterate removes Truthcoin: the node's data with 0.19's set aside inside it, and the wallet's records.
    use crate::obliterate::{execute, plan_items, Part, Places, Shown};
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap();
    let p = Places {
        home,
        app_dir: dir.path().to_path_buf(),
        app_dir_from_env: true,
        screen_uses_app_dir: false,
        caches: vec![],
        program: None,
    };
    let plan = plan_items(&p).unwrap();
    let data = plan.iter().find(|i| i.id == "node-data").expect("node data listed");
    assert!(data.note.contains("0.19"), "{}", data.note);
    let shown: Vec<Shown> = plan.iter().map(|i| Shown { id: i.id.clone(), path: i.path.clone() }).collect();
    let done = execute(&p, &[Part::Truthcoin], &shown).unwrap();
    assert!(done.errors.is_empty(), "{:?}", done.errors);
    assert!(!dir.path().join("node").exists(), "node/, and set-aside-0.19 in it, gone");
}
