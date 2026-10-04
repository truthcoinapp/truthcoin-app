//! The phone link (docs/PROTOCOL.md): a paired phone sends sealed requests over public Nostr relays, and the desktop
//! answers them from its own node, through a narrow door: markets, positions, balance, quotes, trades within the
//! phone's daily limit, and receiving addresses. Nothing else is reachable from a phone.
//!
//! Files, all owner-only in `<app data>/phone/`: `keys.json` (the desktop's P-256 key D and its Nostr key),
//! `devices.json` (paired phones), `answers.json` (every trade request's id and answer, 24 hours: a repeat gets the
//! stored answer and never runs twice) and `held.json` (trades over a phone's limit, waiting for the desktop).

pub mod commands;
pub mod crypto;
pub mod nostr;
pub mod relays;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod vectors;

use crate::markets;
use crate::node::{Node, RunState};
use crate::trades::{Side, Trades};
use crypto::{b64u, parse_pub, pub_b64u, Envelope};
use nostr::{Event, NostrKey};
use p256::{PublicKey, SecretKey};
use relays::RelayPool;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// How long a pairing code lives.
pub const PAIR_SECS: u64 = 300;
/// How far a request's clock may be from ours.
pub const SKEW_SECS: u64 = 300;
/// How long answers are kept.
pub const ANSWER_SECS: u64 = 24 * 3600;
/// New requests a phone may make in a minute, and repeats (asking again under a known id), counted apart: ordinary
/// use (Home, a market, a quote, their resends) stays well under both (UX review B2).
/// Together they bound what one phone can make the desktop publish (about 95 events a minute), so a misbehaving phone
/// can't get the desktop's key rate-limited on the relays (review U2).
pub const MAX_PER_MINUTE: usize = 60;
pub const MAX_REPEATS_PER_MINUTE: usize = 30;
/// "Busy" answers sent in a minute, at most: over-limit requests get one, so the phone can say so and ask again.
const MAX_BUSY_PER_MINUTE: usize = 5;
const MARKETS_PER_PAGE: usize = 25;
/// Trades a phone may have waiting for the desktop at once, and waiting for their block (review M2, L7).
pub const MAX_HELD: usize = 3;
pub const MAX_WAITING: usize = 3;
/// A held trade not answered within this is dropped (review L8).
pub const HELD_SECS: u64 = 3600;
/// A phone's sell may not give up more than this share of its value to the price moving (review M1).
const MAX_SELL_IMPACT: f64 = 0.2;

