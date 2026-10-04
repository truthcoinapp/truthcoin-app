//! Settings › Phone: pairing, paired phones and their limits, held trades, relays.

use crate::state::St;
use serde::Serialize;
use serde_json::Value;

#[derive(Serialize)]
pub struct PhoneDevice {
    pub name: String,
    pub np: String,
    pub limit_sats: u64,
    pub left_sats: u64,
    pub paired_at: u64,
    pub last_seen: u64,
}

#[derive(Serialize)]
pub struct PhoneInfo {
    pub devices: Vec<PhoneDevice>,
    pub held: Vec<super::Held>,
    pub relays: Vec<String>,
    pub relay_status: Vec<super::relays::RelayStatus>,
    pub page: String,
    pub default_limit_sats: u64,
    /// This computer's key, as a paired phone shows it in its Settings.
    pub fingerprint: String,
    /// Why phones can't trade in this run, if they can't.
    pub blocked: Option<String>,
}

#[tauri::command]
pub fn phone_info(st: St<'_>) -> PhoneInfo {
    let p = &st.phone;
    let s = st.node.settings();
    PhoneInfo {
        devices: p
            .devices()
            .iter()
            .map(|d| PhoneDevice {
                name: d.name.clone(),
                np: d.np.clone(),
                limit_sats: d.limit_sats,
                left_sats: p.left_today(d),
                paired_at: d.paired_at,
                last_seen: d.last_seen,
            })
            .collect(),
        held: p.held(),
        relays: s.relays,
        relay_status: p.relay_status(),
        page: s.phone_page,
        default_limit_sats: s.phone_daily_limit_sats,
        fingerprint: super::crypto::fingerprint(&p.d_pub()),
        blocked: p.blocked(),
    }
}

/// Start pairing: the URL for the QR code.
#[tauri::command]
pub fn phone_pair_start(st: St<'_>) -> String {
    let page = st.node.settings().phone_page;
    st.phone.pair_start(&page)
}

#[tauri::command]
pub fn phone_pair_state(st: St<'_>) -> Value {
    st.phone.pair_state()
}

#[tauri::command]
pub fn phone_pair_answer(st: St<'_>, allow: bool) -> Result<(), String> {
    st.phone.pair_answer(allow)
}

#[tauri::command]
pub fn phone_pair_cancel(st: St<'_>) {
    st.phone.pair_cancel()
}

/// The user has looked at the records set aside as unreadable: phones may trade again.
#[tauri::command]
pub fn phone_records_seen(st: St<'_>) {
    st.phone.records_seen()
}

#[tauri::command]
pub fn phone_revoke(st: St<'_>, np: String) -> Result<(), String> {
    st.phone.revoke(&np)?;
    crate::activity::note(&st.dir, "phone removed");
    Ok(())
}

#[tauri::command]
pub fn phone_set_limit(st: St<'_>, np: String, limit_sats: u64) -> Result<(), String> {
    if limit_sats > 100_000_000_000 {
        return Err("That limit is too big".into());
    }
    st.phone.set_limit(&np, limit_sats)
}

#[tauri::command]
pub async fn phone_held_answer(st: St<'_>, id: String, approve: bool) -> Result<Value, String> {
    st.phone.held_answer(&id, approve).await
}

/// Change the relays (wss:// only, or ws:// on this computer; 1 to 5) and where the phone page loads from.
#[tauri::command]
pub fn phone_set_relays(st: St<'_>, relays: Vec<String>, page: String) -> Result<(), String> {
    let relays: Vec<String> = relays.into_iter().map(|r| r.trim().to_string()).filter(|r| !r.is_empty()).collect();
    if relays.is_empty() || relays.len() > 5 {
        return Err("Give between 1 and 5 relays".into());
    }
    if let Some(bad) = relays.iter().find(|r| !super::relays::usable(r) || r.len() > 200) {
        return Err(format!("{bad} isn't a relay address (wss://…)"));
    }
    let page = page.trim().to_string();
    let u = url::Url::parse(&page).map_err(|_| "The phone page needs a full address")?;
    let local = matches!(u.host_str(), Some("127.0.0.1") | Some("localhost"));
    if u.scheme() != "https" && !(u.scheme() == "http" && local) {
        return Err("The phone page must be https://".into());
    }
    {
        let mut s = st.node.settings.lock().unwrap();
        let mut n = s.clone();
        n.relays = relays;
        n.phone_page = page;
        n.save(&st.dir).map_err(|e| e.to_string())?;
        *s = n;
    }
    st.phone.restart();
    Ok(())
}
