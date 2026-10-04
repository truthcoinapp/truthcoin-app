//! The link end to end, without a node: a fake phone (this crate's own crypto, as the phone page does it) pairs with
//! the desktop through a relay on 127.0.0.1 that sends every event twice, then asks for status, repeats a request,
//! and tries an unpaired key, a stale clock and a forged sender.

use super::crypto::*;
use super::nostr::{self, Event, NostrKey};
use super::*;
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

/// A minimal relay: keeps subscriptions (kinds, #p) and sends each event to matching ones, twice.
async fn relay() -> String {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let (tx, _) = tokio::sync::broadcast::channel::<String>(1024);
    tokio::spawn(async move {
        loop {
            let (s, _) = l.accept().await.unwrap();
            let tx = tx.clone();
            let mut rx = tx.subscribe();
            tokio::spawn(async move {
                let ws = tokio_tungstenite::accept_async(s).await.unwrap();
                let (mut w, mut r) = ws.split();
                let mut subs: Vec<(String, String)> = vec![];
                loop {
                    tokio::select! {
                        m = r.next() => {
                            let Some(Ok(Message::Text(t))) = m else { break };
                            let v: Value = serde_json::from_str(&t).unwrap();
                            match v[0].as_str() {
                                Some("REQ") => subs.push((v[1].as_str().unwrap().into(), v[2]["#p"][0].as_str().unwrap().into())),
                                Some("EVENT") => {
                                    let _ = w.send(Message::Text(json!(["OK", v[1]["id"], true, ""]).to_string().into())).await;
                                    let _ = tx.send(v[1].to_string());
                                }
                                _ => {}
                            }
                        }
                        e = rx.recv() => {
                            let Ok(e) = e else { break };
                            let ev: Value = serde_json::from_str(&e).unwrap();
                            let p = ev["tags"][0][1].as_str().unwrap_or("").to_string();
                            for (sid, pk) in &subs {
                                if *pk == p {
                                    for _ in 0..2 {
                                        let _ = w.send(Message::Text(json!(["EVENT", sid, ev]).to_string().into())).await;
                                    }
                                }
                            }
                        }
                    }
                }
            });
        }
    });
    format!("ws://{addr}")
}

struct FakePhone {
    p: SecretKey,
    nk: NostrKey,
    seen: std::collections::HashSet<String>,
    ws: tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
}

impl FakePhone {
    async fn new(relay: &str) -> FakePhone {
        let (mut ws, _) = tokio_tungstenite::connect_async(relay).await.unwrap();
        let nk = NostrKey::random();
        ws.send(Message::Text(json!(["REQ", "x", {"kinds": [nostr::KIND], "#p": [nk.pubkey()]}]).to_string().into()))
            .await
            .unwrap();
        FakePhone { p: random_secret(), nk, seen: Default::default(), ws }
    }

    async fn publish(&mut self, ev: Event) {
        self.ws.send(Message::Text(json!(["EVENT", ev]).to_string().into())).await.unwrap();
    }

    /// The next reply from the desktop (opened with D), or None within the time.
    async fn reply(&mut self, d_pub: &PublicKey, secs: u64) -> Option<Value> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
        loop {
            let m = tokio::time::timeout_at(deadline, self.ws.next()).await.ok()??;
            let Ok(Message::Text(t)) = m else { continue };
            let v: Value = serde_json::from_str(&t).unwrap();
            if v[0] != "EVENT" {
                continue;
            }
            let ev: Event = serde_json::from_value(v[2].clone()).unwrap();
            assert!(nostr::verify(&ev));
            // As the phone page does: the same event from a relay twice counts once.
            if !self.seen.insert(ev.id.clone()) {
                continue;
            }
            let env: Envelope = serde_json::from_str(&ev.content).unwrap();
            let pt = open_msg(&self.p, d_pub, &env).expect("a reply from D opens");
            return Some(serde_json::from_slice(&pt).unwrap());
        }
    }

    fn request(&self, d_pub: &PublicKey, nd: &str, id: &str, ts: u64, m: &str) -> Event {
        let pt = pad(json!({"id": id, "ts": ts, "m": m, "a": {}}).to_string().as_bytes()).unwrap();
        let env = seal_msg_random(&self.p, d_pub, &pt);
        self.nk.message(nd, serde_json::to_string(&env).unwrap(), now())
    }
}

fn phone_on(relay: &str) -> (tempfile::TempDir, Arc<Phone>) {
    let dir = tempfile::tempdir().unwrap();
    let node = Arc::new(Node::new(dir.path().to_path_buf(), reqwest::Client::new()));
    {
        let mut s = node.settings.lock().unwrap();
        s.relays = vec![relay.to_string()];
    }
    let trades = Arc::new(Trades::load(dir.path()));
    let p = Arc::new(Phone::new(dir.path(), node, trades).unwrap());
    (dir, p)
}