#[derive(Serialize, Deserialize)]
struct KeyFile {
    d: String,
    nd: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Device {
    pub name: String,
    /// The phone's P-256 key, b64u.
    pub p: String,
    /// The phone's Nostr key, hex.
    pub np: String,
    pub limit_sats: u64,
    pub paired_at: u64,
    pub last_seen: u64,
}

impl Device {
    /// How the trade log names this phone (its limit is counted by this).
    pub fn source(&self) -> String {
        format!("phone:{}", &self.np[..16])
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Answer {
    pub np: String,
    pub at: u64,
    /// The latest reply, resent to a repeat.
    pub reply: Value,
    /// False while the request is held for the desktop, or was cut off before it was answered.
    pub done: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Held {
    pub id: String,
    pub np: String,
    pub name: String,
    pub at: u64,
    pub market_id: String,
    pub title: String,
    pub outcome: u32,
    pub label: String,
    pub shares: u64,
    pub side: Side,
    pub limit_sats: u64,
    /// What it counts against the phone's limit (a buy's cap, a sell's value).
    #[serde(default)]
    pub charge_sats: u64,
}

struct Claim {
    id: String,
    name: String,
    p: String,
    np: String,
    /// The commitment nonce sent to this phone; the code is over it.
    nonce: [u8; 16],
    code: String,
    /// When the nonce was last sent, and how many times: a resend at most every 2 s, 10 in all (rereview R4).
    sent: (Instant, u32),
}

struct Pairing {
    c: [u8; 16],
    expires: u64,
    claim: Option<Claim>,
    /// A second phone opened a request with this code while the first claim was shown: someone else has the code.
    contested: bool,
    /// "allowed" or "refused" once decided.
    result: Option<String>,
}

pub struct Phone {
    dir: PathBuf,
    d: SecretKey,
    nk: NostrKey,
    devices: Mutex<Vec<Device>>,
    /// Keyed by "<phone's Nostr key>:<request id>" (review N1).
    answers: Mutex<HashMap<String, Answer>>,
    reads: Mutex<HashMap<String, (Instant, Value)>>,
    held: Mutex<Vec<Held>>,
    pairing: Mutex<Option<Pairing>>,
    pool: Mutex<Option<RelayPool>>,
    rate: Mutex<HashMap<String, VecDeque<Instant>>>,
    /// One handler for the link's whole life, fed by whichever pool is connected, and one record of events seen: a
    /// reconnect can't hand the same request to two handlers (review L2).
    incoming: Mutex<Option<mpsc::Sender<Event>>>,
    seen: Arc<Mutex<relays::Seen>>,
    gate: Arc<relays::Gate>,
    /// The address last given to a phone: given again until it has been used (review L6).
    last_receive: Mutex<Option<String>>,

    /// Records that couldn't be read or set aside: never written over.
    frozen: Vec<PathBuf>,
    pub node: Arc<Node>,
    pub trades: Arc<Trades>,
}

fn key(np: &str, id: &str) -> String {
    format!("{np}:{id}")
}

fn now() -> u64 {
    crate::activity::unix_now()
}

fn load_keys(dir: &Path) -> Result<(SecretKey, NostrKey), String> {
    let path = dir.join("keys.json");
    if let Some(k) = crate::files::read_json::<KeyFile>(&path).map_err(|e| e.to_string())? {
        let d = SecretKey::from_slice(&hex::decode(&k.d).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        let nk = NostrKey::from_bytes(&hex::decode(&k.nd).map_err(|e| e.to_string())?)?;
        return Ok((d, nk));
    }
    let d = crypto::random_secret();
    let nk = NostrKey::random();
    let kf = KeyFile { d: hex::encode(d.to_bytes()), nd: hex::encode(nk.to_bytes()) };
    crate::files::write_json(&path, &kf).map_err(|e| e.to_string())?;
    Ok((d, nk))
}

impl Phone {
    pub fn new(app_dir: &Path, node: Arc<Node>, trades: Arc<Trades>) -> Result<Phone, String> {
        let dir = app_dir.join("phone");
        crate::files::private_dir(&dir).map_err(|e| e.to_string())?;
        let (d, nk) = load_keys(&dir)?;
        use crate::trades::load_or_set_aside;
        let (devices, bad_devices): (Vec<Device>, _) = load_or_set_aside(&dir.join("devices.json"));
        let (mut answers, bad_answers): (HashMap<String, Answer>, _) = load_or_set_aside(&dir.join("answers.json"));
        let (held, bad_held): (Vec<Held>, _) = load_or_set_aside(&dir.join("held.json"));
        answers.retain(|_, a| a.at + ANSWER_SECS > now());
        // A trade answered "held" whose card is gone (the app stopped between the two writes) is over: nothing ran.
        for (k, a) in answers.iter_mut() {
            if !a.done && a.reply.get("held").is_some() && !held.iter().any(|h: &Held| key(&h.np, &h.id) == *k) {
                a.reply = json!({"re": a.reply["re"], "err": "Not done: the computer stopped before you answered it"});
                a.done = true;
            }
        }
        let frozen: Vec<PathBuf> = [&bad_devices, &bad_answers, &bad_held]
            .into_iter()
            .flatten()
            .filter(|p| !p.to_string_lossy().contains(".bad-"))
            .cloned()
            .collect();
        let bad: Vec<String> = [bad_devices, bad_answers, bad_held, trades.unreadable.clone()]
            .into_iter()
            .flatten()
            .map(|p| p.display().to_string())
            .collect();
        if !bad.is_empty() {
            crate::activity::note(app_dir, &format!("records couldn't be read; set aside: {}", bad.join(", ")));
        }
        let gate = Arc::new(relays::Gate::default());
        *gate.keys.write().unwrap() = devices.iter().map(|d| d.np.clone()).collect();
        Ok(Phone {
            dir,
            d,
            nk,
            devices: Mutex::new(devices),
            answers: Mutex::new(answers),
            reads: Mutex::new(HashMap::new()),
            held: Mutex::new(held),
            pairing: Mutex::new(None),
            pool: Mutex::new(None),
            rate: Mutex::new(HashMap::new()),
            incoming: Mutex::new(None),
            seen: Arc::new(Mutex::new(relays::Seen::default())),
            gate,
            last_receive: Mutex::new(None),
            frozen,
            node,
            trades,
        })
    }

    pub fn d_pub(&self) -> PublicKey {
        self.d.public_key()
    }

    pub fn devices(&self) -> Vec<Device> {
        self.devices.lock().unwrap().clone()
    }

    fn save_to<T: Serialize>(&self, name: &str, v: &T) -> Result<(), String> {
        let p = self.dir.join(name);
        if self.frozen.contains(&p) {
            return Err(format!("{name} couldn't be read or set aside, so it isn't written over"));
        }
        crate::files::write_json(&p, v).map_err(|e| e.to_string())
    }

    fn save_devices(&self, l: &[Device]) -> Result<(), String> {
        self.save_to("devices.json", &l)
    }

    fn save_answers(&self, a: &HashMap<String, Answer>) -> Result<(), String> {
        self.save_to("answers.json", a)
    }

    /// Trades waiting for the desktop. Any older than an hour are dropped first, and their phones told.
    pub fn held(&self) -> Vec<Held> {
        let expired: Vec<Held> = {
            let mut l = self.held.lock().unwrap();
            let (old, keep): (Vec<Held>, Vec<Held>) = l.drain(..).partition(|h| h.at + HELD_SECS < now());
            *l = keep;
            if !old.is_empty() {
                let _ = self.save_held(&l);
            }
            old
        };
        for h in expired {
            let r = json!({"re": h.id, "err": "Not done: it waited over an hour for your computer"});
            self.remember(&h.id, &h.np, &r, true);
            self.tell(&h.np, &r);
        }
        self.held.lock().unwrap().clone()
    }

    /// Why phones may not trade: records set aside as unreadable (`*.bad-*`) that the user hasn't looked at. It lasts
    /// across restarts until `records_seen` (re-review R4).
    pub fn blocked(&self) -> Option<String> {
        let bad = [crate::trades::set_aside_in(&self.node.dir), crate::trades::set_aside_in(&self.dir)].concat();
        (!bad.is_empty())
            .then(|| "The computer couldn't read some of its trade records, so phones can't trade until you look".into())
    }

    /// The user has looked at the set-aside records: phones may trade again.
    pub fn records_seen(&self) {
        crate::trades::mark_seen(&self.node.dir);
        crate::trades::mark_seen(&self.dir);
        crate::activity::note(&self.node.dir, "set-aside records marked as looked at");
    }

    /// Send a reply to a phone, if it is still paired.
    fn tell(&self, np: &str, reply: &Value) {
        let dev = self.devices.lock().unwrap().iter().find(|d| d.np == np).cloned();
        if let Some(d) = dev {
            if let Ok(p) = parse_pub(&d.p) {
                self.send(&p, &d.np, reply);
            }
        }
    }

    fn save_held(&self, h: &[Held]) -> Result<(), String> {
        self.save_to("held.json", &h)
    }

    pub fn relay_status(&self) -> Vec<relays::RelayStatus> {
        self.pool.lock().unwrap().as_ref().map(|p| p.status()).unwrap_or_default()
    }

    fn wanted(&self) -> bool {
        !self.devices.lock().unwrap().is_empty() || self.pairing.lock().unwrap().is_some()
    }

    /// Connect to the relays if there is anything to listen for (a paired phone, or pairing under way).
    pub fn ensure_running(self: &Arc<Self>) {
        if !self.wanted() || self.pool.lock().unwrap().is_some() {
            return;
        }
        let tx = {
            let mut inc = self.incoming.lock().unwrap();
            if inc.is_none() {
                let (tx, mut rx) = mpsc::channel::<Event>(256);
                let me = self.clone();
                tauri::async_runtime::spawn(async move {
                    while let Some(ev) = rx.recv().await {
                        me.handle(ev).await;
                    }
                });
                *inc = Some(tx);
            }
            inc.clone().expect("set above")
        };
        let relays = self.node.settings().relays;
        let pool = RelayPool::start(&relays, self.nk.pubkey(), tx, self.seen.clone(), self.gate.clone());
        *self.pool.lock().unwrap() = Some(pool);
    }

    /// Disconnect when nothing is paired and no pairing is under way (review N8).
    fn stop_if_idle(&self) {
        if !self.wanted() {
            *self.pool.lock().unwrap() = None;
        }
    }

    /// Reconnect with the relays now in Settings.
    pub fn restart(self: &Arc<Self>) {
        *self.pool.lock().unwrap() = None;
        self.ensure_running();
    }

    fn publish(&self, ev: Event) {
        if let Some(p) = self.pool.lock().unwrap().as_ref() {
            p.publish(ev);
        }
    }

    /// Seal `reply` to a phone and send it.
    fn send(&self, p: &PublicKey, np: &str, reply: &Value) {
        let Ok(pt) = crypto::pad(reply.to_string().as_bytes()) else {
            let msg = json!({"re": reply["re"], "err": "The answer is too big to send"});
            return self.send(p, np, &msg);
        };
        let env = crypto::seal_msg_random(&self.d, p, &pt);
        let content = serde_json::to_string(&env).expect("serializable");
        self.publish(self.nk.message(np, content, now()));
    }

    // --- pairing ---

    /// A new pairing code; the QR code's URL.
    pub fn pair_start(self: &Arc<Self>, page: &str) -> String {
        let c: [u8; 16] = rand::random();
        let expires = now() + PAIR_SECS;
        *self.pairing.lock().unwrap() = Some(Pairing { c, expires, claim: None, contested: false, result: None });
        self.gate.pairing.store(true, std::sync::atomic::Ordering::Relaxed);
        self.ensure_running();
        let relays: Vec<String> = self.node.settings().relays.into_iter().take(5).collect();
        let j = json!({"v": 1, "r": relays, "n": self.nk.pubkey(), "d": pub_b64u(&self.d.public_key()),
                       "c": b64u(&c), "x": expires});
        format!("{page}#pair={}", b64u(j.to_string().as_bytes()))
    }

    /// Where pairing stands: "waiting", "claimed" (with the phone's name and the comparison code), "allowed",
    /// "refused", "expired" or "none".
    pub fn pair_state(&self) -> Value {
        let g = self.pairing.lock().unwrap();
        if g.as_ref().is_some_and(|p| p.expires < now() && p.claim.is_none()) {
            self.gate.pairing.store(false, std::sync::atomic::Ordering::Relaxed);
        }
        match g.as_ref() {
            None => json!({"state": "none"}),
            Some(p) if p.result.is_some() => json!({"state": p.result}),
            Some(p) if p.contested => json!({"state": "contested"}),
            Some(p) if p.expires < now() && p.claim.is_none() => json!({"state": "expired"}),
            Some(Pairing { claim: Some(c), .. }) => json!({"state": "claimed", "name": c.name, "code": c.code}),
            Some(p) => json!({"state": "waiting", "expires": p.expires}),
        }
    }

    /// The desktop's yes or no to the phone that claimed the code.
    pub fn pair_answer(&self, allow: bool) -> Result<(), String> {
        let (claim, limit) = {
            let mut g = self.pairing.lock().unwrap();
            let p = g.as_mut().ok_or("no pairing under way")?;
            if p.result.is_some() {
                return Err("already answered".into());
            }
            if allow && p.contested {
                return Err("Two phones tried to pair with this code: refuse, and start again".into());
            }
            let claim = p.claim.take().ok_or("no phone has asked yet")?;
            p.result = Some(if allow { "allowed" } else { "refused" }.into());
            (claim, self.node.settings().phone_daily_limit_sats)
        };
        self.gate.pairing.store(false, std::sync::atomic::Ordering::Relaxed);
        let p_pub = parse_pub(&claim.p)?;
        if !allow {
            self.send(&p_pub, &claim.np, &json!({"re": claim.id, "err": "not allowed"}));
            return Ok(());
        }
        {
            let mut d = self.devices.lock().unwrap();
            d.retain(|x| x.np != claim.np && x.p != claim.p);
            d.push(Device {
                name: claim.name.clone(),
                p: claim.p.clone(),
                np: claim.np.clone(),
                limit_sats: limit,
                paired_at: now(),
                last_seen: now(),
            });
            self.save_devices(&d)?;
            *self.gate.keys.write().unwrap() = d.iter().map(|x| x.np.clone()).collect();
        }
        crate::activity::note(&self.node.dir, &format!("phone paired: {}", claim.name));
        self.send(&p_pub, &claim.np, &json!({"re": claim.id, "ok": {"paired": true, "name": claim.name, "limit_sats": limit}}));
        Ok(())
    }

    pub fn pair_cancel(&self) {
        *self.pairing.lock().unwrap() = None;
        self.gate.pairing.store(false, std::sync::atomic::Ordering::Relaxed);
        self.stop_if_idle();
    }

    /// A pairing request: the first one that opens with the live code claims it, and gets the commitment nonce; the
    /// same phone asking again gets the same nonce again; another phone makes it contested.
    fn try_pair(&self, ev: &Event, env: &Envelope) {
        let Some((p_pub, np, reply)) = self.claim(ev, env) else { return };
        self.send(&p_pub, &np, &reply);
    }

    fn claim(&self, ev: &Event, env: &Envelope) -> Option<(PublicKey, String, Value)> {
        let mut g = self.pairing.lock().unwrap();
        let p = g.as_mut()?;
        if p.result.is_some() || p.expires < now() {
            return None;
        }
        let (pt, e_pub) = crypto::open_pair(&self.d, &p.c, env).ok()?;
        if let Some(c) = &p.claim {
            // The same phone asking again (its answer was lost) gets the same nonce; another phone, while one is
            // shown, means the code is out.
            let other = serde_json::from_slice::<Value>(&pt)
                .ok()
                .and_then(|v| v["p"].as_str().and_then(|k| parse_pub(k).ok()))
                .map(|k| pub_b64u(&k));
            if other.as_deref() == Some(c.p.as_str()) && ev.pubkey == c.np {
                if c.sent.0.elapsed() < Duration::from_secs(2) || c.sent.1 >= 10 {
                    return None;
                }
                let reply = json!({"re": c.id, "nonce": b64u(&c.nonce)});
                let target = (parse_pub(&c.p).ok()?, c.np.clone());
                if let Some(c) = p.claim.as_mut() {
                    c.sent = (Instant::now(), c.sent.1 + 1);
                }
                return Some((target.0, target.1, reply));
            }
            if other.is_some() {
                p.contested = true;
            }
            return None;
        }
        let v: Value = serde_json::from_slice(&pt).ok()?;
        if v["t"] != "pair" || v["np"].as_str() != Some(ev.pubkey.as_str()) || !nostr::valid_pubkey(&ev.pubkey) {
            return None;
        }
        let id = v["id"].as_str().filter(|i| valid_id(i))?.to_string();
        let p_pub = parse_pub(v["p"].as_str()?).ok()?;
        let nonce: [u8; 16] = rand::random();
        let code = crypto::pair_code(&self.d.public_key(), &p_pub, &e_pub, &p.c, &nonce);
        let reply = json!({"re": id, "nonce": b64u(&nonce)});
        p.claim = Some(Claim {
            id,
            name: crypto::clean_name(v["name"].as_str().unwrap_or("")),
            p: pub_b64u(&p_pub),
            np: ev.pubkey.clone(),
            nonce,
            code,
            sent: (Instant::now(), 1),
        });
        Some((p_pub, ev.pubkey.clone(), reply))
    }

    // --- requests ---

    pub async fn handle(self: &Arc<Self>, ev: Event) {
        let Ok(env) = serde_json::from_str::<Envelope>(&ev.content) else { return };
        let dev = self.devices.lock().unwrap().iter().find(|d| d.np == ev.pubkey).cloned();
        let Some(dev) = dev else {
            if env.k == "pair" {
                self.try_pair(&ev, &env);
            }
            return;
        };
        let Ok(p_pub) = parse_pub(&dev.p) else { return };
        let Ok(pt) = crypto::open_msg(&self.d, &p_pub, &env) else { return };
        let Ok(req) = serde_json::from_slice::<Value>(&pt) else { return };
        let Some(id) = req["id"].as_str().filter(|i| valid_id(i)).map(String::from) else { return };
        // A request already answered (or held) gets its answer again, whatever its time. Repeats have a rate of their
        // own: each answer is an event published (review N11).
        let k = key(&dev.np, &id);
        let known = self.answers.lock().unwrap().get(&k).map(|a| a.reply.clone());
        let known = known.or_else(|| self.reads.lock().unwrap().get(&k).map(|r| r.1.clone()));
        if let Some(r) = known {
            if self.within_rate(&format!("{}:repeat", dev.np), MAX_REPEATS_PER_MINUTE) {
                self.send(&p_pub, &dev.np, &r);
            }
            return;
        }
        // Over the rate, a new request isn't run: the phone is told (a few times a minute at most) and asks again.
        if !self.within_rate(&dev.np, MAX_PER_MINUTE) {
            if self.within_rate(&format!("{}:busy", dev.np), MAX_BUSY_PER_MINUTE) {
                let r = json!({"re": id, "err": "Your computer is busy: ask again in a few seconds", "busy": true});
                self.send(&p_pub, &dev.np, &r);
            }
            return;
        }
        {
            let mut d = self.devices.lock().unwrap();
            if let Some(x) = d.iter_mut().find(|x| x.np == dev.np) {
                x.last_seen = now();
            }
        }
        let ts = req["ts"].as_u64().unwrap_or(0);
        if ts.abs_diff(now()) > SKEW_SECS {
            let r = json!({"re": id, "err": "This request reached the computer more than 5 minutes after it was made (or the \
                                             two clocks differ by that much), so it wasn't run"});
            return self.send(&p_pub, &dev.np, &r);
        }
        let m = req["m"].as_str().unwrap_or("");
        if m == "unpair" {
            // The phone forgot this computer: answer, then forget the phone. The relays stay up a few seconds more, so
            // the answer goes out even when this was the last phone (rereview R3).
            self.send(&p_pub, &dev.np, &json!({"re": id, "ok": {"unpaired": true}}));
            let _ = self.forget_device(&dev.np);
            crate::activity::note(&self.node.dir, &format!("phone {} unpaired itself", dev.name));
            let me = self.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_secs(5)).await;
                me.stop_if_idle();
            });
            return;
        }
        let reply = if m == "trade" {
            self.trade(&dev, &id, &req["a"]).await
        } else {
            let r = match self.read(&dev, m, &req["a"]).await {
                Ok(v) => json!({"re": id, "ok": v}),
                Err(e) => json!({"re": id, "err": e}),
            };
            let mut reads = self.reads.lock().unwrap();
            reads.retain(|_, (t, _)| t.elapsed() < Duration::from_secs(600));
            reads.insert(k, (Instant::now(), r.clone()));
            r
        };
        self.send(&p_pub, &dev.np, &reply);
    }

    fn within_rate(&self, key: &str, max: usize) -> bool {
        let mut r = self.rate.lock().unwrap();
        let q = r.entry(key.to_string()).or_default();
        while q.front().is_some_and(|t| t.elapsed() > Duration::from_secs(60)) {
            q.pop_front();
        }
        if q.len() >= max {
            return false;
        }
        q.push_back(Instant::now());
        true
    }

    /// What a phone may still spend today.
    pub fn left_today(&self, dev: &Device) -> u64 {
        let spent = self.trades.spent_since(&dev.source(), now().saturating_sub(24 * 3600));
        let held = self
            .held
            .lock()
            .unwrap()
            .iter()
            .filter(|h| h.np == dev.np)
            .map(|h| if h.charge_sats > 0 { h.charge_sats } else { h.limit_sats })
            .fold(0u64, |a, b| a.saturating_add(b));
        dev.limit_sats.saturating_sub(spent.saturating_add(held))
    }

    pub(crate) async fn read(&self, dev: &Device, m: &str, a: &Value) -> Result<Value, String> {
        if m == "status" {
            return Ok(self.status(dev).await);
        }
        let rpc = self.node.rpc().ok_or("The Truthcoin node on your computer isn't running")?;
        match m {
            "markets" => {
                let mut l = markets::list(&rpc).await?;
                l.sort_by(|a, b| (b.state == "trading").cmp(&(a.state == "trading")).then(b.created_at_height.cmp(&a.created_at_height)));
                let pages = l.len().div_ceil(MARKETS_PER_PAGE).max(1);
                let page = (a["page"].as_u64().unwrap_or(0) as usize).min(pages - 1);
                let mut items: Vec<Value> = vec![];
                for m in l.iter().skip(page * MARKETS_PER_PAGE).take(MARKETS_PER_PAGE) {
                    // The leading outcome and its chance (UX review M4): one node call per market on the page.
                    let leading = match markets::get(&rpc, &m.market_id).await {
                        Ok(v) if m.state == "trading" => v["outcomes"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|o| markets::outcome(&v, o["outcome_index"].as_u64()? as u32))
                            .max_by(|a, b| a.1.total_cmp(&b.1))
                            .map(|(label, price)| json!({"label": cut(&label, 80), "price": price})),
                        _ => None,
                    };
                    items.push(json!({"id": m.market_id, "title": cut(&m.title, 140), "state": m.state, "outcomes": m.outcome_count,
                                      "volume": m.volume_sats, "created": m.created_at_height, "leading": leading}));
                }
                Ok(json!({"markets": items, "page": page, "pages": pages}))
            }
            "market" => {
                let id = a["id"].as_str().unwrap_or("");
                let m = markets::get(&rpc, id).await?;
                let outcomes: Vec<Value> = m["outcomes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .take(64)
                    .map(|o| {
                        let i = o["outcome_index"].as_u64().unwrap_or(0) as u32;
                        let label = markets::outcome(&m, i).map(|x| x.0).unwrap_or_default();
                        json!({"i": i, "label": cut(&label, 80), "price": o["price"], "volume": o["volume_sats"]})
                    })
                    .collect();
                let resolution = if m["resolution"].is_null() {
                    Value::Null
                } else {
                    json!({"summary": cut(m["resolution"]["summary"].as_str().unwrap_or(""), 300),
                           "winners": m["resolution"]["winning_outcomes"].as_array().into_iter().flatten()
                               .filter_map(|w| w["outcome_index"].as_u64()).collect::<Vec<_>>()})
                };
                let holdings: Vec<Value> = markets::holdings(&rpc, &self.trades)
                    .await?
                    .into_iter()
                    .filter(|h| h.market_id == id)
                    .map(|h| json!({"outcome": h.outcome, "shares": h.shares, "value": h.value_sats}))
                    .collect();
                Ok(json!({"id": id, "title": cut(m["title"].as_str().unwrap_or(""), 200),
                          "description": cut(m["description"].as_str().unwrap_or(""), 1500), "state": m["state"],
                          "fee_rate": m["trading_fee_rate"], "volume": m["total_volume_sats"], "outcomes": outcomes,
                          "resolution": resolution, "holdings": holdings}))
            }
            "positions" => {
                let mut h = markets::holdings(&rpc, &self.trades).await?;
                h.sort_by(|a, b| b.value_sats.cmp(&a.value_sats));
                let total: u64 = h.iter().map(|x| x.value_sats).sum();
                let items: Vec<Value> = h
                    .iter()
                    .take(30)
                    .map(|x| json!({"market_id": x.market_id, "title": cut(&x.market_title, 120), "state": x.market_state,
                                    "outcome": x.outcome, "label": cut(&x.outcome_label, 80), "shares": x.shares,
                                    "price": x.price, "value": x.value_sats, "paid": x.paid_sats}))
                    .collect();
                Ok(json!({"positions": items, "total_value": total}))
            }
            "balance" => {
                let b = crate::wallet::balance(&rpc, &self.trades).await?;
                Ok(json!({"total": b.total_sats, "available": b.available_sats, "in_pending_trades": b.in_pending_trades_sats,
                          "pending_trades": b.pending_trades, "withdrawing": b.withdrawing_sats}))
            }
            "quote" => {
                let (id, outcome, shares, side) = trade_args(a)?;
                let q = markets::quote(&rpc, &id, outcome, shares, side).await?;
                Ok(json!({"side": q.side, "sats": q.sats, "fee": q.trading_fee_sats, "miner_fee": q.miner_fee_sats,
                          "price_now": q.price_now, "price_after": q.price_after, "limit": q.limit_sats}))
            }
            "trades" => {
                let _ = markets::refresh(&rpc, &self.trades).await;
                let src = dev.source();
                let l: Vec<Value> = self
                    .trades
                    .all()
                    .iter()
                    .rev()
                    .filter(|t| t.source == src)
                    .take(20)
                    .map(|t| json!({"id": t.id, "time": t.time, "title": cut(&t.market_title, 120), "label": cut(&t.outcome_label, 80),
                                    "side": t.side, "shares": t.shares, "sats": t.quoted_sats, "limit": t.limit_sats,
                                    "status": t.status, "txid": t.txid}))
                    .collect();
                Ok(json!({"trades": l}))
            }
            "receive" => {
                // The address given last time, until it has been used: a phone can't make the wallet grow (review L6).
                let last = self.last_receive.lock().unwrap().clone();
                if let Some(a) = last {
                    let u: Value = rpc.public("get_utxos", json!([[a]])).await.unwrap_or(json!([1]));
                    let s: Value = rpc.public("get_stxos", json!([[a]])).await.unwrap_or(json!([1]));
                    if u.as_array().is_some_and(|x| x.is_empty()) && s.as_array().is_some_and(|x| x.is_empty()) {
                        return Ok(json!({"address": a, "deposit_address": crate::wallet::deposit_form(&a)}));
                    }
                }
                let r = crate::wallet::receive(&rpc).await?;
                *self.last_receive.lock().unwrap() = Some(r.address.clone());
                Ok(json!({"address": r.address, "deposit_address": r.deposit_address}))
            }
            _ => Err("This app doesn't do that from a phone".into()),
        }
    }

    async fn status(&self, dev: &Device) -> Value {
        let s = self.node.settings();
        let state = match self.node.state() {
            RunState::Running => "running",
            RunState::Starting => "starting",
            RunState::Failed { .. } => "failed",
            _ => "stopped",
        };
        let (mut height, mut synced) = (Value::Null, false);
        if let Some(rpc) = self.node.rpc() {
            if let Ok(h) = rpc.public::<u64>("getblockcount", json!([])).await {
                height = json!(h);
            }
            if let Ok(p) = rpc.public::<Value>("mainchain_sync_progress", json!([])).await {
                synced = p["phase"].as_str() == Some("idle");
            }
        }
        let relays: Vec<String> = s.relays.into_iter().filter(|r| r.starts_with("wss://")).take(5).collect();
        json!({"app": env!("CARGO_PKG_VERSION"), "node": state, "height": height, "synced": synced, "network": s.network,
               "name": dev.name, "limit_sats": dev.limit_sats, "left_sats": self.left_today(dev), "relays": relays})
    }

    /// A trade from a phone: within its limit it goes to the node at once; over it, it waits for the desktop.
    pub(crate) async fn trade(&self, dev: &Device, id: &str, a: &Value) -> Value {
        let (market_id, outcome, shares, side) = match trade_args(a) {
            Ok(x) => x,
            Err(e) => return json!({"re": id, "err": e}),
        };
        let limit = a["limit"].as_u64().unwrap_or(0);
        if limit == 0 {
            return json!({"re": id, "err": "The trade needs its cap"});
        }
        if let Some(why) = self.blocked() {
            return json!({"re": id, "err": why});
        }
        if self.held.lock().unwrap().iter().any(|h| h.np == dev.np && h.id == id) {
            return json!({"re": id, "held": {"text": "Waiting for you to confirm on your computer"}});
        }
        let Some(rpc) = self.node.rpc() else {
            return json!({"re": id, "err": "The Truthcoin node on your computer isn't running"});
        };
        let _ = markets::refresh(&rpc, &self.trades).await;
        if self.trades.waiting(&dev.source()) >= MAX_WAITING {
            return json!({"re": id, "err": format!("{MAX_WAITING} trades from this phone are already waiting for their block")});
        }
        let q = match markets::quote(&rpc, &market_id, outcome, shares, side.clone()).await {
            Ok(q) => q,
            Err(e) => return json!({"re": id, "err": e}),
        };
        let value = (shares as f64 * q.price_now).ceil() as u64;
        let charge = match side {
            // A buy's cap must leave the margin the desktop suggests, so it isn't skipped block after block (L7), and
            // can't be far above what the shares can cost (L9).
            Side::Buy if limit < q.sats + markets::MINER_FEE + markets::margin(q.sats) / 2 => {
                return json!({"re": id, "err": "Your cap is below the price with its margin; get a new price"})
            }
            Side::Buy if limit > q.limit_sats.saturating_mul(2) => {
                return json!({"re": id, "err": "Your cap is far above the price; get a new price"})
            }
            Side::Buy => limit,
            // A sell counts at face value, a sat a share (what the shares pay at most): nothing in the market, not a
            // price pushed down first nor selling in steps, can lower that (M1, re-review R3). It may not lose much
            // of its value to the price moving, nor accept a minimum far below the suggested one.
            // The price movement alone: proceeds before the trading fee (the node's fee is at least 1,000 sats, which
            // on a small sell is a large share; quote() bounds the fee by the market's rate).
            Side::Sell if ((q.sats + q.trading_fee_sats) as f64) < value as f64 * (1.0 - MAX_SELL_IMPACT) => {
                return json!({"re": id, "err": "Selling that many at once would move the price a lot; sell fewer, or sell on your computer"})
            }
            // At most one margin below the minimum the desktop suggests (quote less miner fee and margin).
            Side::Sell if limit < q.limit_sats.saturating_sub(markets::margin(q.sats)) => {
                return json!({"re": id, "err": "Your minimum is far below the price; get a new price"})
            }
            Side::Sell => shares,
        };
        if charge > self.left_today(dev) {
            if self.held.lock().unwrap().iter().filter(|h| h.np == dev.np).count() >= MAX_HELD {
                return json!({"re": id, "err": format!("{MAX_HELD} trades from this phone are already waiting for your computer")});
            }
            let h = Held {
                id: id.into(),
                np: dev.np.clone(),
                name: dev.name.clone(),
                at: now(),
                market_id,
                title: q.market_title.clone(),
                outcome,
                label: q.outcome_label.clone(),
                shares,
                side,
                limit_sats: limit,
                charge_sats: charge,
            };
            let reply = json!({"re": id, "held": {"text": "Over this phone's limit: confirm on your computer"}});
            // The answer first, then the card: a stop between the two leaves an answer with no card, which the next
            // start turns into "not done" (never a card without its answer, which could be approved twice).
            self.remember(id, &dev.np, &reply, false);
            {
                let mut l = self.held.lock().unwrap();
                l.push(h);
                let _ = self.save_held(&l);
            }
            crate::activity::note(&self.node.dir, &format!("phone {} trade held for the desktop", dev.name));
            return reply;
        }
        self.run_trade(&rpc, dev, id, &market_id, outcome, shares, side, limit).await
    }

    /// Place a phone's trade. Its id is on disk first: if the app stops between the two, a repeat is told to check
    /// Positions, and the trade counts against the limit (the trade log has it as "sending").
    #[allow(clippy::too_many_arguments)]
    async fn run_trade(
        &self,
        rpc: &crate::rpc::Rpc,
        dev: &Device,
        id: &str,
        market_id: &str,
        outcome: u32,
        shares: u64,
        side: Side,
        limit: u64,
    ) -> Value {
        let unsure = json!({"re": id, "unsure": "Not confirmed: check Positions before trying again"});
        self.remember(id, &dev.np, &unsure, false);
        let reply = match markets::place(rpc, &self.trades, id, &dev.source(), market_id, outcome, shares, side, limit).await {
            Ok(t) => json!({"re": id, "ok": {"status": "pending", "txid": t.txid}}),
            Err(markets::PlaceError::Refused(e)) => json!({"re": id, "err": e}),
            Err(markets::PlaceError::Unsure(e)) => json!({"re": id, "unsure": e}),
        };
        self.remember(id, &dev.np, &reply, true);
        reply
    }

    fn remember(&self, id: &str, np: &str, reply: &Value, done: bool) {
        let mut a = self.answers.lock().unwrap();
        a.retain(|_, x| x.at + ANSWER_SECS > now());
        a.insert(key(np, id), Answer { np: np.into(), at: now(), reply: reply.clone(), done });
        let _ = self.save_answers(&a);
    }

    /// The desktop's answer to a held trade.
    pub async fn held_answer(&self, id: &str, approve: bool) -> Result<Value, String> {
        // Expired cards go first; an approval needs the node, checked before the card is taken (review L8).
        let _ = self.held();
        if approve {
            self.node.rpc_or_err()?;
        }
        let h = {
            let mut l = self.held.lock().unwrap();
            let i = l.iter().position(|h| h.id == id).ok_or("That trade is no longer waiting")?;
            let h = l.remove(i);
            self.save_held(&l)?;
            h
        };
        let done = self.answers.lock().unwrap().get(&key(&h.np, id)).is_some_and(|a| a.done);
        if done {
            return Err("That trade was already answered".into());
        }
        let dev = self.devices.lock().unwrap().iter().find(|d| d.np == h.np).cloned();
        let reply = if !approve {
            let r = json!({"re": id, "err": "Refused on your computer"});
            self.remember(id, &h.np, &r, true);
            r
        } else {
            let Some(rpc) = self.node.rpc() else {
                // The node stopped in the moment since the check: put the card back.
                let mut l = self.held.lock().unwrap();
                l.push(h);
                let _ = self.save_held(&l);
                return Err("The Truthcoin node isn't running".into());
            };
            let d = dev.clone().unwrap_or(Device {
                name: h.name.clone(),
                p: String::new(),
                np: h.np.clone(),
                limit_sats: 0,
                paired_at: 0,
                last_seen: 0,
            });
            self.run_trade(&rpc, &d, id, &h.market_id, h.outcome, h.shares, h.side.clone(), h.limit_sats).await
        };
        if let Some(d) = dev {
            if let Ok(p) = parse_pub(&d.p) {
                self.send(&p, &d.np, &reply);
            }
        }
        Ok(reply)
    }

    pub fn revoke(&self, np: &str) -> Result<(), String> {
        self.forget_device(np)?;
        self.stop_if_idle();
        Ok(())
    }

    /// Forget a phone and its held trades (the relays stay as they are).
    fn forget_device(&self, np: &str) -> Result<(), String> {
        {
            let mut d = self.devices.lock().unwrap();
            d.retain(|x| x.np != np);
            self.save_devices(&d)?;
            *self.gate.keys.write().unwrap() = d.iter().map(|x| x.np.clone()).collect();
        }
        let mut h = self.held.lock().unwrap();
        h.retain(|x| x.np != np);
        self.save_held(&h)
    }

    #[cfg(test)]
    pub(crate) fn add_device_for_test(&self, d: Device) {
        self.gate.keys.write().unwrap().insert(d.np.clone());
        self.devices.lock().unwrap().push(d);
    }

    pub fn set_limit(&self, np: &str, limit_sats: u64) -> Result<(), String> {
        let mut d = self.devices.lock().unwrap();
        let x = d.iter_mut().find(|x| x.np == np).ok_or("no such phone")?;
        x.limit_sats = limit_sats;
        self.save_devices(&d)
    }
}

pub fn valid_id(s: &str) -> bool {
    s.len() == 32 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// At most `n` bytes of `s` (whole characters only), without control characters: replies must fit their envelope
/// whatever the text (review N9).
fn cut(s: &str, n: usize) -> String {
    let mut out = String::new();
    for c in s.chars().filter(|c| !c.is_control() || *c == '\n') {
        if out.len() + c.len_utf8() > n {
            break;
        }
        out.push(c);
    }
    out
}

fn trade_args(a: &Value) -> Result<(String, u32, u64, Side), String> {
    let id = a["id"].as_str().filter(|i| markets::valid_market_id(i)).ok_or("not a market")?.to_string();
    let outcome = a["outcome"].as_u64().filter(|o| *o < 4096).ok_or("not an outcome")? as u32;
    let shares = a["shares"].as_u64().filter(|s| *s > 0 && *s <= markets::MAX_SHARES).ok_or("pick an amount of shares")?;
    let side = match a["side"].as_str() {
        Some("buy") => Side::Buy,
        Some("sell") => Side::Sell,
        _ => return Err("buy or sell?".into()),
    };
    Ok((id, outcome, shares, side))
}
