//! The commands the screens call for the node and the markets. Wallet commands are in wallet.rs, market creation in
//! create.rs, the phone link's in phone/commands.rs.

use crate::markets::{self, Holding, MarketSummary, Quote};
use crate::node::{enforcer, install, pins, RunState};
use crate::state::St;
use crate::trades::{Side, Trade};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Serialize)]
pub struct AppInfo {
    pub version: &'static str,
    /// A release build: eCash beta only.
    pub beta_only: bool,
    /// This data folder's wallet has been set up.
    pub wallet_ready: bool,
    pub node_version: &'static str,
    pub supported: bool,
    pub dir: String,
}

#[tauri::command]
pub fn app_info(st: St<'_>) -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        beta_only: crate::settings::BETA_ONLY,
        wallet_ready: crate::wallet::is_ready(&st.dir),
        node_version: pins::NODE_VERSION,
        supported: pins::this_computer().is_some(),
        dir: st.dir.display().to_string(),
    }
}

#[derive(Serialize)]
pub struct EnforcerStatus {
    pub address: String,
    /// Not on this computer: its gRPC has no encryption, so someone on the way could change a deposit (review N10).
    pub remote: bool,
    pub reachable: bool,
    pub height: u32,
    pub network: String,
    pub error: Option<String>,
}

#[derive(Serialize)]
pub struct NodeStatus {
    pub installed: bool,
    pub own_program: bool,
    pub run: RunState,
    pub network: String,
    pub height: Option<u64>,
    /// `mainchain_sync_progress`: {phase, done, total, tip_height}.
    pub sync: Option<Value>,
    pub peers: Option<usize>,
    pub rpc_port: u16,
    pub uptime_secs: Option<u64>,
    pub enforcer: EnforcerStatus,
    /// Where the node's wallet calls listen (a random 127.x.y.z on Linux; 127.0.0.1 elsewhere, or as a fallback).
    pub wallet_host: Option<String>,
}

#[tauri::command]
pub async fn node_status(st: St<'_>) -> Result<NodeStatus, String> {
    let s = st.node.settings();
    let e = match enforcer::chain(&s.enforcer).await {
        Ok((h, n)) => EnforcerStatus {
            address: s.enforcer.clone(),
            remote: !is_loopback(&s.enforcer),
            reachable: true,
            height: h,
            network: n,
            error: None,
        },
        Err(err) => EnforcerStatus {
            address: s.enforcer.clone(),
            remote: !is_loopback(&s.enforcer),
            reachable: false,
            height: 0,
            network: String::new(),
            error: Some(err),
        },
    };
    let run = st.node.state();
    let (mut height, mut sync, mut peers) = (None, None, None);
    if run == RunState::Running {
        if let Some(rpc) = st.node.rpc() {
            height = rpc.public::<u64>("getblockcount", json!([])).await.ok();
            sync = rpc.public::<Value>("mainchain_sync_progress", json!([])).await.ok();
            peers = rpc.public::<Vec<Value>>("list_peers", json!([])).await.ok().map(|p| p.len());
        }
    }
    Ok(NodeStatus {
        installed: st.node.installed(),
        own_program: s.node_binary.is_some(),
        run,
        network: s.network,
        height,
        sync,
        peers,
        rpc_port: s.rpc_port,
        uptime_secs: st.node.uptime_secs(),
        enforcer: e,
        wallet_host: st.node.wallet_host(),
    })
}

#[derive(Serialize)]
pub struct EnforcerCheck {
    /// The address as it would be saved.
    pub address: String,
    /// It answers, and (in a release build) follows eCash beta.
    pub ok: bool,
    pub remote: bool,
    pub height: u32,
    pub network: String,
    /// Why not, or what it found, in words for the screen.
    pub detail: String,
}

