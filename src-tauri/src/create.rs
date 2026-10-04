//! Creating a market (desktop only). A market asks one question (a decision) that voters will answer in a later
//! period. The decision is claimed in a slot of that period in the same transaction. The creator pays the market's
//! liquidity (what makes its prices move smoothly; `beta * ln(outcomes)`), the slot's listing fee and the transaction
//! fee, and earns the trading fees.

use crate::state::St;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Serialize)]
pub struct CreateInfo {
    pub current_period: u64,
    pub current_period_name: String,
    pub blocks_per_period: Option<u64>,
    pub testing: bool,
    /// Per open period: its index and the cheapest listing fee.
    pub periods: Vec<Value>,
}

#[tauri::command]
pub async fn create_info(st: St<'_>) -> Result<CreateInfo, String> {
    let rpc = st.node.rpc_or_err()?;
    let s: Value = rpc.public("decision_status", json!([])).await?;
    let periods: Vec<Value> = rpc.public("list_open_periods_with_pricing", json!([])).await?;
    Ok(CreateInfo {
        current_period: s["current_period"].as_u64().unwrap_or(0),
        current_period_name: s["current_period_name"].as_str().unwrap_or("").into(),
        blocks_per_period: s["blocks_per_period"].as_u64(),
        testing: s["is_testing_mode"].as_bool().unwrap_or(false),
        periods,
    })
}

#[derive(Deserialize, Clone, Debug)]
pub struct NewMarket {
    pub title: String,
    pub description: String,
    /// "binary", "scaled" or "category".
    pub kind: String,
    /// The question voters will answer, short.
    pub question: String,
    /// How it will be decided.
    pub rules: String,
    pub period: u32,
    /// binary: the two answers.
    pub no_label: Option<String>,
    pub yes_label: Option<String>,
    /// category: the options.
    pub options: Option<Vec<String>>,
    /// scaled: the range and step.
    pub min: Option<i64>,
    pub max: Option<i64>,
    pub increment: Option<i64>,
    pub beta: f64,
    pub trading_fee: f64,
    pub tags: Vec<String>,
}

#[derive(Serialize)]
pub struct CreateCost {
    pub liquidity_sats: u64,
    pub listing_fee_sats: u64,
    pub tx_fee_sats: u64,
    pub total_sats: u64,
}

pub const TX_FEE: u64 = 1_000;

fn clean(s: &str, max: usize) -> String {
    s.chars().filter(|c| !c.is_control() || *c == '\n').take(max).collect::<String>().trim().to_string()
}

fn dimension(m: &NewMarket) -> Result<Value, String> {
    let header = clean(&m.question, 200);
    if header.is_empty() {
        return Err("Write the question".into());
    }
    let mut d = json!({"type": "new", "period_index": m.period, "decision_type": m.kind, "header": header,
                       "description": clean(&m.rules, 2000), "tags": m.tags.iter().map(|t| clean(t, 40)).collect::<Vec<_>>()});
    match m.kind.as_str() {
        "binary" => {
            d["option_0_label"] = json!(clean(m.no_label.as_deref().unwrap_or("No"), 60));
            d["option_1_label"] = json!(clean(m.yes_label.as_deref().unwrap_or("Yes"), 60));
        }
        "category" => {
            let o: Vec<String> = m.options.clone().unwrap_or_default().iter().map(|o| clean(o, 60)).filter(|o| !o.is_empty()).collect();
            if o.len() < 2 || o.len() > 16 {
                return Err("Give between 2 and 16 options".into());
            }
            d["option_labels"] = json!(o);
        }
        "scaled" => {
            let (min, max, inc) = (m.min.unwrap_or(0), m.max.unwrap_or(0), m.increment.unwrap_or(1));
            if max <= min || inc <= 0 || (max - min) % inc != 0 {
                return Err("The range needs a minimum below the maximum and a step that divides it".into());
            }
            d["min"] = json!(min);
            d["max"] = json!(max);
            d["increment"] = json!(inc);
        }
        _ => return Err("Pick a kind of question".into()),
    }
    Ok(d)
}

fn outcomes(m: &NewMarket) -> u64 {
    match m.kind.as_str() {
        "category" => m.options.as_ref().map(|o| o.len() as u64).unwrap_or(2),
        _ => 2,
    }
}

