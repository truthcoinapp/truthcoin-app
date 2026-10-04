<script lang="ts">
  // Quote first, then trade: "about X, at most Y". The cap carries the 1,000-sat miner fee and a margin, so the
  // trade isn't skipped block after block. Fees are at least 2,000 sats a trade (1,000 trading, 1,000 miner): the
  // panel says so when they eat the whole payout.
  import { createEventDispatcher, onDestroy } from "svelte";
  import { api, errText, type Quote, type Side, type Trade } from "../lib/api";
  import { chance, num, parseWhole, sats } from "../lib/format";

  export let market: any;
  export let outcome: number;
  export let outcomeLabel: string;
  export let held = 0;
  export let side: Side = "buy";
  const dispatch = createEventDispatcher();
  const QUOTE_LIFE_MS = 120_000;

  let shares = "";
  let quote: Quote | null = null;
  let quotedAt = 0;
  let cap = "";
  let err = "";
  let moved = false;
  let busy = false;
  let done: Trade | null = null;
  let losingOk = false;
  let expiry: ReturnType<typeof setInterval> = setInterval(() => {
    if (quote && Date.now() - quotedAt > QUOTE_LIFE_MS) {
      quote = null;
      err = "That price is over two minutes old: get a new one.";
    }
  }, 5000);
  onDestroy(() => clearInterval(expiry));

  $: if (outcome !== undefined || side) reset();
  function reset() {
    quote = null;
    done = null;
    err = "";
    moved = false;
    losingOk = false;
  }

  async function getQuote() {
    const n = parseWhole(shares);
    if (!(n > 0)) return (err = "How many shares?");
    busy = true;
    err = "";
    moved = false;
    losingOk = false;
    try {
      quote = await api.quote(market.market_id, outcome, n, side);
      quotedAt = Date.now();
      cap = num(quote.limit_sats);
    } catch (e) {
      err = errText(e);
      quote = null;
    }
    busy = false;
  }

  async function place() {
    if (!quote) return;
    const c = parseWhole(cap);
    if (!(c > 0)) return (err = "The cap must be a number of sats");
    if (losing && !losingOk) return void (losingOk = true);
    busy = true;
    err = "";
    try {
      done = await api.place(market.market_id, outcome, quote.shares, side, c);
      quote = null;
      shares = "";
      dispatch("done");
    } catch (e) {
      err = errText(e);
      moved = err.startsWith("The price moved");
      if (moved) quote = null;
    }
    busy = false;
  }

  // What the trade costs or brings, fees included, and the fees' share of it.
  $: total = quote ? (side === "buy" ? quote.sats + quote.miner_fee_sats : Math.max(0, quote.sats - quote.miner_fee_sats)) : 0;
  $: fees = quote ? quote.trading_fee_sats + quote.miner_fee_sats : 0;
  $: feeShare = quote && total > 0 ? Math.round((100 * fees) / (side === "buy" ? total : total + fees)) : 0;
  // A buy that costs at least what its shares can ever pay can only lose.
  $: losing = !!quote && side === "buy" && total >= quote.shares;
</script>

<div class="card">
  <div class="row">
    <h2 style="margin:0">{side === "buy" ? "Buy" : "Sell"} {outcomeLabel}</h2>
    <button class="link" on:click={() => dispatch("close")}>Close</button>
  </div>
  <div class="seg" style="margin:10px 0">
    <button class:on={side === "buy"} on:click={() => (side = "buy")}>Buy</button>
    <button class:on={side === "sell"} on:click={() => (side = "sell")} disabled={!held}>Sell</button>
  </div>
  {#if done}
    <div class="notice ok">
      Sent to the node. It goes through with the next Truthcoin block (about 10–17 minutes) if the price is still within
      your cap; Home shows it until then.
    </div>
    <button on:click={() => (done = null)}>Another trade</button>
  {:else}
    <div class="field">
      <label for="sh">Shares{side === "sell" ? ` (you hold ${num(held)})` : ""}</label>
      <input id="sh" bind:value={shares} inputmode="numeric" placeholder="50,000" on:input={() => (quote = null)} disabled={busy} />
    </div>
    {#if err}
      <div class="notice error">{err}</div>
    {/if}
    {#if !quote}
      <button class="primary" disabled={busy} on:click={getQuote}>{#if busy}<span class="spin"></span>{/if} {moved || err ? "Get a new price" : "Get a price"}</button>
    {:else}
      <dl class="kv">
        {#if side === "buy"}
          <dt>Costs about</dt><dd>{sats(total)}</dd>
          <dt>At most</dt><dd>{sats(parseWhole(cap))}</dd>
          <dt>If it happens, pays</dt><dd>{sats(quote.shares)}</dd>
        {:else}
          <dt>You get about</dt><dd>{sats(total)}</dd>
          <dt>At least</dt><dd>{sats(Math.max(0, parseWhole(cap) - quote.miner_fee_sats))}</dd>
        {/if}
        <dt>Chance now → after</dt><dd>{chance(quote.price_now)} → {chance(quote.price_after)}</dd>
        <dt>Fees (included)</dt><dd>{sats(fees)}{feeShare ? ` (${feeShare}% of this trade)` : ""}</dd>
      </dl>
      {#if losing}
        <div class="notice error" style="margin-top:8px">
          This costs more than it can ever pay back ({sats(fees)} of fees on every trade). Buy more shares, or skip it.
        </div>
      {/if}
      <details>
        <summary>Change the cap</summary>
        <p class="small muted">
          The price is fixed when the next block is built. {side === "buy"
            ? "The trade waits if the cost (with the miner fee) would pass this."
            : "The trade waits if what you get (before the miner fee) would fall below this."}
        </p>
        <input bind:value={cap} inputmode="numeric" />
      </details>
      <div class="actions">
        <button class:primary={!losing} class:danger={losing} disabled={busy} on:click={place}>
          {#if busy}<span class="spin"></span>{/if}
          {losing && !losingOk ? "Buy anyway…" : losing ? `Yes, buy ${num(quote.shares)} at a loss` : side === "buy" ? `Buy ${num(quote.shares)}` : `Sell ${num(quote.shares)}`}
        </button>
        <button on:click={() => (quote = null)}>Back</button>
      </div>
    {/if}
  {/if}
</div>
