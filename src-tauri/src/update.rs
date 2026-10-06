//! The release key and its signature check. The app trusts exactly one thing for an update: a release's SHA256SUMS
//! signed with this app's release key (`SHA256SUMS.sig`, made with `ssh-keygen -Y sign -n truthcoinapp-sums`; the key is
//! built in from release/truthcoinapp-release.pub). An unsigned or wrongly signed release is never offered.
//! `app_update.rs` checks for releases and puts them in place.

use ssh_key::{HashAlg, PublicKey, SshSig};

pub const RELEASE_KEY: &str = env!("TRUTHCOINAPP_RELEASE_KEY");
/// The signature's purpose, made with `ssh-keygen -Y sign -n truthcoinapp-sums`: a file signed with the same key for
/// anything else doesn't pass (review N7).
pub const NAMESPACE: &str = "truthcoinapp-sums";

pub fn verify_with(key: &str, sums: &[u8], sig: &[u8]) -> Result<(), &'static str> {
    let key = PublicKey::from_openssh(key).map_err(|_| "the release key doesn't parse")?;
    let sig = SshSig::from_pem(sig).map_err(|_| "the signature file can't be read")?;
    if sig.public_key() != key.key_data() {
        return Err("it was signed by a different key");
    }
    if sig.namespace() != NAMESPACE {
        return Err("it was signed for another use");
    }
    if !matches!(sig.hash_alg(), HashAlg::Sha512 | HashAlg::Sha256) {
        return Err("it uses a hash this app doesn't know");
    }
    key.verify(NAMESPACE, sums, &sig).map_err(|_| "the signature doesn't match the checksums")
}

#[cfg(test)]
mod tests {
    use super::*;
    const SUMS: &[u8] = include_bytes!("../testdata/update/SHA256SUMS");
    const SIG: &[u8] = include_bytes!("../testdata/update/SHA256SUMS.sig");
    const SIG_FILE: &[u8] = include_bytes!("../testdata/update/SHA256SUMS.file.sig");
    // A throwaway key (its private half was deleted after signing).
    const KEY: &str = include_str!("../testdata/update/throwaway.pub");

    #[test]
    fn a_good_signature_passes_and_anything_else_doesnt() {
        assert_eq!(verify_with(KEY.trim(), SUMS, SIG), Ok(()));
        let mut changed = SUMS.to_vec();
        changed[0] ^= 1;
        assert!(verify_with(KEY.trim(), &changed, SIG).is_err());
        assert_eq!(verify_with(KEY.trim(), SUMS, SIG_FILE), Err("it was signed for another use"), "a plain -n file signature");
        let other = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAi2C9Lpi3gHPva6tlbLE+wdF1Cer3uUnmwZYr6SeRjR";
        assert_eq!(verify_with(other, SUMS, SIG), Err("it was signed by a different key"));
    }
}
