<script lang="ts">
  // Quote first, then trade: "about X, at most Y". The cap carries the 1,000-sat miner fee and a margin, so the
  // trade isn't skipped block after block.
  import { createEventDispatcher } from "svelte";
  import { api, errText, type Quote, type Side, type Trade } from "../lib/api";
  import { chance, num, parseWhole, sats } from "../lib/format";

  export let market: any;
  export let outcome: number;
  export let outcomeLabel: string;
  export let held = 0;
  const dispatch = createEventDispatcher();

  let side: Side = "buy";
  let shares = "";
  let quote: Quote | null = null;
  let cap = "";
  let err = "";
  let busy = false;
  let done: Trade | null = null;

  $: if (outcome !== undefined || side) reset();
  function reset() {
    quote = null;
    done = null;
    err = "";
  }

  async function getQuote() {
    const n = parseWhole(shares);
    if (!(n > 0)) return (err = "How many shares?");
    busy = true;
    err = "";
    try {
      quote = await api.quote(market.market_id, outcome, n, side);
      cap = String(quote.limit_sats);
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
    busy = true;
    err = "";
    try {
      done = await api.place(market.market_id, outcome, quote.shares, side, c);
      quote = null;
      shares = "";
      dispatch("done");
    } catch (e) {
      err = errText(e);
    }
    busy = false;
  }
</script>

<div class="card">
  <div class="row">
    <h2 style="margin:0">{outcomeLabel}</h2>
    <button class="link" on:click={() => dispatch("close")}>Close</button>
  </div>
  <div class="seg" style="margin:10px 0">
    <button class:on={side === "buy"} on:click={() => (side = "buy")}>Buy</button>
    <button class:on={side === "sell"} on:click={() => (side = "sell")} disabled={!held}>Sell</button>
  </div>
  {#if done}
    <div class="notice ok">
      Sent. It goes through with the next Truthcoin block (about 10–17 minutes); Home shows it until then.
    </div>
    <button on:click={() => (done = null)}>Another trade</button>
  {:else}
    <div class="field">
      <label for="sh">Shares{side === "sell" ? ` (you hold ${num(held)})` : ""}</label>
      <input id="sh" bind:value={shares} inputmode="numeric" placeholder="10,000" on:input={() => (quote = null)} />
    </div>
    {#if err}<div class="notice error">{err}</div>{/if}
    {#if !quote}
      <button class="primary" disabled={busy} on:click={getQuote}>{#if busy}<span class="spin"></span>{/if} Get a price</button>
    {:else}
      <dl class="kv">
        {#if side === "buy"}
          <dt>Costs about</dt><dd>{sats(quote.sats + quote.miner_fee_sats)}</dd>
          <dt>At most</dt><dd>{sats(parseWhole(cap))}</dd>
          <dt>If it happens, pays</dt><dd>{sats(quote.shares)}</dd>
        {:else}
          <dt>Brings about</dt><dd>{sats(Math.max(0, quote.sats - quote.miner_fee_sats))}</dd>
          <dt>At least (before the miner fee)</dt><dd>{sats(parseWhole(cap))}</dd>
        {/if}
        <dt>Chance now → after</dt><dd>{chance(quote.price_now)} → {chance(quote.price_after)}</dd>
        <dt>Fees (included)</dt><dd>{sats(quote.trading_fee_sats)} trading, {sats(quote.miner_fee_sats)} miner</dd>
      </dl>
      <details>
        <summary>Change the cap</summary>
        <p class="small muted">
          The price is fixed when the next block is built. {side === "buy"
            ? "The trade waits if the cost (with the miner fee) would pass this."
            : "The trade waits if what you get (less the miner fee) would fall below this."}
        </p>
        <input bind:value={cap} inputmode="numeric" />
      </details>
      <div class="actions">
        <button class="primary" disabled={busy} on:click={place}>
          {#if busy}<span class="spin"></span>{/if}
          {side === "buy" ? `Buy ${num(quote.shares)}` : `Sell ${num(quote.shares)}`}
        </button>
        <button on:click={() => (quote = null)}>Back</button>
      </div>
    {/if}
  {/if}
</div>
