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
        let (h, p) = self.enforcer.rsplit_once(':').ok_or("the enforcer's address needs a port, like 127.0.0.1:50051")?;
        let p: u16 = p.parse().map_err(|_| "the enforcer's port isn't a number")?;
        Ok((h.trim_matches(|c| c == '[' || c == ']').to_string(), p))
    }
}


#[cfg(test)]
mod tests {
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
