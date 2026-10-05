//! Settings, kept in `<app data>/settings.json` (owner-only). Everything has a default, so a missing or older file
//! still loads. The node settings are for developers and unusual setups: the screens show them under Advanced.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The relays a new install uses for the phone link (docs/PROTOCOL.md). Editable in Settings › Phone.
pub const DEFAULT_RELAYS: [&str; 3] = ["wss://relay.damus.io", "wss://nos.lol", "wss://relay.primal.net"];

/// A release build runs only on eCash beta: the Truthcoin network is betanet, the node is the pinned release with no
/// extra arguments, and the enforcer must be on eCash beta's chain (`enforcer::check_ecash_beta`). Development builds
/// (debug) may use the dev chain (regtest), a node program of their own and extra arguments.
pub const BETA_ONLY: bool = !cfg!(debug_assertions);

/// The phone page, built from this repository's tagged source by GitHub Pages.
pub const DEFAULT_PHONE_PAGE: &str = "https://truthcoinapp.github.io/truthcoin-app/";

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Settings {
    /// The Truthcoin network: "betanet" (eCash beta, the default), "forknet", "signet" or "regtest".
    pub network: String,
    /// The enforcer's gRPC (BitWindow starts it on 127.0.0.1:50051).
    pub enforcer: String,
    /// The node's read-only RPC. Its wallet calls go on a private port picked at each start.
    pub rpc_port: u16,
    /// The node's peer-to-peer address (QUIC). Peers on the network reach it here.
    pub p2p_addr: String,
    pub zmq_port: u16,
    /// A node program of your own, used instead of the release the app downloads and checks (developers).
    pub node_binary: Option<PathBuf>,
    /// Extra arguments for the node (developers), e.g. ["--decision-config-testing", "10"].
    pub node_args: Vec<String>,
    /// Phone link relays (wss:// only, at most 5).
    pub relays: Vec<String>,
    /// What a newly paired phone may spend on trades in a day, in sats, before the desktop must confirm.
    pub phone_daily_limit_sats: u64,
    /// Where the phone page is served from (the pairing QR code opens it).
    pub phone_page: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            network: "betanet".into(),
            enforcer: "127.0.0.1:50051".into(),
            rpc_port: 16013,
            p2p_addr: "0.0.0.0:14013".into(),
            zmq_port: 16015,
            node_binary: None,
            node_args: vec![],
            relays: DEFAULT_RELAYS.iter().map(|s| s.to_string()).collect(),
            phone_daily_limit_sats: 100_000,
            phone_page: DEFAULT_PHONE_PAGE.into(),
        }
    }
}

impl Settings {
    pub fn path(dir: &Path) -> PathBuf {
        dir.join("settings.json")
    }

    pub fn load(dir: &Path) -> Settings {
        let s: Settings = crate::files::read_json(&Self::path(dir)).ok().flatten().unwrap_or_default();
        s.locked()
    }

    /// In a release build, the settings eCash beta requires, whatever the file says.
    pub fn locked(self) -> Settings {
        self.lock(BETA_ONLY)
    }

    fn lock(mut self, beta_only: bool) -> Settings {
        if beta_only {
            self.network = "betanet".into();
            self.node_binary = None;
            self.node_args = vec![];
        }
        self
    }

    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        crate::files::write_json(&Self::path(dir), self)
    }

    /// The enforcer's host and port.
    pub fn enforcer_host_port(&self) -> Result<(String, u16), String> {
        enforcer_host_port(&self.enforcer)
    }
}

/// An enforcer address as typed, made plain: `host:port`, with `http://` and a trailing `/` taken off. Its gRPC has
/// no TLS, so `https://` is refused rather than quietly sent in the clear.
pub fn clean_enforcer(typed: &str) -> Result<String, String> {
    let mut a = typed.trim();
    let starts = |a: &str, p: &str| a.get(..p.len()).is_some_and(|x| x.eq_ignore_ascii_case(p));
    if starts(a, "https://") {
        return Err("The enforcer has no encrypted (https) address: type its host and port, like 192.168.1.20:50051".into());
    }
    if starts(a, "http://") {
        a = &a[7..];
    }
    let a = a.trim_end_matches('/');
    if a.is_empty() || a.len() > 100 || !a.chars().all(|c| c.is_ascii_alphanumeric() || ".-:[]".contains(c)) {
        return Err("Type the enforcer's host and port, like 192.168.1.20:50051".into());
    }
    let (h, p) = enforcer_host_port(a)?;
    if h.is_empty() || p == 0 {
        return Err("The enforcer's address needs a host, like 192.168.1.20:50051".into());
    }
    Ok(a.to_string())
}

fn enforcer_host_port(enforcer: &str) -> Result<(String, u16), String> {
    let (h, p) = enforcer.rsplit_once(':').ok_or("the enforcer's address needs a port, like 127.0.0.1:50051")?;
    let p: u16 = p.parse().map_err(|_| "the enforcer's port isn't a number")?;
    Ok((h.trim_matches(|c| c == '[' || c == ']').to_string(), p))
}


#[cfg(test)]
mod tests {
    #[test]
    fn enforcer_addresses_are_made_plain() {
        use super::clean_enforcer as c;
        assert_eq!(c(" 192.168.1.20:50051 ").unwrap(), "192.168.1.20:50051");
        assert_eq!(c("http://192.168.1.20:50051/").unwrap(), "192.168.1.20:50051");
        assert_eq!(c("HTTP://box.local:50051").unwrap(), "box.local:50051");
        assert_eq!(c("[fd7a:115c::1]:50051").unwrap(), "[fd7a:115c::1]:50051");
        assert!(c("https://192.168.1.20:50051").unwrap_err().contains("https"));
        for bad in ["", "192.168.1.20", "192.168.1.20:", "192.168.1.20:port", ":50051", "a b:50051", "x/y:50051", "1.2.3.4:99999", "1.2.3.4:0", "éééé:50051"] {
            assert!(c(bad).is_err(), "{bad:?} should be refused");
        }
    }

    #[test]
    fn a_release_build_ignores_dev_settings() {
        let s = super::Settings {
            network: "regtest".into(),
            node_binary: Some("/tmp/other".into()),
            node_args: vec!["--network".into(), "regtest".into()],
            ..Default::default()
        };
        let l = s.clone().lock(true);
        assert_eq!(l.network, "betanet");
        assert!(l.node_binary.is_none() && l.node_args.is_empty());
        assert_eq!(s.lock(false).network, "regtest", "development builds keep them");
    }
}
