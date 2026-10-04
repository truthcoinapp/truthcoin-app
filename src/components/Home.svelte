<script lang="ts">
  // Home: the balance, then the main actions opening in place (deposit from eCash, receive, withdraw), positions,
  // and trades with their state.
  import { createEventDispatcher, onDestroy, onMount } from "svelte";
  import { api, errText, type Holding, type Trade, type WalletStatus } from "../lib/api";
  import { chance, num, parseWhole, sats, short, STATUS_WORDS, when } from "../lib/format";
  import QrCode from "./QrCode.svelte";

  export let wallet: WalletStatus | null;
  export let running: boolean;
  const dispatch = createEventDispatcher();

  let panel: "" | "deposit" | "receive" | "withdraw" = "";
  let holdings: Holding[] = [];
  let trades: Trade[] = [];
  let height = 0;
  let err = "";
  let timer: ReturnType<typeof setInterval>;

  // Deposit
  let dep: { reachable: boolean; error: string | null; confirmed_sats: number; pending_sats: number; synced: boolean } | null = null;
  let depAmount = "";
  let depFee = "1000";
  let depDone = "";
  // Receive
  let recv: { address: string; deposit_address: string } | null = null;
  // Withdraw
  let wAddr = "";
  let wAmount = "";
  let wFee = "1000";
  let wMainFee = "1000";
  let wDone = "";
  let busy = false;
  // Money leaving: the panel shows what will go, and a second press sends it.
  let depConfirm = false;
  let wConfirm = false;

  async function load() {
    if (!running) return;
    try {
      holdings = await api.positions();
      const t = await api.trades();
      trades = t.trades;
      height = t.height;
      err = "";
    } catch (e) {
      err = errText(e);
    }
  }

  onMount(() => {
    load();
    timer = setInterval(load, 15000);
  });
  onDestroy(() => clearInterval(timer));
  $: if (running) load();

  async function open(p: typeof panel) {
    panel = panel === p ? "" : p;
    depDone = wDone = "";
    err = "";
    if (panel === "deposit") dep = await api.depositInfo().catch(() => null);
    if (panel === "receive") recv = await api.receive().catch((e) => ((err = errText(e)), null));
  }

  async function deposit() {
    const a = parseWhole(depAmount), f = parseWhole(depFee);
    if (!(a > 0) || !(f > 0)) return (err = "Give the amount and the eCash fee in sats");
    if (!depConfirm) return void (depConfirm = true);
    depConfirm = false;
    busy = true;
    err = "";
    try {
      const txid = await api.deposit(a, f);
      depDone = txid;
      depAmount = "";
    } catch (e) {
      err = errText(e);
    }
    busy = false;
  }

  async function withdraw() {
    const a = parseWhole(wAmount), f = parseWhole(wFee), m = parseWhole(wMainFee);
    if (!(a > 0) || !(f >= 0) || !(m >= 0)) return (err = "Give the amount and fees in sats");
    if (!wConfirm) return void (wConfirm = true);
    wConfirm = false;
    busy = true;
    err = "";
    try {
      const r = await api.withdraw(wAddr, a, f, m);
      wDone = typeof r === "string" ? r : JSON.stringify(r);
      wAmount = "";
    } catch (e) {
      err = errText(e);
    }
    busy = false;
  }

  let splitDone = false;
  async function split() {
    busy = true;
    err = "";
    try {
      await api.split(4);
      splitDone = true;
      dispatch("changed");
    } catch (e) {
      err = errText(e);
    }
    busy = false;
  }

  async function cancel(t: Trade) {
    err = "";
    try {
      await api.cancel(t.id);
      await load();
      dispatch("changed");
    } catch (e) {
      err = errText(e);
    }
  }

  $: pending = trades.filter((t) => t.status === "pending" || t.status === "sending");
  $: recent = trades.filter((t) => t.status !== "pending" && t.status !== "sending").slice(0, 10);
  $: totalValue = holdings.reduce((s, h) => s + h.value_sats, 0);
  // A trade still waiting two blocks after it was sent was most likely skipped: the price moved past its cap.
  const stuck = (t: Trade) => t.status === "pending" && height >= t.height + 2;
</script>