/// Try an enforcer address as typed, saved or not: does it answer, and in a release build, is it on eCash beta (the
/// check the node's start makes)? Nothing is sent but the enforcer's read-only chain questions.
#[tauri::command]
pub async fn enforcer_test(address: String) -> EnforcerCheck {
    let mut c = EnforcerCheck { address: address.trim().into(), ok: false, remote: false, height: 0, network: String::new(), detail: String::new() };
    let a = match crate::settings::clean_enforcer(&address) {
        Ok(a) => a,
        Err(e) => {
            c.detail = e;
            return c;
        }
    };
    c.address = a.clone();
    c.remote = !is_loopback(&a);
    match enforcer::chain(&a).await {
        Ok((h, n)) => {
            c.height = h;
            c.network = n;
        }
        Err(_) => {
            c.detail = if c.remote {
                format!(
                    "Nothing answers at {a}. On that computer the enforcer must listen on an address this computer can \
                     reach (BitWindow starts it on 127.0.0.1 only), and its firewall must let this computer in."
                )
            } else {
                format!("Nothing answers at {a}. Is eCash running in BitWindow?")
            };
            return c;
        }
    }
    if crate::settings::BETA_ONLY {
        if let Err(e) = enforcer::check_ecash_beta(&a).await {
            c.detail = e;
            return c;
        }
    }
    c.ok = true;
    c.detail = format!("It answers: eCash block {}", c.height);
    c
}

/// Use this enforcer from the node's next start (Setup, when none answers on this computer).
#[tauri::command]
pub fn enforcer_set(st: St<'_>, address: String) -> Result<String, String> {
    let a = crate::settings::clean_enforcer(&address)?;
    let mut s = st.node.settings.lock().unwrap();
    let mut n = s.clone();
    n.enforcer = a.clone();
    n.save(&st.dir).map_err(|e| e.to_string())?;
    *s = n;
    Ok(a)
}

pub fn is_loopback(addr: &str) -> bool {
    let host = addr.rsplit_once(':').map(|x| x.0).unwrap_or(addr).trim_matches(|c| c == '[' || c == ']');
    host == "localhost" || host.parse::<std::net::IpAddr>().map(|ip| ip.is_loopback()).unwrap_or(false)
}

/// Download, check and install the node, in the background; `install_progress` follows it.
#[tauri::command]
pub async fn node_install(st: St<'_>) -> Result<(), String> {
    {
        let p = st.node.install.lock().unwrap();
        if p.running {
            return Ok(());
        }
    }
    *st.node.install.lock().unwrap() = install::InstallProgress { running: true, ..Default::default() };
    let node = st.node.clone();
    tauri::async_runtime::spawn(async move {
        let web = reqwest::Client::builder().user_agent(concat!("truthcoin-app/", env!("CARGO_PKG_VERSION"))).build();
        let r = match web {
            Ok(web) => install::install(&web, &node.dir, |p| *node.install.lock().unwrap() = p).await,
            Err(e) => Err(e.to_string()),
        };
        let mut p = node.install.lock().unwrap();
        p.running = false;
        match r {
            Ok(_) => {
                p.finished = true;
                crate::activity::note(&node.dir, &format!("node {} installed", pins::NODE_VERSION));
            }
            Err(e) => p.error = Some(e),
        }
    });
    Ok(())
}

#[tauri::command]
pub fn install_progress(st: St<'_>) -> install::InstallProgress {
    st.node.install.lock().unwrap().clone()
}

#[tauri::command]
pub async fn node_start(st: St<'_>) -> Result<(), String> {
    st.node.start().await
}

#[tauri::command]
pub async fn node_stop(st: St<'_>) -> Result<(), String> {
    st.node.stop().await;
    Ok(())
}

#[tauri::command]
pub fn node_log(st: St<'_>) -> String {
    st.node.log_tail(200)
}

#[derive(Serialize, Deserialize)]
pub struct Advanced {
    pub network: String,
    pub enforcer: String,
    pub rpc_port: u16,
    pub p2p_addr: String,
    pub zmq_port: u16,
}

#[tauri::command]
pub fn settings_advanced(st: St<'_>) -> Advanced {
    let s = st.node.settings();
    Advanced { network: s.network, enforcer: s.enforcer, rpc_port: s.rpc_port, p2p_addr: s.p2p_addr, zmq_port: s.zmq_port }
}

