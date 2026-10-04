//! The app's own record of every trade it placed (`<app data>/trades.json`, owner-only). The node's position figures
//! (cost basis, labels) aren't reliable, so what was paid comes from here. A trade is written down *before* it goes
//! to the node ("sending"), so a crash between the two never loses one: a "sending" trade with no txid may have gone
//! out, and counts against a phone's limit.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Buy,
    Sell,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Written down, about to go to the node (or it went and the app stopped before hearing back).
    Sending,
    /// In the node's mempool, waiting for a block.
    Pending,
    /// In a block.
    Done,
    /// The node refused it.
    Failed,
    /// Taken out of the mempool by the user (a stuck trade); its coin is back.
    Cancelled,
    /// No longer in the mempool and not in a block (dropped by the node).
    Dropped,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Trade {
    /// The app's id for the trade (a phone's request id, or a random one).
    pub id: String,
    pub time: u64,
    pub market_id: String,
    pub market_title: String,
    pub outcome: u32,
    pub outcome_label: String,
    pub side: Side,
    pub shares: u64,
    /// The quote shown: cost (buy) or net proceeds (sell), in sats, fees included.
    pub quoted_sats: u64,
    /// The trade's cap: at most this (buy), at least this (sell).
    pub limit_sats: u64,
    pub txid: Option<String>,
    pub status: Status,
    /// "desktop" or "phone:<name>".
    pub source: String,
    /// The Truthcoin height when it was sent.
    pub height: u64,
    pub error: Option<String>,
    /// What it counts against a phone's daily limit: a buy, its cap; a sell, the shares' value at the price when it was
    /// placed (security review M1).
    #[serde(default)]
    pub charge_sats: u64,
}

pub struct Trades {
    path: PathBuf,
    list: Mutex<Vec<Trade>>,
    /// The file couldn't be read at start: it was moved aside, and phones may not trade until the app restarts with a
    /// readable one (security review L10). Its new name.
    pub unreadable: Option<PathBuf>,
}

/// Read `path` as JSON. A file that exists but doesn't parse is moved aside (`<name>.bad-<time>`), never overwritten;
/// its new name comes back with the default value. If it can't be moved, its own name comes back: the caller must
/// then not save over it (re-review R4).
pub fn load_or_set_aside<T: serde::de::DeserializeOwned + Default>(path: &Path) -> (T, Option<PathBuf>) {
    match crate::files::read_json::<T>(path) {
        Ok(Some(v)) => (v, None),
        Ok(None) => (T::default(), None),
        Err(_) => {
            let aside = path.with_extension(format!("bad-{}", crate::activity::unix_now()));
            match std::fs::rename(path, &aside) {
                Ok(()) => (T::default(), Some(aside)),
                Err(_) => (T::default(), Some(path.to_path_buf())),
            }
        }
    }
}

/// Records set aside in `dir` (`*.bad-*`) that the user hasn't looked at yet.
pub fn set_aside_in(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.contains(".bad-")))
        .collect()
}

/// The user has looked at the set-aside records: they are renamed `*.seen-*` and stop blocking phones.
pub fn mark_seen(dir: &Path) {
    for p in set_aside_in(dir) {
        let n = p.file_name().and_then(|n| n.to_str()).unwrap_or("").replace(".bad-", ".seen-");
        let _ = std::fs::rename(&p, p.with_file_name(n));
    }
}

impl Trades {
    pub fn load(dir: &Path) -> Trades {
        let path = dir.join("trades.json");
        let (list, unreadable) = load_or_set_aside(&path);
        if let Some(a) = &unreadable {
            crate::activity::note(dir, &format!("trades.json couldn't be read; moved aside to {}", a.display()));
        }
        Trades { path, list: Mutex::new(list), unreadable }
    }

    fn save(&self, list: &[Trade]) -> Result<(), String> {
        if self.unreadable.as_deref() == Some(self.path.as_path()) {
            return Err("the trade log couldn't be read or set aside, so it isn't written over".into());
        }
        crate::files::write_json(&self.path, &list).map_err(|e| format!("can't write the trade log: {e}"))
    }

    /// Add a trade; it is on disk before this returns.
    pub fn add(&self, t: Trade) -> Result<(), String> {
        let mut l = self.list.lock().unwrap();
        l.push(t);
        self.save(&l)
    }

    pub fn update(&self, id: &str, f: impl FnOnce(&mut Trade)) -> Result<(), String> {
        let mut l = self.list.lock().unwrap();
        if let Some(t) = l.iter_mut().rev().find(|t| t.id == id) {
            f(t);
        }
        self.save(&l)
    }

    pub fn all(&self) -> Vec<Trade> {
        self.list.lock().unwrap().clone()
    }

    pub fn get(&self, id: &str) -> Option<Trade> {
        self.list.lock().unwrap().iter().rev().find(|t| t.id == id).cloned()
    }

