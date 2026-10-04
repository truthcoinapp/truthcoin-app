//! The phone link's sealing (docs/PROTOCOL.md, "Pairing" and "Sealed messages"): P-256 ECDH, HKDF-SHA256,
//! AES-256-GCM and SHA-256, the same primitives the phone page gets from WebCrypto. Every message is sealed on its own,
//! because relays may drop, repeat or reorder it. Every function takes its keys explicitly so the test vectors
//! (testdata/protocol-v1.json) can drive it.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hkdf::Hkdf;
use p256::elliptic_curve::sec1::ToEncodedPoint;
use p256::{PublicKey, SecretKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const PAIR_LABEL: &[u8] = b"tcr-pair-v1";
pub const MSG_LABEL: &[u8] = b"tcr-msg-v1";
pub const SAS_LABEL: &[u8] = b"tcr-pair-sas-v1";
/// Plaintexts are padded with spaces to one of these sizes, so relays can't tell a trade from a balance check.
pub const PAD_SIZES: [usize; 3] = [1024, 4096, 16384];

pub fn b64u(b: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(b)
}

pub fn unb64u(s: &str) -> Result<Vec<u8>, String> {
    URL_SAFE_NO_PAD.decode(s.trim_end_matches('=')).map_err(|_| "bad base64url".to_string())
}

/// Uncompressed SEC1, 65 bytes.
pub fn pub_bytes(p: &PublicKey) -> Vec<u8> {
    p.to_encoded_point(false).as_bytes().to_vec()
}

pub fn pub_b64u(p: &PublicKey) -> String {
    b64u(&pub_bytes(p))
}

/// A public key from b64u uncompressed SEC1. Compressed or off-curve points are refused.
pub fn parse_pub(s: &str) -> Result<PublicKey, String> {
    let b = unb64u(s)?;
    if b.len() != 65 || b[0] != 4 {
        return Err("public key must be 65-byte uncompressed SEC1".into());
    }
    PublicKey::from_sec1_bytes(&b).map_err(|_| "public key is not on P-256".to_string())
}

pub fn random_secret() -> SecretKey {
    SecretKey::random(&mut rand::rngs::OsRng)
}

/// The 32-byte X coordinate of the shared point (WebCrypto deriveBits(256)).
pub fn ecdh(sk: &SecretKey, pk: &PublicKey) -> [u8; 32] {
    let s = p256::ecdh::diffie_hellman(sk.to_nonzero_scalar(), pk.as_affine());
    let mut out = [0u8; 32];
    out.copy_from_slice(s.raw_secret_bytes());
    out
}

pub fn hkdf32(ikm: &[u8], salt: &[u8], info: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 32];
    Hkdf::<Sha256>::new(Some(salt), ikm).expand(info, &mut k).expect("HKDF length is within limits");
    k
}

pub fn sha256(b: &[u8]) -> [u8; 32] {
    Sha256::digest(b).into()
}

fn concat(parts: &[&[u8]]) -> Vec<u8> {
    parts.iter().flat_map(|p| p.iter().copied()).collect()
}

/// JSON padded with spaces to the smallest size that fits (JSON ignores trailing spaces).
pub fn pad(json: &[u8]) -> Result<Vec<u8>, String> {
    let size = PAD_SIZES.iter().find(|s| **s >= json.len()).ok_or("message too big for one envelope")?;
    let mut v = json.to_vec();
    v.resize(*size, b' ');
    Ok(v)
}

/// What travels in an event's content.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Envelope {
    pub v: u32,
    /// "pair" (a phone asking to pair) or "msg".
    pub k: String,
    /// The sender's ephemeral P-256 key, b64u uncompressed SEC1.
    pub e: String,
    /// The 12-byte AES-GCM nonce, b64u.
    pub n: String,
    /// Ciphertext and tag, b64u.
    pub ct: String,
}

fn gcm_seal(key: &[u8; 32], nonce: &[u8; 12], pt: &[u8], aad: &[u8]) -> Vec<u8> {
    Aes256Gcm::new_from_slice(key)
        .expect("32-byte key")
        .encrypt(Nonce::from_slice(nonce), Payload { msg: pt, aad })
        .expect("AES-GCM encrypt")
}