/// Change the node's network settings; they apply at its next start.
#[tauri::command]
pub fn settings_advanced_set(st: St<'_>, a: Advanced) -> Result<(), String> {
    if !["betanet", "forknet", "signet", "regtest"].contains(&a.network.as_str()) {
        return Err("unknown network".into());
    }
    if crate::settings::BETA_ONLY && a.network != "betanet" {
        return Err("This release runs only on eCash beta".into());
    }
    let enforcer = crate::settings::clean_enforcer(&a.enforcer)?;
    let mut s = st.node.settings.lock().unwrap();
    let mut n = s.clone();
    n.network = a.network;
    n.enforcer = enforcer;
    a.p2p_addr.parse::<std::net::SocketAddr>().map_err(|_| "the P2P address needs an IP and a port")?;
    n.rpc_port = a.rpc_port;
    n.p2p_addr = a.p2p_addr;
    n.zmq_port = a.zmq_port;
    n.save(&st.dir).map_err(|e| e.to_string())?;
    *s = n;
    Ok(())
}

// --- markets ---

#[derive(Serialize)]
pub struct MarketRow {
    #[serde(flatten)]
    pub summary: MarketSummary,
    /// The outcome with the highest chance, and that chance (UX review M4).
    pub leading: Option<(String, f64)>,
}

#[tauri::command]
pub async fn markets(st: St<'_>) -> Result<Vec<MarketRow>, String> {
    let rpc = st.node.rpc_or_err()?;
    let mut out = vec![];
    for m in markets::list(&rpc).await? {
        let leading = match markets::get(&rpc, &m.market_id).await {
            Ok(v) => v["outcomes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|o| {
                    let i = o["outcome_index"].as_u64()? as u32;
                    markets::outcome(&v, i)
                })
                .max_by(|a, b| a.1.total_cmp(&b.1)),
            Err(_) => None,
        };
        out.push(MarketRow { summary: m, leading });
    }
    Ok(out)
}

#[derive(Serialize)]
pub struct DecisionView {
    pub id: String,
    pub question: String,
    pub rules: String,
    pub period: u64,
}

#[derive(Serialize)]
pub struct MarketDetail {
    pub market: Value,
    pub holdings: Vec<Holding>,
    pub height: u64,
    /// The questions behind the market: how each is decided, and in which voting period (UX review M4).
    pub decisions: Vec<DecisionView>,
    pub current_period: u64,
    pub blocks_per_period: Option<u64>,
    pub testing: bool,
}

#[tauri::command]
pub async fn market(st: St<'_>, id: String) -> Result<MarketDetail, String> {
    let rpc = st.node.rpc_or_err()?;
    let market = markets::get(&rpc, &id).await?;
    let holdings = markets::holdings(&rpc, &st.trades).await?.into_iter().filter(|h| h.market_id == id).collect();
    let height = rpc.public("getblockcount", json!([])).await.unwrap_or(0);
    let mut decisions = vec![];
    for d in market["dimensions"].as_array().into_iter().flatten() {
        let Some(did) = d["decision_id"].as_str() else { continue };
        let v: Value = rpc.public("decision_get", json!([did])).await.unwrap_or(Value::Null);
        let info = &v["content"]["Decision"];
        decisions.push(DecisionView {
            id: did.into(),
            question: info["header"].as_str().or(d["name"].as_str()).unwrap_or("").into(),
            rules: info["description"].as_str().unwrap_or("").into(),
            period: v["period_index"].as_u64().unwrap_or(0),
        });
    }
    let s: Value = rpc.public("decision_status", json!([])).await.unwrap_or(Value::Null);
    Ok(MarketDetail {
        market,
        holdings,
        height,
        decisions,
        current_period: s["current_period"].as_u64().unwrap_or(0),
        blocks_per_period: s["blocks_per_period"].as_u64(),
        testing: s["is_testing_mode"].as_bool().unwrap_or(false),
    })
}

#[derive(Serialize, Clone)]
pub struct SettledOutcome {
    pub label: String,
    pub shares: u64,
    /// What each share paid (0 to 1 sat).
    pub per_share: f64,
}

#[derive(Serialize, Clone)]
pub struct Settled {
    pub market_id: String,
    pub title: String,
    /// The winning outcomes' labels.
    pub winners: Vec<String>,
    /// What the shares the app's own trades left you holding were paid, by the node's final prices.
    pub paid_sats: u64,
    pub shares: u64,
    pub outcomes: Vec<SettledOutcome>,
}

