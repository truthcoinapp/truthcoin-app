//! Is there a newer release? The app trusts exactly one thing for that: a release's SHA256SUMS signed with this app's
//! release key (`SHA256SUMS.sig`, made with `ssh-keygen -Y sign -n file`; the key is built in from
//! release/truthcoinapp-release.pub). An unsigned or wrongly signed release is never offered. The app then points to
//! the release page; it doesn't replace itself (v0.1.0).

use serde::Serialize;
use serde_json::Value;
use ssh_key::{HashAlg, PublicKey, SshSig};

pub const RELEASE_KEY: &str = env!("TRUTHCOINAPP_RELEASE_KEY");
/// The signature's purpose, made with `ssh-keygen -Y sign -n truthcoinapp-sums`: a file signed with the same key for
/// anything else doesn't pass (review N7).
pub const NAMESPACE: &str = "truthcoinapp-sums";
const DOWNLOADS: &str = "https://github.com/mblowes/truthcoin-app/releases/download/";
const RELEASE_PAGES: &str = "https://github.com/mblowes/truthcoin-app/releases/tag/";
const LATEST: &str = "https://api.github.com/repos/mblowes/truthcoin-app/releases/latest";

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

/// A URL under `prefix`, with nothing that could climb out of it.
fn pinned(u: &str, prefix: &str) -> bool {
    u.starts_with(prefix) && !u.contains("..") && !u.contains('?') && !u.contains('#') && !u.contains('%')
}

/// The version every package in SHA256SUMS names (`…_0.1.1_…`); they must all agree.
pub fn version_in(sums: &str) -> Option<(u32, u32, u32)> {
    let mut found: Option<(u32, u32, u32)> = None;
    for line in sums.lines().filter(|l| !l.trim().is_empty()) {
        let name = line.split_once("  ").map(|x| x.1).unwrap_or(line);
        let v = name.split('_').find_map(parse_version)?;
        if found.is_some_and(|f| f != v) {
            return None;
        }
        found = Some(v);
    }
    found
}

fn parse_version(s: &str) -> Option<(u32, u32, u32)> {
    let mut p = s.trim_start_matches('v').split('.');
    let v = (p.next()?.parse().ok()?, p.next()?.parse().ok()?, p.next()?.parse().ok()?);
    p.next().is_none().then_some(v)
}

#[derive(Serialize)]
pub struct UpdateInfo {
    pub current: String,
    /// A newer signed release, if there is one.
    pub newer: Option<String>,
    pub url: Option<String>,
    /// Why there is no answer (no key in this build, no network, an unsigned release).
    pub note: Option<String>,
}

#[tauri::command]
pub async fn update_check() -> UpdateInfo {
    let current = env!("CARGO_PKG_VERSION").to_string();
    match check().await {
        Ok((newer, url)) => UpdateInfo { current, newer, url, note: None },
        Err(e) => UpdateInfo { current, newer: None, url: None, note: Some(e) },
    }
}

async fn check() -> Result<(Option<String>, Option<String>), String> {
    if RELEASE_KEY.is_empty() {
        return Err("This build has no release key, so it can't check for updates".into());
    }
    let http = reqwest::Client::builder()
        .user_agent(concat!("truthcoin-app/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let rel: Value = http.get(LATEST).send().await.map_err(|_| "GitHub didn't answer")?.json().await.map_err(|e| e.to_string())?;
    let asset = |n: &str| {
        rel["assets"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|a| a["name"] == n)
            .and_then(|a| a["browser_download_url"].as_str().map(String::from))
            .filter(|u| pinned(u, DOWNLOADS))
    };
    let (Some(s), Some(g)) = (asset("SHA256SUMS"), asset("SHA256SUMS.sig")) else {
        return Err("The latest release isn't signed yet".into());
    };
    let sums = http.get(&s).send().await.map_err(|e| e.to_string())?.bytes().await.map_err(|e| e.to_string())?;
    let sig = http.get(&g).send().await.map_err(|e| e.to_string())?.bytes().await.map_err(|e| e.to_string())?;
    verify_with(RELEASE_KEY, &sums, &sig).map_err(|e| format!("The latest release's signature isn't good: {e}"))?;
    let v = version_in(&String::from_utf8_lossy(&sums)).ok_or("The latest release's files don't agree on a version")?;
    let mine = parse_version(env!("CARGO_PKG_VERSION")).unwrap_or((0, 0, 0));
    if v > mine {
        let url = rel["html_url"].as_str().filter(|u| pinned(u, RELEASE_PAGES)).map(String::from);
        Ok((Some(format!("{}.{}.{}", v.0, v.1, v.2)), url))
    } else {
        Ok((None, None))
    }
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

    #[test]
    fn urls_stay_in_the_repository() {
        assert!(pinned("https://github.com/mblowes/truthcoin-app/releases/tag/v0.1.1", RELEASE_PAGES));
        assert!(!pinned("https://github.com/mblowes/truthcoin-app/releases/tag/../../../x/y", RELEASE_PAGES));
        assert!(!pinned("https://github.com/other/repo/releases/download/v1/SHA256SUMS", DOWNLOADS));
    }

    #[test]
    fn versions_from_file_names() {
        assert_eq!(version_in(&String::from_utf8_lossy(SUMS)), Some((0, 1, 1)));
        assert_eq!(version_in("a  x_0.1.1_amd64.deb\nb  y_0.1.2_universal.dmg\n"), None, "files disagree");
        assert!(parse_version("0.10.0").unwrap() > parse_version("0.9.9").unwrap());
    }
}
