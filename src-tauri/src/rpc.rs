//! The Truthcoin node's JSON-RPC (truthcoin_dc, L2L). Our own client, written from the node's documented calls; none
//! of L2L's code is used. The node serves read-only calls on its RPC port and, because the app starts it with a
//! private RPC address, the wallet and node-control calls on a second port that only this app knows. Neither port
//! has a login: anything on this computer that finds them can call them (VERIFY.md says so).

use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

#[derive(Debug, Clone)]
pub enum RpcError {
    /// Nothing answered (the node isn't running, or is still starting).
    Unreachable(String),
    /// No answer within the call's time.
    Timeout,
    /// An HTTP error with no JSON-RPC body.
    Http(u16),
    /// The node answered with an error; its message is the only detail it gives.
    Rpc { code: i64, message: String },
    /// The answer wasn't what the call returns.
    Decode(String),
}

impl fmt::Display for RpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RpcError::Unreachable(_) => write!(f, "The Truthcoin node isn't answering"),
            RpcError::Timeout => write!(f, "The Truthcoin node took too long to answer"),
            RpcError::Http(s) => write!(f, "The Truthcoin node answered HTTP {s}"),
            RpcError::Rpc { message, .. } => write!(f, "{message}"),
            RpcError::Decode(e) => write!(f, "Unexpected answer from the Truthcoin node: {e}"),
        }
    }
}

impl From<RpcError> for String {
    fn from(e: RpcError) -> String {
        e.to_string()
    }
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
pub struct Rpc {
    http: reqwest::Client,
    public: String,
    private: String,
}

impl Rpc {
    pub fn new(http: reqwest::Client, rpc_port: u16, private_host: &str, private_port: u16) -> Rpc {
        Rpc {
            http,
            public: format!("http://127.0.0.1:{rpc_port}"),
            private: format!("http://{private_host}:{private_port}"),
        }
    }

    /// A read-only call (markets, chain state).
    pub async fn public<T: DeserializeOwned>(&self, method: &str, params: Value) -> Result<T, RpcError> {
        call(&self.http, &self.public, method, params, Duration::from_secs(30)).await
    }

    /// A wallet or node-control call.
    pub async fn private<T: DeserializeOwned>(&self, method: &str, params: Value) -> Result<T, RpcError> {
        call(&self.http, &self.private, method, params, Duration::from_secs(60)).await
    }

    /// A wallet call that may take long (a rescan).
    pub async fn private_slow<T: DeserializeOwned>(&self, method: &str, params: Value) -> Result<T, RpcError> {
        call(&self.http, &self.private, method, params, Duration::from_secs(600)).await
    }
}

pub async fn call<T: DeserializeOwned>(
    http: &reqwest::Client,
    url: &str,
    method: &str,
    params: Value,
    timeout: Duration,
) -> Result<T, RpcError> {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let body = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
    let resp = http.post(url).json(&body).timeout(timeout).send().await.map_err(|e| {
        if e.is_timeout() {
            RpcError::Timeout
        } else {
            RpcError::Unreachable(e.to_string())
        }
    })?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| if e.is_timeout() { RpcError::Timeout } else { RpcError::Unreachable(e.to_string()) })?;
    let v: Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) if !status.is_success() => return Err(RpcError::Http(status.as_u16())),
        Err(e) => return Err(RpcError::Decode(e.to_string())),
    };
    if let Some(err) = v.get("error").filter(|e| !e.is_null()) {
        let code = err.get("code").and_then(Value::as_i64).unwrap_or(0);
        let mut message = err.get("message").and_then(Value::as_str).unwrap_or("error").to_string();
        if let Some(d) = err.get("data").and_then(Value::as_str) {
            message = format!("{message}: {d}");
        }
        return Err(RpcError::Rpc { code, message });
    }
    let result = v.get("result").cloned().unwrap_or(Value::Null);
    serde_json::from_value(result).map_err(|e| RpcError::Decode(format!("{method}: {e}")))
}