/// Settled markets the app traded in, and what they paid (UX review M2). The node pays out by itself; this works it
/// out from the app's own record of trades and each market's final prices.
#[tauri::command]
pub async fn settled(st: St<'_>) -> Result<Vec<Settled>, String> {
    settled_for(&st.node.rpc_or_err()?, &st.trades).await
}

pub async fn settled_for(rpc: &crate::rpc::Rpc, trades: &crate::trades::Trades) -> Result<Vec<Settled>, String> {
    let mut by_market: std::collections::BTreeMap<String, std::collections::BTreeMap<u32, i128>> = Default::default();
    for t in trades.all().iter().filter(|t| t.status == crate::trades::Status::Done) {
        let e = by_market.entry(t.market_id.clone()).or_default().entry(t.outcome).or_default();
        *e += if t.side == Side::Buy { t.shares as i128 } else { -(t.shares as i128) };
    }
    let mut out = vec![];
    for (id, held) in by_market {
        let Ok(m) = markets::get(rpc, &id).await else { continue };
        if m["state"].as_str() != Some("settled") {
            continue;
        }
        let price = |i: u32| -> f64 {
            m["resolution"]["winning_outcomes"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|w| w["outcome_index"].as_u64() == Some(i as u64))
                .and_then(|w| w["price"].as_f64())
                .unwrap_or(0.0)
        };
        let shares: u64 = held.values().map(|n| (*n).max(0) as u64).fold(0u64, |a, b| a.saturating_add(b));
        let paid = held
            .iter()
            .map(|(i, n)| ((*n).max(0) as f64 * price(*i)).floor() as u64)
            .fold(0u64, |a, b| a.saturating_add(b));
        let winners = m["resolution"]["winning_outcomes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|w| markets::outcome(&m, w["outcome_index"].as_u64()? as u32).map(|x| x.0))
            .collect();
        let outcomes = held
            .iter()
            .filter(|(_, n)| **n > 0)
            .map(|(i, n)| SettledOutcome {
                label: markets::outcome(&m, *i).map(|x| x.0).unwrap_or_default(),
                shares: *n as u64,
                per_share: price(*i),
            })
            .collect();
        out.push(Settled {
            market_id: id,
            title: m["title"].as_str().unwrap_or("").into(),
            winners,
            paid_sats: paid,
            shares,
            outcomes,
        });
    }
    Ok(out)
}

#[tauri::command]
pub async fn positions(st: St<'_>) -> Result<Vec<Holding>, String> {
    markets::holdings(&st.node.rpc_or_err()?, &st.trades).await
}

#[tauri::command]
pub async fn trade_quote(st: St<'_>, market_id: String, outcome: u32, shares: u64, side: Side) -> Result<Quote, String> {
    markets::quote(&st.node.rpc_or_err()?, &market_id, outcome, shares, side).await
}

#[tauri::command]
pub async fn trade_place(
    st: St<'_>,
    market_id: String,
    outcome: u32,
    shares: u64,
    side: Side,
    limit_sats: u64,
) -> Result<Trade, String> {
    let rpc = st.node.rpc_or_err()?;
    let id = hex::encode(rand::random::<[u8; 16]>());
    Ok(markets::place(&rpc, &st.trades, &id, "desktop", &market_id, outcome, shares, side, limit_sats).await?)
}

#[derive(Serialize)]
pub struct TradeList {
    pub trades: Vec<Trade>,
    pub height: u64,
}

#[tauri::command]
pub async fn trades(st: St<'_>) -> Result<TradeList, String> {
    let rpc = st.node.rpc_or_err()?;
    let _ = markets::refresh(&rpc, &st.trades).await;
    let mut trades = st.trades.all();
    trades.reverse();
    trades.truncate(200);
    Ok(TradeList { trades, height: rpc.public("getblockcount", json!([])).await.unwrap_or(0) })
}

/// "It didn't go through": a trade that never got an answer from the node leaves the waiting list.
#[tauri::command]
pub fn trade_clear(st: St<'_>, id: String) -> Result<(), String> {
    st.trades.clear(&id)
}

#[tauri::command]
pub async fn trade_cancel(st: St<'_>, id: String) -> Result<(), String> {
    markets::cancel(&st.node.rpc_or_err()?, &st.trades, &id).await
}
