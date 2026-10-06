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
  incoming_sats: number;
  withdrawing_sats: number;
  withdrawals: { time: number; amount_sats: number; address: string; txid: string; stage: "waiting" | "bundled" | "sent" }[];
  pending_cost_sats: number;
}

export interface SettledRow {
  market_id: string;
  title: string;
  winners: string[];
  paid_sats: number;
  shares: number;
  outcomes: { label: string; shares: number; per_share: number }[];
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
  decisions: { id: string; question: string; rules: string; period: number }[];
  current_period: number;
  blocks_per_period: number | null;
  testing: boolean;
}

export interface MarketRow extends MarketSummary {
  leading: [string, number] | null;
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

/** Obliterate's two parts: Truthcoin (the node program, its data with the wallet, the wallet's records) and the app. */
export type ObliteratePart = "truthcoin" | "app";

export interface ObliterateItem {
  id: string;
  part: ObliteratePart;
  label: string;
  path: string;
  size: number;
  note: string;
  /** Goes when the app closes. */
  at_exit: boolean;
}

/** How the app's own program goes: removed as the app closes (`at_exit`), or by hand. */
export interface RemoveApp {
  kind: "mac" | "appimage" | "deb" | "other";
  path: string | null;
  at_exit: boolean;
}

export interface ObliteratePlan {
  items: ObliterateItem[];
  blocked: string | null;
  remove_app: RemoveApp;
  own_node_program: string | null;
}

export interface Obliterated {
  removed: string[];
  at_exit: string[];
  errors: string[];
  app_removed: boolean;
  remove_app: RemoveApp;
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

export type EnforcerCheck = {
  address: string;
  ok: boolean;
  remote: boolean;
  height: number;
  network: string;
  detail: string;
};

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
  enforcerTest: (address: string) => invoke<EnforcerCheck>("enforcer_test", { address }),
  enforcerSet: (address: string) => invoke<string>("enforcer_set", { address }),
  wallet: () => invoke<WalletStatus>("wallet_status"),
  newWords: () => invoke<{ words: string[]; ask: number[] }>("wallet_new_words"),
  confirmWords: (answers: string[]) => invoke<void>("wallet_confirm_words", { answers }),
  restore: (words: string) => invoke<void>("wallet_restore", { words }),
  receive: () => invoke<{ address: string; deposit_address: string }>("wallet_receive"),
  withdraw: (address: string, amountSats: number, feeSats: number, mainchainFeeSats: number) =>
    invoke<any>("withdraw", { address, amountSats, feeSats, mainchainFeeSats }),
  updateCheck: () => invoke<{ current: string; newer: string | null; url: string | null; note: string | null }>("update_check"),
  split: (parts: number) => invoke<any>("wallet_split", { parts }),
  markets: () => invoke<MarketRow[]>("markets"),
  settled: () => invoke<SettledRow[]>("settled"),
  withdrawalHide: (txid: string) => invoke<void>("withdrawal_hide", { txid }),
  ecashAddress: () => invoke<string>("ecash_address"),
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
  pairAnswer: (allow: boolean, replace = false) => invoke<void>("phone_pair_answer", { allow, replace }),
  pairCancel: () => invoke<void>("phone_pair_cancel"),
  phoneRevoke: (np: string) => invoke<void>("phone_revoke", { np }),
  phoneSetLimit: (np: string, limitSats: number) => invoke<void>("phone_set_limit", { np, limitSats }),
  heldAnswer: (id: string, approve: boolean) => invoke<any>("phone_held_answer", { id, approve }),
  setRelays: (relays: string[], page: string) => invoke<void>("phone_set_relays", { relays, page }),
  obliteratePlan: () => invoke<ObliteratePlan>("obliterate_plan"),
  obliterate: (truthcoin: boolean, theApp: boolean, words: boolean, shown: { id: string; path: string }[]) =>
    invoke<Obliterated>("obliterate", { truthcoin, theApp, words, shown }),
  appClose: () => invoke<void>("app_close"),
};

/** Open a link in the system's browser. Only the addresses tauri.conf.json allows (the app's GitHub pages) open. */
export function openUrl(url: string): Promise<void> {
  return invoke("plugin:shell|open", { path: url });
}

/** A command's error as text. */
export function errText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e);
}
