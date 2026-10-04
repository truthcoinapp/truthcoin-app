//! The protocol's test vectors (testdata/protocol-v1.json), shared with the phone page's tests. `cargo test
//! write_vectors -- --ignored` writes the file from fixed keys; `vectors_match` checks this code against it.

use super::crypto::*;
use super::nostr::{self, NostrKey};
use p256::SecretKey;
use serde_json::{json, Value};

fn sk(hexs: &str) -> SecretKey {
    SecretKey::from_slice(&hex::decode(hexs).unwrap()).unwrap()
}

const D: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const P: &str = "2222222222222222222222222222222222222222222222222222222222222222";
const E1: &str = "3333333333333333333333333333333333333333333333333333333333333333";
const E2: &str = "4444444444444444444444444444444444444444444444444444444444444444";
const E3: &str = "5555555555555555555555555555555555555555555555555555555555555555";
const NK: &str = "6666666666666666666666666666666666666666666666666666666666666666";

fn build() -> Value {
    let (d, p, e1, e2, e3) = (sk(D), sk(P), sk(E1), sk(E2), sk(E3));
    let c = [0x0cu8; 16];
    let pair_pt = pad(br#"{"t":"pair","p":"PUB","np":"NPUB","name":"Test phone","id":"00112233445566778899aabbccddeeff"}"#)
        .unwrap();
    let pair_env = seal_pair(&d.public_key(), &c, &e1, &[1u8; 12], &pair_pt);
    let req = pad(br#"{"id":"0123456789abcdef0123456789abcdef","ts":1791200000,"m":"status","a":{}}"#).unwrap();
    let req_env = seal_msg(&p, &d.public_key(), &e2, &[2u8; 12], &req);
    let rep = pad(br#"{"re":"0123456789abcdef0123456789abcdef","ok":{"height":42}}"#).unwrap();
    let rep_env = seal_msg(&d, &p.public_key(), &e3, &[3u8; 12], &rep);
    let nk = NostrKey::from_bytes(&hex::decode(NK).unwrap()).unwrap();
    let to = "ab".repeat(32);
    let content = serde_json::to_string(&req_env).unwrap();
    let ev = nk.sign_with(
        1791200000,
        nostr::KIND,
        vec![vec!["p".into(), to.clone()], vec!["expiration".into(), "1791200300".into()]],
        content,
        &[7u8; 32],
    );
    json!({
        "about": "Test vectors for docs/PROTOCOL.md v1. Secret keys are 32-byte P-256 / secp256k1 scalars, hex. \
                  Public keys are b64u uncompressed SEC1 (P-256) or x-only hex (Nostr). Plaintexts are UTF-8, padded \
                  with spaces to 1024, 4096 or 16384 bytes.",
        "keys": {
            "D": {"secret": D, "public": pub_b64u(&d.public_key()), "fingerprint": fingerprint(&d.public_key())},
            "P": {"secret": P, "public": pub_b64u(&p.public_key())},
            "E1": {"secret": E1, "public": pub_b64u(&e1.public_key())},
            "E2": {"secret": E2, "public": pub_b64u(&e2.public_key())},
            "E3": {"secret": E3, "public": pub_b64u(&e3.public_key())}
        },
        "pair": {
            "C": b64u(&c),
            "nonce": b64u(&[1u8; 12]),
            "plaintext": String::from_utf8(pair_pt).unwrap(),
            "envelope": pair_env,
            "comparison_code": pair_code(&d.public_key(), &p.public_key(), &e1.public_key(), &c)
        },
        "request": {"from": "P", "to": "D", "ephemeral": "E2", "nonce": b64u(&[2u8; 12]),
                    "plaintext": String::from_utf8(req).unwrap(), "envelope": req_env},
        "reply": {"from": "D", "to": "P", "ephemeral": "E3", "nonce": b64u(&[3u8; 12]),
                  "plaintext": String::from_utf8(rep).unwrap(), "envelope": rep_env},
        "nostr": {"secret": NK, "pubkey": nk.pubkey(), "aux": hex::encode([7u8; 32]), "event": ev}
    })
}

#[test]
#[ignore]
fn write_vectors() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/protocol-v1.json");
    std::fs::write(path, serde_json::to_string_pretty(&build()).unwrap() + "\n").unwrap();
}

#[test]
fn vectors_match() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/protocol-v1.json");
    let file: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(file, build(), "the code no longer makes testdata/protocol-v1.json");
    // And the file's envelopes open, its event verifies.
    let env: Envelope = serde_json::from_value(file["request"]["envelope"].clone()).unwrap();
    let pt = open_msg(&sk(D), &sk(P).public_key(), &env).unwrap();
    assert_eq!(String::from_utf8(pt).unwrap(), file["request"]["plaintext"].as_str().unwrap());
    let ev: nostr::Event = serde_json::from_value(file["nostr"]["event"].clone()).unwrap();
    assert!(nostr::verify(&ev));
}