fn gcm_open(key: &[u8; 32], nonce: &[u8; 12], ct: &[u8], aad: &[u8]) -> Result<Vec<u8>, String> {
    Aes256Gcm::new_from_slice(key)
        .expect("32-byte key")
        .decrypt(Nonce::from_slice(nonce), Payload { msg: ct, aad })
        .map_err(|_| "doesn't open".to_string())
}

fn nonce_of(env: &Envelope) -> Result<[u8; 12], String> {
    unb64u(&env.n)?.try_into().map_err(|_| "bad nonce".to_string())
}

/// The key and associated data of a sealed message from S to R with ephemeral E:
/// ikm = ECDH(E, R) || ECDH(S, R); salt = SHA-256(label || S_pub || R_pub || E_pub); k = HKDF(ikm, salt, label);
/// aad = label || S_pub || R_pub || E_pub. Either side computes the two ECDHs from its own secrets.
fn msg_key(dh_er: [u8; 32], dh_sr: [u8; 32], s_pub: &PublicKey, r_pub: &PublicKey, e_pub: &PublicKey) -> ([u8; 32], Vec<u8>) {
    let aad = concat(&[MSG_LABEL, &pub_bytes(s_pub), &pub_bytes(r_pub), &pub_bytes(e_pub)]);
    let salt = sha256(&aad);
    (hkdf32(&concat(&[&dh_er, &dh_sr]), &salt, MSG_LABEL), aad)
}

/// Seal `pt` (already padded) from the static key `s` to `r_pub`, with ephemeral `e` and `nonce` (random in use;
/// fixed in the test vectors).
pub fn seal_msg(s: &SecretKey, r_pub: &PublicKey, e: &SecretKey, nonce: &[u8; 12], pt: &[u8]) -> Envelope {
    let e_pub = e.public_key();
    let (k, aad) = msg_key(ecdh(e, r_pub), ecdh(s, r_pub), &s.public_key(), r_pub, &e_pub);
    Envelope { v: 1, k: "msg".into(), e: pub_b64u(&e_pub), n: b64u(nonce), ct: b64u(&gcm_seal(&k, nonce, pt, &aad)) }
}

pub fn seal_msg_random(s: &SecretKey, r_pub: &PublicKey, pt: &[u8]) -> Envelope {
    seal_msg(s, r_pub, &random_secret(), &rand::random(), pt)
}

/// Open a message to `r` that claims to come from `s_pub`. Anything that doesn't open is an error, to be dropped.
pub fn open_msg(r: &SecretKey, s_pub: &PublicKey, env: &Envelope) -> Result<Vec<u8>, String> {
    if env.v != 1 || env.k != "msg" {
        return Err("not a v1 message".into());
    }
    let e_pub = parse_pub(&env.e)?;
    let (k, aad) = msg_key(ecdh(r, &e_pub), ecdh(r, s_pub), s_pub, &r.public_key(), &e_pub);
    gcm_open(&k, &nonce_of(env)?, &unb64u(&env.ct)?, &aad)
}

/// The pairing key: k = HKDF(ECDH(E, D), salt = C, info = "tcr-pair-v1"); aad = label || D_pub || E_pub.
fn pair_key(dh: [u8; 32], c: &[u8], d_pub: &PublicKey, e_pub: &PublicKey) -> ([u8; 32], Vec<u8>) {
    (hkdf32(&dh, c, PAIR_LABEL), concat(&[PAIR_LABEL, &pub_bytes(d_pub), &pub_bytes(e_pub)]))
}

/// The phone's side of pairing (here for tests; the phone page does this in WebCrypto).
#[cfg(test)]
pub fn seal_pair(d_pub: &PublicKey, c: &[u8], e: &SecretKey, nonce: &[u8; 12], pt: &[u8]) -> Envelope {
    let e_pub = e.public_key();
    let (k, aad) = pair_key(ecdh(e, d_pub), c, d_pub, &e_pub);
    Envelope { v: 1, k: "pair".into(), e: pub_b64u(&e_pub), n: b64u(nonce), ct: b64u(&gcm_seal(&k, nonce, pt, &aad)) }
}