    /// What trades from `source` since `since` (unix seconds) count against its limit: every trade the node may have
    /// taken, at its charge (a buy's cap; a sell's value given up). A cancelled trade still counts: other nodes may
    /// have had it before it left this one. Only a trade the node refused doesn't.
    pub fn spent_since(&self, source: &str, since: u64) -> u64 {
        self.list
            .lock()
            .unwrap()
            .iter()
            .filter(|t| t.source == source && t.time >= since)
            .filter(|t| t.status != Status::Failed)
            .map(|t| if t.charge_sats > 0 || t.side == Side::Sell { t.charge_sats } else { t.limit_sats })
            .fold(0u64, |a, b| a.saturating_add(b))
    }

    /// Trades from `source` still waiting for their block. A "sending" trade that never got a transaction id stops
    /// counting after 5 minutes: it is in no mempool this node knows (it still counts against the limit; re-review R1).
    pub fn waiting(&self, source: &str) -> usize {
        let now = crate::activity::unix_now();
        self.list
            .lock()
            .unwrap()
            .iter()
            .filter(|t| t.source == source)
            .filter(|t| match t.status {
                Status::Pending => true,
                Status::Sending => t.txid.is_some() || t.time + 300 > now,
                _ => false,
            })
            .count()
    }

    /// The user's word that a "sending" trade with no transaction didn't go through: it leaves the waiting list but
    /// still counts against a phone's limit (it may have gone, so it is marked dropped, not failed).
    pub fn clear(&self, id: &str) -> Result<(), String> {
        let t = self.get(id).ok_or("no such trade")?;
        if t.status != Status::Sending || t.txid.is_some() {
            return Err("Only a trade that never got an answer can be cleared".into());
        }
        if t.time + 120 > crate::activity::unix_now() {
            return Err("Give it two minutes first".into());
        }
        self.update(id, |t| t.status = Status::Dropped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(id: &str, source: &str, side: Side, status: Status, limit: u64) -> Trade {
        let charge = if side_is_buy(&side) { limit } else { limit * 2 };
        Trade {
            id: id.into(),
            time: 100,
            market_id: "m".into(),
            market_title: "M".into(),
            outcome: 1,
            outcome_label: "Yes".into(),
            side,
            shares: 10,
            quoted_sats: limit - 1,
            limit_sats: limit,
            txid: None,
            status,
            source: source.into(),
            height: 1,
            error: None,
            charge_sats: charge,
        }
    }

    fn side_is_buy(s: &Side) -> bool {
        *s == Side::Buy
    }

    #[test]
    fn spending_counts_every_buy_that_may_have_gone_out() {
        let d = tempfile::tempdir().unwrap();
        let tr = Trades::load(d.path());
        tr.add(t("a", "phone:x", Side::Buy, Status::Sending, 1000)).unwrap();
        tr.add(t("b", "phone:x", Side::Buy, Status::Pending, 2000)).unwrap();
        tr.add(t("c", "phone:x", Side::Buy, Status::Failed, 4000)).unwrap();
        tr.add(t("d", "phone:x", Side::Sell, Status::Done, 8000)).unwrap();
        tr.add(t("e", "desktop", Side::Buy, Status::Done, 16000)).unwrap();
        tr.add(t("f", "phone:x", Side::Buy, Status::Cancelled, 32000)).unwrap();
        // Buys at their caps, the sell at its charge (twice its limit here), the refused buy not at all.
        assert_eq!(tr.spent_since("phone:x", 0), 1000 + 2000 + 16000 + 32000);
        assert_eq!(tr.spent_since("phone:x", 101), 0);
        // On disk, and read back.
        assert_eq!(Trades::load(d.path()).all().len(), 6);
    }

    #[test]
    fn a_trade_that_never_got_an_answer_stops_waiting() {
        let d = tempfile::tempdir().unwrap();
        let tr = Trades::load(d.path());
        let mut old = t("o", "phone:x", Side::Buy, Status::Sending, 1000);
        old.time = crate::activity::unix_now() - 600;
        let mut fresh = t("f", "phone:x", Side::Buy, Status::Sending, 1000);
        fresh.time = crate::activity::unix_now();
        tr.add(old).unwrap();
        tr.add(fresh).unwrap();
        assert_eq!(tr.waiting("phone:x"), 1, "only the recent one may still be on its way");
        assert!(tr.clear("f").is_err(), "too soon to clear");
        tr.clear("o").unwrap();
        assert_eq!(tr.get("o").unwrap().status, Status::Dropped);
        assert_eq!(tr.spent_since("phone:x", 0), 2000, "a cleared trade still counts against the limit");
    }

    #[test]
    fn an_unreadable_record_is_set_aside_not_overwritten() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("trades.json"), b"{not json").unwrap();
        let tr = Trades::load(d.path());
        assert!(tr.all().is_empty());
        let aside = tr.unreadable.clone().expect("set aside");
        assert_eq!(std::fs::read(&aside).unwrap(), b"{not json");
        assert_eq!(set_aside_in(d.path()).len(), 1, "it blocks phones until looked at");
        mark_seen(d.path());
        assert!(set_aside_in(d.path()).is_empty());
    }
}