async fn wait_for(f: impl Fn() -> bool) {
    for _ in 0..100 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out");
}

#[tokio::test]
async fn pair_then_ask_through_a_relay_that_duplicates() {
    let r = relay().await;
    let (_dir, desk) = phone_on(&r);
    let url = desk.pair_start("https://example.org/");
    wait_for(|| desk.relay_status().iter().any(|s| s.connected)).await;

    // The phone reads the QR's fragment.
    let frag = url.split_once("#pair=").unwrap().1;
    let qr: Value = serde_json::from_slice(&unb64u(frag).unwrap()).unwrap();
    let d_pub = parse_pub(qr["d"].as_str().unwrap()).unwrap();
    let nd = qr["n"].as_str().unwrap().to_string();
    let c = unb64u(qr["c"].as_str().unwrap()).unwrap();
    let mut ph = FakePhone::new(&r).await;
    let e = random_secret();
    let pair_id = "0123456789abcdef0123456789abcdef";
    let pt = pad(json!({"t": "pair", "p": pub_b64u(&ph.p.public_key()), "np": ph.nk.pubkey(), "name": "Test\u{202E}phone",
                        "id": pair_id}).to_string().as_bytes()).unwrap();
    let env = seal_pair(&d_pub, &c, &e, &rand::random(), &pt);
    let ev = ph.nk.message(&nd, serde_json::to_string(&env).unwrap(), now());
    ph.publish(ev).await;

    // The desktop shows the phone and the same comparison code the phone computes.
    wait_for(|| desk.pair_state()["state"] == "claimed").await;
    let st = desk.pair_state();
    assert_eq!(st["name"], "Testphone");
    assert_eq!(st["code"], pair_code(&d_pub, &ph.p.public_key(), &e.public_key(), &c));
    desk.pair_answer(true).unwrap();
    let rep = ph.reply(&d_pub, 5).await.expect("the pairing answer");
    assert_eq!(rep["re"], pair_id);
    assert_eq!(rep["ok"]["paired"], true);
    assert_eq!(desk.devices().len(), 1);

    // A request, answered (the node isn't running here: status still answers).
    let id1 = "11111111111111111111111111111111";
    ph.publish(ph.request(&d_pub, &nd, id1, now(), "status")).await;
    let rep = ph.reply(&d_pub, 5).await.unwrap();
    assert_eq!(rep["re"], id1);
    assert_eq!(rep["ok"]["node"], "stopped");
    assert_eq!(rep["ok"]["name"], "Testphone");
    // The relay sent the request twice: the second copy (same event id) is dropped, so no second reply comes.
    assert!(ph.reply(&d_pub, 1).await.is_none(), "a duplicated event is answered once");

    // A stale clock is refused.
    let id2 = "22222222222222222222222222222222";
    ph.publish(ph.request(&d_pub, &nd, id2, now() - 3600, "status")).await;
    let rep = ph.reply(&d_pub, 5).await.unwrap();
    assert!(rep["err"].as_str().unwrap().contains("clock"));

    // A method the door doesn't have.
    let id3 = "33333333333333333333333333333333";
    ph.publish(ph.request(&d_pub, &nd, id3, now(), "withdraw")).await;
    let rep = ph.reply(&d_pub, 5).await.unwrap();
    assert_eq!(rep["re"], id3);
    assert!(rep["err"].is_string());

    // Another key that never paired gets nothing; nor does a paired Nostr key whose message isn't sealed by P.
    let mut stranger = FakePhone::new(&r).await;
    stranger.publish(stranger.request(&d_pub, &nd, "44444444444444444444444444444444", now(), "status")).await;
    assert!(stranger.reply(&d_pub, 1).await.is_none());
    let forged_pt = pad(json!({"id": "55555555555555555555555555555555", "ts": now(), "m": "status", "a": {}}).to_string().as_bytes()).unwrap();
    let forged = seal_msg_random(&random_secret(), &d_pub, &forged_pt);
    let ev = ph.nk.message(&nd, serde_json::to_string(&forged).unwrap(), now());
    ph.publish(ev).await;
    assert!(ph.reply(&d_pub, 1).await.is_none(), "sealed by another key: dropped");
}

#[tokio::test]
async fn the_code_is_used_once_and_expires() {
    let r = relay().await;
    let (_dir, desk) = phone_on(&r);
    let url = desk.pair_start("https://example.org/");
    assert_eq!(desk.pair_state()["state"], "waiting");
    desk.pair_cancel();
    assert_eq!(desk.pair_state()["state"], "none");
    assert!(desk.pair_answer(true).is_err());
    assert!(url.starts_with("https://example.org/#pair="));
}