<div class="card">
  <div class="muted small">Balance</div>
  <div class="big">{wallet ? sats(wallet.total_sats) : "—"}</div>
  {#if wallet && wallet.in_pending_trades_sats > 0}
    <p class="small muted" style="margin-top:4px">
      Plus {sats(wallet.in_pending_trades_sats)} in {wallet.pending_trades} pending trade{wallet.pending_trades === 1 ? "" : "s"}:
      each spends a whole coin until its block, then the change comes back.
    </p>
  {/if}
  {#if totalValue > 0}
    <p class="small muted" style="margin-top:4px">Positions worth about {sats(totalValue)} at today's prices.</p>
  {/if}
  {#if wallet && wallet.coins === 1 && wallet.available_sats >= 100_000 && !splitDone}
    <div class="notice" style="margin-top:10px">
      Your balance is one coin, and a waiting trade holds a whole coin until its block, so only one trade can wait at a
      time. <button class="link" disabled={busy} on:click={split}>Split it into 4 coins</button> (one transfer, 1,000 sats
      fee, done at the next block).
    </div>
  {:else if splitDone}
    <p class="small muted" style="margin-top:6px">Splitting: done at the next Truthcoin block.</p>
  {/if}
  <div class="actions">
    <button class:primary={panel === "deposit"} on:click={() => open("deposit")}>Deposit from eCash</button>
    <button class:primary={panel === "receive"} on:click={() => open("receive")}>Receive</button>
    <button class:primary={panel === "withdraw"} on:click={() => open("withdraw")}>Withdraw</button>
  </div>
</div>

{#if err}<div class="notice error">{err}</div>{/if}

{#if panel === "deposit"}
  <div class="card">
    <h2>Deposit from eCash</h2>
    {#if !dep}
      <p class="muted"><span class="spin"></span></p>
    {:else if !dep.reachable}
      <div class="notice error">BitWindow's eCash wallet (the enforcer's) isn't answering: {dep.error}</div>
    {:else}
      <p class="small muted">
        From BitWindow's eCash wallet (the enforcer's): {sats(dep.confirmed_sats)} available{dep.pending_sats ? `, ${sats(dep.pending_sats)} pending` : ""}{dep.synced ? "" : " (still syncing)"}.
        The coins arrive here once the deposit is in an eCash block and the next Truthcoin block after it.
      </p>
      <div class="field"><label for="da">Amount (sats)</label><input id="da" bind:value={depAmount} inputmode="numeric" placeholder="100,000" on:input={() => (depConfirm = false)} /></div>
      <div class="field"><label for="df">eCash fee (sats)</label><input id="df" bind:value={depFee} inputmode="numeric" on:input={() => (depConfirm = false)} /></div>
      {#if depConfirm}
        <div class="notice warn">
          Move {sats(parseWhole(depAmount))} from BitWindow's eCash wallet to this Truthcoin wallet, paying
          {sats(parseWhole(depFee))} on eCash?
        </div>
      {/if}
      <button class="primary" disabled={busy} on:click={deposit}>{#if busy}<span class="spin"></span>{/if} {depConfirm ? "Yes, deposit" : "Deposit"}</button>
      {#if depConfirm}<button on:click={() => (depConfirm = false)}>Not now</button>{/if}
      {#if depDone}<div class="notice ok" style="margin-top:10px">Sent on eCash: <code>{short(depDone)}</code></div>{/if}
    {/if}
  </div>
{:else if panel === "receive"}
  <div class="card">
    <h2>Receive</h2>
    {#if recv}
      <p class="small muted">From another Truthcoin wallet, this address:</p>
      <p><code>{recv.address}</code></p>
      <p class="small muted" style="margin-top:10px">For a deposit from an eCash wallet that asks for a sidechain deposit address (BitWindow does):</p>
      <div class="qr-wrap"><QrCode text={recv.deposit_address} size={180} /></div>
      <p><code>{recv.deposit_address}</code></p>
    {:else}
      <p class="muted"><span class="spin"></span></p>
    {/if}
  </div>
{:else if panel === "withdraw"}
  <div class="card">
    <h2>Withdraw to eCash</h2>
    <p class="small muted">
      A withdrawal joins the next bundle, which eCash miners approve over many blocks before it pays out. Expect days, not
      minutes.
    </p>
    <div class="field"><label for="wa">eCash address</label><input id="wa" bind:value={wAddr} autocomplete="off" spellcheck="false" on:input={() => (wConfirm = false)} /></div>
    <div class="field"><label for="wm">Amount (sats)</label><input id="wm" bind:value={wAmount} inputmode="numeric" on:input={() => (wConfirm = false)} /></div>
    <div class="row">
      <div class="field" style="flex:1"><label for="wf">Truthcoin fee</label><input id="wf" bind:value={wFee} inputmode="numeric" on:input={() => (wConfirm = false)} /></div>
      <div class="field" style="flex:1"><label for="wmf">eCash fee</label><input id="wmf" bind:value={wMainFee} inputmode="numeric" on:input={() => (wConfirm = false)} /></div>
    </div>
    {#if wConfirm}
      <div class="notice warn">
        Withdraw {sats(parseWhole(wAmount))} to <code>{wAddr.trim()}</code>, paying {sats(parseWhole(wFee))} here and
        {sats(parseWhole(wMainFee))} on eCash? It can't be taken back.
      </div>
    {/if}
    <button class="primary" disabled={busy || !wAddr.trim()} on:click={withdraw}>{#if busy}<span class="spin"></span>{/if} {wConfirm ? "Yes, withdraw" : "Withdraw"}</button>
    {#if wConfirm}<button on:click={() => (wConfirm = false)}>Not now</button>{/if}
    {#if wDone}<div class="notice ok" style="margin-top:10px">Withdrawal started: <code>{short(wDone, 14)}</code></div>{/if}
  </div>
{/if}

{#if pending.length}
  <h3>Pending trades</h3>
  <div class="card flush list">
    {#each pending as t}
      <div class="item">
        <div>
          <div class="title">{t.side === "buy" ? "Buy" : "Sell"} {num(t.shares)} {t.outcome_label}</div>
          <div class="small muted">{t.market_title}</div>
          <div class="small" style:color={stuck(t) ? "var(--warn)" : "var(--text-2)"}>
            {#if t.status === "sending"}
              {STATUS_WORDS.sending}
            {:else if stuck(t)}
              Still waiting after {height - t.height} blocks: the price has probably moved past your cap.
            {:else}
              Waiting for the next Truthcoin block (about 10–17 minutes)
            {/if}
          </div>
        </div>
        <div style="text-align:right">
          <div class="small">{t.side === "buy" ? "at most" : "at least"} {sats(t.limit_sats)}</div>
          {#if t.status === "pending"}
            <button class="link small" on:click={() => cancel(t)} title="Take it out of this node's queue and get its coin back">Cancel</button>
          {:else if t.status === "sending" && !t.txid && Date.now() / 1000 - t.time > 120}
            <button class="link small" title="The node never answered for this trade. Check Positions; if it isn't there, clear it."
              on:click={() => api.clearTrade(t.id).then(load).catch((e) => (err = errText(e)))}>It didn't go through</button>
          {/if}
        </div>
      </div>
    {/each}
  </div>
{/if}

<h3>Positions</h3>
{#if !holdings.length}
  <p class="muted small">None yet. Pick a market in Markets.</p>
{:else}
  <div class="card flush list">
    {#each holdings as h}
      <button class="item" on:click={() => dispatch("market", h.market_id)}>
        <div>
          <div class="title">{h.outcome_label}</div>
          <div class="small muted">{h.market_title}{h.market_state !== "trading" ? ` · ${h.market_state}` : ""}</div>
          <div class="small muted">{num(h.shares)} shares · {chance(h.price)} chance</div>
        </div>
        <div style="text-align:right">
          <div>{sats(h.value_sats)}</div>
          {#if h.paid_sats !== null}<div class="small muted">paid {sats(h.paid_sats)}</div>{/if}
        </div>
      </button>
    {/each}
  </div>
{/if}

{#if recent.length}
  <h3>Recent trades</h3>
  <div class="card flush list">
    {#each recent as t}
      <div class="item">
        <div>
          <div>{t.side === "buy" ? "Bought" : "Sold"} {num(t.shares)} {t.outcome_label}</div>
          <div class="small muted">{t.market_title} · {when(t.time)}{t.source !== "desktop" ? " · from a phone" : ""}</div>
          {#if t.error}<div class="small" style="color:var(--error)">{t.error}</div>{/if}
        </div>
        <div style="text-align:right">
          <div class="small">{sats(t.side === "buy" ? t.quoted_sats + 1000 : Math.max(0, t.quoted_sats - 1000))}</div>
          <div class="small muted">{STATUS_WORDS[t.status]}</div>
        </div>
      </div>
    {/each}
  </div>
{/if}
