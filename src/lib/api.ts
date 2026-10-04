// Typed calls to the app's Rust side (src-tauri/src). Every call that touches the node, keys or files goes there.
import { invoke } from "@tauri-apps/api/core";

export type RunState =
  | { state: "stopped" }
  | { state: "starting" }
  | { state: "running" }
  | { state: "stopping" }
  | { state: "failed"; message: string };

export interface AppInfo {
  version: string;
  node_version: string;
  supported: boolean;
  dir: string;
  /** A release build: eCash beta only. */
  beta_only: boolean;
  /** This data folder's wallet has been set up. */
  wallet_ready: boolean;
}

export interface NodeStatus {
  installed: boolean;
  own_program: boolean;
  run: RunState;
  network: string;
  height: number | null;
  sync: { phase: string; done: number; total: number; tip_height: number } | null;
  peers: number | null;
  rpc_port: number;
  uptime_secs: number | null;
  wallet_host: string | null;
  enforcer: { address: string; remote: boolean; reachable: boolean; height: number; network: string; error: string | null };
}

export interface InstallProgress {
  running: boolean;
  done_bytes: number;
  total_bytes: number;
  error: string | null;
  finished: boolean;
}

export interface WalletStatus {
  has_seed: boolean;
  total_sats: number;
  available_sats: number;
  coins: number;
  in_pending_trades_sats: number;
  pending_trades: number;
}

export interface MarketSummary {
  market_id: string;
  title: string;
  description: string;
  outcome_count: number;
  state: string;
  volume_sats: number;
  created_at_height: number;
}

export interface Holding {
  market_id: string;
  market_title: string;
  market_state: string;
  outcome: number;
  outcome_label: string;
  shares: number;
  by_address: [string, number][];
  price: number;
  value_sats: number;
  paid_sats: number | null;
}

export type Side = "buy" | "sell";

export interface Quote {
  side: Side;
  market_id: string;
  market_title: string;
  outcome: number;
  outcome_label: string;
  shares: number;
  sats: number;
  trading_fee_sats: number;
  miner_fee_sats: number;
  price_now: number;
  price_after: number;
  limit_sats: number;
  seller_address: string | null;
}

export interface Trade {
  id: string;
  time: number;
  market_id: string;
  market_title: string;
  outcome: number;
  outcome_label: string;
  side: Side;
  shares: number;
  quoted_sats: number;
  limit_sats: number;
  txid: string | null;
  status: "sending" | "pending" | "done" | "failed" | "cancelled" | "dropped";
  source: string;
  height: number;
  error: string | null;
}

export interface MarketDetail {
  market: any;
  holdings: Holding[];
  height: number;
}

export interface PhoneInfo {
  devices: { name: string; np: string; limit_sats: number; left_sats: number; paired_at: number; last_seen: number }[];
  held: {
    id: string; np: string; name: string; at: number; market_id: string; title: string; outcome: number; label: string;
    shares: number; side: Side; limit_sats: number;
  }[];
  relays: string[];
  relay_status: { url: string; connected: boolean; error: string | null }[];
  page: string;
  default_limit_sats: number;
  fingerprint: string;
  blocked: string | null;
}

export interface NewMarket {
  title: string;
  description: string;
  kind: "binary" | "scaled" | "category";
  question: string;
  rules: string;
  period: number;
  no_label?: string;
  yes_label?: string;
  options?: string[];
  min?: number;
  max?: number;
  increment?: number;
  beta: number;
  trading_fee: number;
  tags: string[];
}