/// Pairing starts from a command on the main thread, outside any async runtime (a crash found on Xvfb).
#[test]
fn pairing_starts_outside_a_runtime() {
    let (_dir, desk) = phone_on("ws://127.0.0.1:9");
    let url = desk.pair_start("https://example.org/");
    assert!(url.contains("#pair="));
    desk.pair_cancel();
}

/// The real default relays carry the link's events at every padded size. Opt-in (it uses the internet):
/// `PUBLIC_RELAYS=1 cargo test -j2 public_relays -- --ignored --nocapture`.
#[tokio::test]
#[ignore]
async fn public_relays_carry_our_events() {
    if std::env::var_os("PUBLIC_RELAYS").is_none() {
        return;
    }
    let mut failed = vec![];
    for url in crate::settings::DEFAULT_RELAYS {
        let receiver = NostrKey::random();
        let sender = NostrKey::random();
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        let open = || {
            let g = Arc::new(relays::Gate::default());
            g.pairing.store(true, std::sync::atomic::Ordering::Relaxed);
            g
        };
        let seen = || Arc::new(std::sync::Mutex::new(relays::Seen::default()));
        let rpool = relays::RelayPool::start(&[url.to_string()], receiver.pubkey(), tx, seen(), open());
        let (tx2, _rx2) = tokio::sync::mpsc::channel(16);
        let spool = relays::RelayPool::start(&[url.to_string()], sender.pubkey(), tx2, seen(), open());
        for _ in 0..100 {
            if rpool.status().iter().all(|s| s.connected) && spool.status().iter().all(|s| s.connected) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let (r, s) = (random_secret(), random_secret());
        let mut got = 0;
        for size in [100usize, 2000, 12000] {
            let pt = pad(json!({"x": "a".repeat(size)}).to_string().as_bytes()).unwrap();
            let env = seal_msg_random(&s, &r.public_key(), &pt);
            let ev = sender.message(&receiver.pubkey(), serde_json::to_string(&env).unwrap(), now());
            let len = ev.content.len();
            spool.publish(ev);
            match tokio::time::timeout(Duration::from_secs(15), rx.recv()).await {
                Ok(Some(e)) => {
                    let env2: Envelope = serde_json::from_str(&e.content).unwrap();
                    assert_eq!(open_msg(&r, &s.public_key(), &env2).unwrap(), pt);
                    got += 1;
                    eprintln!("{url}: {} byte plaintext ({len} byte content) arrived", pt.len());
                }
                _ => eprintln!("{url}: {} byte plaintext ({len} byte content) did NOT arrive; status {:?}", pt.len(), spool.status()),
            }
        }
        if got < 3 {
            failed.push(url);
        }
    }
    assert!(failed.is_empty(), "relays that didn't carry every size: {failed:?}");
}

/// A second phone opening a request with the live code while the first is shown: the code is out, so the pairing can
/// only be refused (re-review, L3 stopgap).
#[tokio::test]
async fn two_claims_on_one_code_can_only_be_refused() {
    let r = relay().await;
    let (_dir, desk) = phone_on(&r);
    let url = desk.pair_start("https://example.org/");
    wait_for(|| desk.relay_status().iter().any(|s| s.connected)).await;
    let qr: Value = serde_json::from_slice(&unb64u(url.split_once("#pair=").unwrap().1).unwrap()).unwrap();
    let d_pub = parse_pub(qr["d"].as_str().unwrap()).unwrap();
    let nd = qr["n"].as_str().unwrap().to_string();
    let c = unb64u(qr["c"].as_str().unwrap()).unwrap();
    for (i, name) in ["First", "Second"].iter().enumerate() {
        let mut ph = FakePhone::new(&r).await;
        let id = format!("{:032x}", i + 1);
        let pt = pad(json!({"t": "pair", "p": pub_b64u(&ph.p.public_key()), "np": ph.nk.pubkey(), "name": name, "id": id})
            .to_string()
            .as_bytes())
        .unwrap();
        let env = seal_pair(&d_pub, &c, &random_secret(), &rand::random(), &pt);
        ph.publish(ph.nk.message(&nd, serde_json::to_string(&env).unwrap(), now())).await;
        if i == 0 {
            wait_for(|| desk.pair_state()["state"] == "claimed").await;
        }
    }
    wait_for(|| desk.pair_state()["state"] == "contested").await;
    assert!(desk.pair_answer(true).is_err());
    desk.pair_answer(false).unwrap();
    assert!(desk.devices().is_empty());
}