/// Every cost of a new market, before it is made.
#[tauri::command]
pub async fn create_cost(st: St<'_>, market: NewMarket) -> Result<CreateCost, String> {
    let rpc = st.node.rpc_or_err()?;
    cost(&rpc, &market).await
}

async fn cost(rpc: &crate::rpc::Rpc, m: &NewMarket) -> Result<CreateCost, String> {
    if !(m.beta >= 1000.0 && m.beta <= 1e12) {
        return Err("Pick the market's depth".into());
    }
    let liq: Value =
        rpc.public("calculate_initial_liquidity", json!([{"beta": m.beta, "num_outcomes": outcomes(m)}])).await?;
    let liquidity_sats = liq
        .as_u64()
        .or_else(|| liq["initial_liquidity_sats"].as_u64())
        .or_else(|| liq["liquidity_sats"].as_u64())
        .or_else(|| liq.as_object().and_then(|o| o.values().find_map(|v| v.as_u64())))
        .ok_or("the node didn't say what the liquidity costs")?;
    let periods: Vec<Value> = rpc.public("list_open_periods_with_pricing", json!([])).await?;
    let p = periods
        .iter()
        .find(|p| p["period_index"].as_u64() == Some(m.period as u64))
        .ok_or("That period isn't open for new questions")?;
    let listing_fee_sats = p["cheapest_available_slot_sats"].as_u64().ok_or("no free slot in that period")?;
    Ok(CreateCost { liquidity_sats, listing_fee_sats, tx_fee_sats: TX_FEE, total_sats: liquidity_sats + listing_fee_sats + TX_FEE })
}

/// Make the market, paying at most the listing fee just shown.
#[tauri::command]
pub async fn create_market(st: St<'_>, market: NewMarket, max_listing_fee_sats: u64) -> Result<Value, String> {
    let rpc = st.node.rpc_or_err()?;
    let r = make(&rpc, &market, max_listing_fee_sats).await?;
    crate::activity::note(&st.dir, &format!("created market {}", r["market_id"].as_str().unwrap_or("?")));
    Ok(r)
}

pub async fn make(rpc: &crate::rpc::Rpc, market: &NewMarket, max_listing_fee_sats: u64) -> Result<Value, String> {
    let title = clean(&market.title, 200);
    if title.is_empty() {
        return Err("Give the market a title".into());
    }
    if !(0.0..=0.1).contains(&market.trading_fee) {
        return Err("The trading fee must be between 0% and 10%".into());
    }
    let c = cost(rpc, market).await?;
    if c.listing_fee_sats > max_listing_fee_sats {
        return Err(format!("The listing fee rose to {} sats; check the costs again", c.listing_fee_sats));
    }
    let req = json!({"title": title, "description": clean(&market.description, 4000), "dimensions": [dimension(market)?],
                     "beta": market.beta, "trading_fee": market.trading_fee, "tx_fee_sats": TX_FEE,
                     "max_listing_fee_sats": max_listing_fee_sats});
    Ok(rpc.private("market_create", json!([req])).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(kind: &str) -> NewMarket {
        NewMarket {
            title: "T".into(),
            description: "D".into(),
            kind: kind.into(),
            question: "Q?".into(),
            rules: "R".into(),
            period: 3,
            no_label: None,
            yes_label: None,
            options: Some(vec!["a".into(), "b".into(), "c".into()]),
            min: Some(0),
            max: Some(100),
            increment: Some(5),
            beta: 1e6,
            trading_fee: 0.01,
            tags: vec![],
        }
    }

    #[test]
    fn dimensions_by_kind() {
        let b = dimension(&m("binary")).unwrap();
        assert_eq!(b["option_1_label"], "Yes");
        let c = dimension(&m("category")).unwrap();
        assert_eq!(c["option_labels"].as_array().unwrap().len(), 3);
        assert_eq!(outcomes(&m("category")), 3);
        let s = dimension(&m("scaled")).unwrap();
        assert_eq!(s["increment"], 5);
        let mut bad = m("scaled");
        bad.increment = Some(7);
        assert!(dimension(&bad).is_err());
        assert!(dimension(&m("other")).is_err());
    }
}