/// The desktop opens a pairing request with its key `d` and the live code `c`. Returns the plaintext and E_pub.
pub fn open_pair(d: &SecretKey, c: &[u8], env: &Envelope) -> Result<(Vec<u8>, PublicKey), String> {
    if env.v != 1 || env.k != "pair" {
        return Err("not a v1 pairing request".into());
    }
    let e_pub = parse_pub(&env.e)?;
    let (k, aad) = pair_key(ecdh(d, &e_pub), c, &d.public_key(), &e_pub);
    Ok((gcm_open(&k, &nonce_of(env)?, &unb64u(&env.ct)?, &aad)?, e_pub))
}

/// The comparison code both screens show at pairing: SHA-256("tcr-pair-sas-v1" || D_pub || P_pub || E_pub || C), the
/// first 4 bytes big-endian, modulo 1,000,000, as 6 digits in two groups of three ("042 917").
pub fn pair_code(d_pub: &PublicKey, p_pub: &PublicKey, e_pub: &PublicKey, c: &[u8]) -> String {
    let h = sha256(&concat(&[SAS_LABEL, &pub_bytes(d_pub), &pub_bytes(p_pub), &pub_bytes(e_pub), c]));
    let n = u32::from_be_bytes([h[0], h[1], h[2], h[3]]) % 1_000_000;
    let s = format!("{n:06}");
    format!("{} {}", &s[..3], &s[3..])
}

/// A key's short fingerprint, shown on both screens: the first 6 bytes of SHA-256 of its 65-byte public key, hex, in
/// three groups ("1a2b 3c4d 5e6f").
pub fn fingerprint(p: &PublicKey) -> String {
    let h = hex::encode(&sha256(&pub_bytes(p))[..6]);
    format!("{} {} {}", &h[..4], &h[4..8], &h[8..])
}

/// A device name as the desktop shows it: at most 40 characters, with control and format characters (bidi
/// overrides, zero-width) taken out.
pub fn clean_name(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .filter(|c| !c.is_control())
        .filter(|c| !matches!(*c as u32, 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x2069 | 0xFEFF | 0x00AD | 0x061C))
        .take(40)
        .collect();
    let t = cleaned.trim();
    if t.is_empty() { "Phone".into() } else { t.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_opens_only_for_its_receiver_and_from_its_sender() {
        let (s, r, x) = (random_secret(), random_secret(), random_secret());
        let pt = pad(br#"{"id":"1"}"#).unwrap();
        let env = seal_msg_random(&s, &r.public_key(), &pt);
        assert_eq!(open_msg(&r, &s.public_key(), &env).unwrap(), pt);
        assert!(open_msg(&x, &s.public_key(), &env).is_err(), "another receiver");
        assert!(open_msg(&r, &x.public_key(), &env).is_err(), "another claimed sender");
        let mut bad = env.clone();
        bad.ct = b64u(&{
            let mut c = unb64u(&env.ct).unwrap();
            c[0] ^= 1;
            c
        });
        assert!(open_msg(&r, &s.public_key(), &bad).is_err(), "changed ciphertext");
    }

    #[test]
    fn pairing_opens_only_with_the_live_code() {
        let d = random_secret();
        let c = [7u8; 16];
        let env = seal_pair(&d.public_key(), &c, &random_secret(), &[1; 12], b"hi");
        assert_eq!(open_pair(&d, &c, &env).unwrap().0, b"hi");
        assert!(open_pair(&d, &[8u8; 16], &env).is_err());
    }

    #[test]
    fn padding_hides_sizes() {
        assert_eq!(pad(b"{}").unwrap().len(), 1024);
        assert_eq!(pad(&[b'x'; 1025]).unwrap().len(), 4096);
        assert!(pad(&[b'x'; 16385]).is_err());
        let v: serde_json::Value = serde_json::from_slice(&pad(br#"{"a":1}"#).unwrap()).unwrap();
        assert_eq!(v["a"], 1);
    }

    #[test]
    fn names_are_cleaned() {
        assert_eq!(clean_name("  Mi\u{202E}ke's\u{200B} phone\n"), "Mike's phone");
        assert_eq!(clean_name(&"x".repeat(50)).len(), 40);
        assert_eq!(clean_name("\u{200B}"), "Phone");
    }
}