export const api = {
  appInfo: () => invoke<AppInfo>("app_info"),
  nodeStatus: () => invoke<NodeStatus>("node_status"),
  nodeInstall: () => invoke<void>("node_install"),
  installProgress: () => invoke<InstallProgress>("install_progress"),
  nodeStart: () => invoke<void>("node_start"),
  nodeStop: () => invoke<void>("node_stop"),
  nodeLog: () => invoke<string>("node_log"),
  advanced: () => invoke<{ network: string; enforcer: string; rpc_port: number; p2p_addr: string; zmq_port: number }>("settings_advanced"),
  advancedSet: (a: { network: string; enforcer: string; rpc_port: number; p2p_addr: string; zmq_port: number }) =>
    invoke<void>("settings_advanced_set", { a }),
  wallet: () => invoke<WalletStatus>("wallet_status"),
  newWords: () => invoke<{ words: string[]; ask: number[] }>("wallet_new_words"),
  confirmWords: (answers: string[]) => invoke<void>("wallet_confirm_words", { answers }),
  restore: (words: string) => invoke<void>("wallet_restore", { words }),
  receive: () => invoke<{ address: string; deposit_address: string }>("wallet_receive"),
  depositInfo: () =>
    invoke<{ enforcer: string; reachable: boolean; error: string | null; confirmed_sats: number; pending_sats: number; synced: boolean }>("deposit_info"),
  deposit: (amountSats: number, feeSats: number) => invoke<string>("deposit", { amountSats, feeSats }),
  withdraw: (address: string, amountSats: number, feeSats: number, mainchainFeeSats: number) =>
    invoke<any>("withdraw", { address, amountSats, feeSats, mainchainFeeSats }),
  updateCheck: () => invoke<{ current: string; newer: string | null; url: string | null; note: string | null }>("update_check"),
  split: (parts: number) => invoke<any>("wallet_split", { parts }),
  markets: () => invoke<MarketSummary[]>("markets"),
  market: (id: string) => invoke<MarketDetail>("market", { id }),
  positions: () => invoke<Holding[]>("positions"),
  quote: (marketId: string, outcome: number, shares: number, side: Side) =>
    invoke<Quote>("trade_quote", { marketId, outcome, shares, side }),
  place: (marketId: string, outcome: number, shares: number, side: Side, limitSats: number) =>
    invoke<Trade>("trade_place", { marketId, outcome, shares, side, limitSats }),
  trades: () => invoke<{ trades: Trade[]; height: number }>("trades"),
  cancel: (id: string) => invoke<void>("trade_cancel", { id }),
  clearTrade: (id: string) => invoke<void>("trade_clear", { id }),
  recordsSeen: () => invoke<void>("phone_records_seen"),
  createInfo: () =>
    invoke<{ current_period: number; current_period_name: string; blocks_per_period: number | null; testing: boolean; periods: any[] }>("create_info"),
  createCost: (market: NewMarket) =>
    invoke<{ liquidity_sats: number; listing_fee_sats: number; tx_fee_sats: number; total_sats: number }>("create_cost", { market }),
  createMarket: (market: NewMarket, maxListingFeeSats: number) =>
    invoke<{ txid: string; market_id: string }>("create_market", { market, maxListingFeeSats }),
  phoneInfo: () => invoke<PhoneInfo>("phone_info"),
  pairStart: () => invoke<string>("phone_pair_start"),
  pairState: () => invoke<{ state: string; name?: string; code?: string; expires?: number }>("phone_pair_state"),
  pairAnswer: (allow: boolean) => invoke<void>("phone_pair_answer", { allow }),
  pairCancel: () => invoke<void>("phone_pair_cancel"),
  phoneRevoke: (np: string) => invoke<void>("phone_revoke", { np }),
  phoneSetLimit: (np: string, limitSats: number) => invoke<void>("phone_set_limit", { np, limitSats }),
  heldAnswer: (id: string, approve: boolean) => invoke<any>("phone_held_answer", { id, approve }),
  setRelays: (relays: string[], page: string) => invoke<void>("phone_set_relays", { relays, page }),
};

/** Open a link in the system's browser. Only the addresses tauri.conf.json allows (the app's GitHub pages) open. */
export function openUrl(url: string): Promise<void> {
  return invoke("plugin:shell|open", { path: url });
}

/** A command's error as text. */
export function errText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e);
}
