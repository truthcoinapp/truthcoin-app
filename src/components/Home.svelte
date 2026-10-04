<script lang="ts">
  // Home: the balance as people read it (what you hold, with what's moving listed under it), then the main actions
  // opening in place (deposit from eCash, receive, withdraw), trades on their way, settled markets, positions and recent
  // trades.
  import { createEventDispatcher, onDestroy, onMount } from "svelte";
  import { api, errText, type Holding, type SettledRow, type Trade, type WalletStatus } from "../lib/api";
  import { chance, num, parseWhole, sats, short, when } from "../lib/format";
  import { copy, COPY_FAILED } from "../lib/copy";
  import QrCode from "./QrCode.svelte";

  export let wallet: WalletStatus | null;
  export let running: boolean;
  export let height: number | null = null;
  const dispatch = createEventDispatcher();
  const MINER_FEE = 1000;

  let panel: "" | "deposit" | "receive" | "withdraw" = "";
  let holdings: Holding[] = [];
  let trades: Trade[] = [];
  let settled: SettledRow[] = [];
  let tipHeight = 0;
  let loadErr = "";
  let panelErr = "";
  let timer: ReturnType<typeof setInterval>;

  // Deposit
  let dep: { reachable: boolean; error: string | null; confirmed_sats: number; pending_sats: number; synced: boolean } | null = null;
  let depAmount = "";
  let depFee = "1,000";
  let depDone = "";
  let depHold = false;
  // Receive
  let recv: { address: string; deposit_address: string } | null = null;
  let copied = "";
  // Withdraw
  let wAddr = "";
  let wAmount = "";
  let wFee = "1,000";
  let wMainFee = "1,000";
  let wDone = "";
  let busy = false;
  // Money leaving: the panel shows what will go, and a second press sends it.
  let depConfirm = false;
  let wConfirm = false;
  let cancelAsk: string | null = null;

  async function load() {
    if (!running) return;
    try {
      holdings = await api.positions();
      const t = await api.trades();
      trades = t.trades;
      tipHeight = t.height;
      settled = await api.settled().catch(() => settled);
      loadErr = "";
    } catch (e) {
      loadErr = errText(e);
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
    depDone = wDone = panelErr = "";
    depConfirm = wConfirm = false;
    if (panel === "deposit") dep = await api.depositInfo().catch(() => null);
    if (panel === "receive") recv = await api.receive().catch((e) => ((panelErr = errText(e)), null));
  }

  async function doCopy(text: string, what: string) {
    if (await copy(text)) {
      copied = what;
      panelErr = "";
    } else {
      copied = "";
      panelErr = COPY_FAILED;
    }
    setTimeout(() => (copied = ""), 2500);
  }
  const STAGE: Record<string, string> = {
    waiting: "waiting for the next bundle",
    bundled: "in the bundle eCash miners are approving (takes days)",
    sent: "with eCash miners: it pays out once they approve it (days); BitWindow shows the payment",
  };
  // "You got 50,000 sats (50,000 Yes shares; 20,000 No shares paid nothing)".
  function settledWords(s: SettledRow): string {
    const parts = s.outcomes.map((o) =>
      o.per_share >= 0.999 ? `${num(o.shares)} ${o.label} shares` : o.per_share > 0 ? `${num(o.shares)} ${o.label} shares at ${o.per_share.toFixed(2)} sat` : `${num(o.shares)} ${o.label} shares paid nothing`,
    );
    return `You got ${sats(s.paid_sats)} (${parts.join("; ")})`;
  }

  async function deposit() {
    const a = parseWhole(depAmount), f = parseWhole(depFee);
    if (!(a > 0) || !(f > 0)) return (panelErr = "Give the amount and the eCash fee in sats");
    if (!depConfirm) return void (depConfirm = true);
    depConfirm = false;
    busy = true;
    panelErr = "";
    try {
      const txid = await api.deposit(a, f);
      depDone = txid;
      depAmount = "";
      // The eCash wallet's figures change: read them again, and give the button a moment so it isn't pressed twice.
      depHold = true;
      setTimeout(() => (depHold = false), 8000);
      dep = await api.depositInfo().catch(() => dep);
      dispatch("changed");
    } catch (e) {
      panelErr = errText(e);
    }
    busy = false;
  }

  // An eCash address: Bitcoin's forms (bech32 bc1/tb1/bcrt1, or base58 1/3/m/n/2), as eCash beta uses them.
  function looksLikeEcash(a: string): boolean {
    const s = a.trim();
    return /^(bc1|tb1|bcrt1)[02-9ac-hj-np-z]{8,87}$/i.test(s) || /^[123mn][1-9A-HJ-NP-Za-km-z]{25,39}$/.test(s);
  }

  async function fillEcash() {
    panelErr = "";
    try {
      wAddr = await api.ecashAddress();
      wConfirm = false;
    } catch (e) {
      panelErr = `BitWindow's eCash wallet didn't give an address: ${errText(e)}`;
    }
  }

  function withdrawMax() {
    if (!wallet) return;
    const f = parseWhole(wFee) || 0, m = parseWhole(wMainFee) || 0;
    wAmount = num(Math.max(0, wallet.available_sats - f - m - 1000));
    wConfirm = false;
  }

  async function withdraw() {
    const a = parseWhole(wAmount), f = parseWhole(wFee), m = parseWhole(wMainFee);
    if (!looksLikeEcash(wAddr)) return (panelErr = "That isn't an eCash address. In BitWindow, Receive gives you one, or use \"Send to BitWindow's eCash wallet\" above.");
    if (!(a > 0) || !(f >= 0) || !(m >= 0)) return (panelErr = "Give the amount and the fees in sats");
    if (!wConfirm) return void ((wConfirm = true), (panelErr = ""));
    wConfirm = false;
    busy = true;
    panelErr = "";
    try {
      const r = await api.withdraw(wAddr.trim(), a, f, m);
      wDone = typeof r === "string" ? r : JSON.stringify(r);
      wAmount = "";
      dispatch("changed");
    } catch (e) {
      panelErr = errText(e);
    }
    busy = false;
  }

  async function cancel(t: Trade) {
    if (cancelAsk !== t.id) return void (cancelAsk = t.id);
    cancelAsk = null;
    try {
      await api.cancel(t.id);
      await load();
      dispatch("changed");
    } catch (e) {
      loadErr = errText(e);
    }
  }

  let splitDone = false;
  let splitAt = 0;
  async function split() {
    busy = true;
    try {
      await api.split(4);
      splitDone = true;
      splitAt = height ?? 0;
      dispatch("changed");
    } catch (e) {
      loadErr = errText(e);
    }
    busy = false;
  }
  $: if (splitDone && height !== null && height > splitAt) splitDone = false;

  $: pending = trades.filter((t) => t.status === "pending" || t.status === "sending");
  $: recent = trades.filter((t) => t.status !== "pending" && t.status !== "sending").slice(0, 10);
  $: totalValue = holdings.reduce((s, h) => s + h.value_sats, 0);
  // A trade still waiting two blocks after it was sent was most likely skipped: the price moved past its cap.
  const stuck = (t: Trade) => t.status === "pending" && tipHeight >= t.height + 2;
  // What a trade cost or brought, the miner fee included.
  const amount = (t: Trade) => (t.side === "buy" ? t.quoted_sats + MINER_FEE : Math.max(0, t.quoted_sats - MINER_FEE));
  const verb = (t: Trade) => (t.side === "buy" ? "buy" : "sell");
  const now = () => Date.now() / 1000;
</script>

<div class="card" class:stale={!running}>
  <div class="muted small">Balance</div>
  <div class="big">{wallet ? sats(wallet.total_sats) : "—"}</div>
  {#if !running && wallet}
    <p class="small muted">The node isn't running: these are the last figures it gave{height ? ` (block ${height})` : ""}.</p>
  {/if}
  {#if wallet}
    <dl class="kv small" style="margin-top:6px">
      {#if wallet.available_sats !== wallet.total_sats}
        <dt>Ready to use now</dt><dd>{sats(wallet.available_sats)}</dd>
      {/if}
      {#if wallet.in_pending_trades_sats > 0}
        <dt>Held by {wallet.pending_trades} waiting trade{wallet.pending_trades === 1 ? "" : "s"}</dt>
        <dd>{sats(wallet.in_pending_trades_sats)}{wallet.pending_cost_sats ? ` (${sats(wallet.pending_cost_sats)} of it pays for the trade${wallet.pending_trades === 1 ? "" : "s"})` : ""}</dd>
      {/if}
      {#if wallet.incoming_sats > 0}
        <dt>Change coming back</dt><dd>{sats(wallet.incoming_sats)}</dd>
      {/if}
      {#if wallet.withdrawing_sats > 0}
        <dt>On its way to eCash</dt><dd>{sats(wallet.withdrawing_sats)}</dd>
      {/if}
      {#if totalValue > 0}
        <dt>Positions, at today's prices</dt><dd>about {sats(totalValue)}</dd>
      {/if}
    </dl>
    {#if wallet.in_pending_trades_sats > 0}
      <p class="small muted">A waiting trade holds a whole piece of your balance until its block; the change comes back then.</p>
    {/if}
    {#if wallet.withdrawing_sats > 0}
      <p class="small muted">A withdrawal pays out on eCash once miners approve its bundle: that takes days.</p>
    {/if}
    {#each wallet.recent_deposits as d}
      <p class="small muted">On its way: a deposit from eCash, {sats(d.amount_sats)}, sent {when(d.time)}. It arrives after the next eCash block and the Truthcoin block after it.</p>
    {/each}
    {#each wallet.withdrawals as w}
      <p class="small muted">
        Withdrawal of {sats(w.amount_sats)} to eCash, {when(w.time)}: {STAGE[w.stage] ?? w.stage}.
        <button class="link small" on:click={() => api.withdrawalHide(w.txid).then(() => dispatch("changed"))}>Hide</button>
      </p>
    {/each}
  {/if}
  {#if wallet && wallet.coins === 1 && wallet.available_sats >= 100_000 && !splitDone}
    <div class="notice" style="margin-top:10px">
      Your balance is in one piece, and a waiting trade ties up a whole piece until its block, so only one trade can
      wait at a time. <button class="link" disabled={busy} on:click={split}>Split it into 4 pieces</button> (one
      transfer, 1,000 sats fee, done at the next block).
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

{#if loadErr}<div class="notice error">{loadErr}</div>{/if}

{#if panel === "deposit"}
  <div class="card">
    <h2>Deposit from eCash</h2>
    {#if !dep}
      <p class="muted"><span class="spin"></span></p>
    {:else if !dep.reachable}
      <div class="notice error">BitWindow's eCash wallet (the enforcer's) isn't answering: {dep.error}</div>
    {:else}
      <p class="small muted">
        From BitWindow's eCash wallet: {sats(dep.confirmed_sats)} available{dep.pending_sats ? `, ${sats(dep.pending_sats)} pending` : ""}{dep.synced ? "" : " (still syncing)"}.
        The coins arrive here after the deposit's eCash block and the Truthcoin block after it.
      </p>
      <div class="field"><label for="da">Amount (sats)</label><input id="da" bind:value={depAmount} inputmode="numeric" placeholder="100,000" on:input={() => (depConfirm = false)} /></div>
      <div class="field"><label for="df">eCash fee (sats)</label><input id="df" bind:value={depFee} inputmode="numeric" on:input={() => (depConfirm = false)} /></div>
      {#if depConfirm}
        <div class="notice warn">
          Move {sats(parseWhole(depAmount))} from BitWindow's eCash wallet to this Truthcoin wallet, paying
          {sats(parseWhole(depFee))} on eCash?
        </div>
      {/if}
      {#if panelErr}<div class="notice error">{panelErr}</div>{/if}
      <div class="actions">
        <button class="primary" disabled={busy || depHold} on:click={deposit}>{#if busy}<span class="spin"></span>{/if} {depConfirm ? "Yes, deposit" : "Deposit"}</button>
        {#if depConfirm}<button on:click={() => (depConfirm = false)}>Not now</button>{/if}
      </div>
      {#if depDone}<div class="notice ok" style="margin-top:10px">Sent on eCash: <code>{short(depDone)}</code>. Home shows it until it arrives.</div>{/if}
    {/if}
  </div>
{:else if panel === "receive"}
  <div class="card">
    <h2>Receive</h2>
    {#if panelErr}<div class="notice error">{panelErr}</div>{/if}
    {#if recv}
      <p class="small muted">From another Truthcoin wallet, this address:</p>
      <div class="row"><code>{recv.address}</code><button class="small" on:click={() => recv && doCopy(recv.address, "address")}>{copied === "address" ? "Copied" : "Copy"}</button></div>
      <p class="small muted" style="margin-top:10px">For a deposit from an eCash wallet that asks for a sidechain deposit address (BitWindow does):</p>
      <div class="qr-wrap"><QrCode text={recv.deposit_address} size={180} /></div>
      <div class="row"><code>{recv.deposit_address}</code><button class="small" on:click={() => recv && doCopy(recv.deposit_address, "deposit")}>{copied === "deposit" ? "Copied" : "Copy"}</button></div>
    {:else if !panelErr}
      <p class="muted"><span class="spin"></span></p>
    {/if}
  </div>
{:else if panel === "withdraw"}
  <div class="card">
    <h2>Withdraw to eCash</h2>
    <p class="small muted">
      A withdrawal joins the next bundle, which eCash miners approve over many blocks before it pays out: expect days,
      not minutes. You pay a Truthcoin fee now and an eCash fee when the bundle pays out.
    </p>
    <div class="field">
      <label for="wa">eCash address</label>
      <input id="wa" bind:value={wAddr} autocomplete="off" spellcheck="false" on:input={() => (wConfirm = false)} />
      <button class="link small" style="margin-top:4px" on:click={fillEcash}>Send to BitWindow's eCash wallet</button>
    </div>
    <div class="field">
      <label for="wm">Amount (sats) <button class="link small" on:click={withdrawMax}>Max</button></label>
      <input id="wm" bind:value={wAmount} inputmode="numeric" on:input={() => (wConfirm = false)} />
    </div>
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
    {#if panelErr}<div class="notice error">{panelErr}</div>{/if}
    <div class="actions">
      <button class="primary" disabled={busy || !wAddr.trim()} on:click={withdraw}>{#if busy}<span class="spin"></span>{/if} {wConfirm ? "Yes, withdraw" : "Withdraw"}</button>
      {#if wConfirm}<button on:click={() => (wConfirm = false)}>Not now</button>{/if}
    </div>
    {#if wDone}<div class="notice ok" style="margin-top:10px">Withdrawal started: <code>{short(wDone, 14)}</code>. Home shows it as on its way to eCash.</div>{/if}
  </div>
{/if}

{#if pending.length}
  <h3>Waiting for their block</h3>
  <div class="card flush list">
    {#each pending as t}
      <div class="item">
        <div style="flex:1">
          <div class="title">{t.side === "buy" ? "Buy" : "Sell"} {num(t.shares)} {t.outcome_label}</div>
          <div class="small muted">{t.market_title}</div>
          <div class="small" style:color={stuck(t) ? "var(--warn)" : "var(--text-2)"}>
            {#if t.status === "sending"}
              No answer came from the node for this trade. Check Positions: if it isn't there after a block, it didn't go through.
            {:else if stuck(t)}
              Still waiting after {tipHeight - t.height} blocks: the price has probably moved past your cap.
            {:else}
              Goes through with the next Truthcoin block (about 10–17 minutes)
            {/if}
          </div>
          {#if t.status === "pending" && cancelAsk !== t.id}
            <button class="small" style="margin-top:6px" on:click={() => cancel(t)}>Cancel and get the coin back</button>
          {:else if t.status === "sending" && !t.txid && now() - t.time > 120}
            <button class="link small" on:click={() => api.clearTrade(t.id).then(load).catch((e) => (loadErr = errText(e)))}>It didn't go through</button>
          {/if}
          {#if cancelAsk === t.id}
            <div class="notice warn small" style="margin-top:6px">
              Take it out of this node's queue and get its coin back? If other nodes already have it and the price comes
              back within your cap, it may still go through.
              <div class="actions"><button class="danger" on:click={() => cancel(t)}>Yes, cancel it</button><button on:click={() => (cancelAsk = null)}>Keep waiting</button></div>
            </div>
          {/if}
        </div>
        <div class="nowrap" style="text-align:right">
          <div class="small">{t.side === "buy" ? "at most" : "at least"} {sats(t.limit_sats)}</div>
        </div>
      </div>
    {/each}
  </div>
{/if}

{#if settled.length}
  <h3>Settled</h3>
  <div class="card flush list">
    {#each settled as s}
      <button class="item" on:click={() => dispatch("market", s.market_id)}>
        <div style="flex:1">
          <div class="title">{s.title}</div>
          <div class="small">Settled: {s.winners.join(", ") || "no single answer"}</div>
          <div class="small muted">{settledWords(s)}</div>
        </div>
      </button>
    {/each}
  </div>
{/if}

<h3>Positions</h3>
{#if !running}
  <p class="muted small">Positions show when the node is running.</p>
{:else if !holdings.length}
  <p class="muted small">None yet. Pick a market in Markets.</p>
{:else}
  <div class="card flush list">
    {#each holdings as h}
      <button class="item" on:click={() => dispatch("market", h.market_id)}>
        <div style="flex:1">
          <div class="title">{h.outcome_label}</div>
          <div class="small muted">{h.market_title}{h.market_state !== "trading" ? ` · ${h.market_state}` : ""}</div>
          <div class="small muted">{num(h.shares)} shares · {chance(h.price)} chance</div>
        </div>
        <div class="nowrap" style="text-align:right">
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
      <div class="item" class:faded={t.status !== "done"}>
        <div style="flex:1">
          {#if t.status === "done"}
            <div>{t.side === "buy" ? "Bought" : "Sold"} {num(t.shares)} {t.outcome_label}</div>
          {:else}
            <div>{t.status === "cancelled" ? "Cancelled" : t.status === "failed" ? "Refused" : "Didn't go through"}: {verb(t)} {num(t.shares)} {t.outcome_label}</div>
          {/if}
          <div class="small muted">{t.market_title} · {when(t.time)}{t.source !== "desktop" ? " · from a phone" : ""}</div>
          {#if t.error}<div class="small" style="color:var(--error)">{t.error}</div>{/if}
        </div>
        {#if t.status === "done"}
          <div class="small nowrap" style="text-align:right">{sats(amount(t))}</div>
        {/if}
      </div>
    {/each}
  </div>
{/if}
