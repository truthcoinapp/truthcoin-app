<script lang="ts">
  // Buy or sell shares of one outcome: get a price from the computer, then trade at "about X, at most Y".
  import { createEventDispatcher, onDestroy } from 'svelte';
  import { failureText } from '../lib/api';
  import { fmtChanceFine, fmtSats, fmtShares, parseShares } from '../lib/format';
  import { currentApi, flows, status, tradeFlowsNow } from '../lib/session';
  import type { Market, Quote, Side } from '../lib/validate';
  import Back from './Back.svelte';
  import FlowCard from './FlowCard.svelte';

  export let market: Market;
  export let outcome: number;
  export let side: Side = 'buy';

  const dispatch = createEventDispatcher<{ back: null; home: null }>();
  const o = market.outcomes.find((x) => x.i === outcome)!;
  const label = o?.label || `Outcome ${outcome + 1}`;
  const holding = market.holdings.find((h) => h.outcome === outcome);
  const heldShares = Math.floor(holding?.shares ?? 0);

  let sharesText = '';
  let quote: Quote | null = null;
  /** What the shown quote was asked for: the trade sends exactly these. */
  let quoteFor: { shares: number; side: Side } | null = null;
  let quotedAt = 0;
  let quoting = false;
  let error = '';
  let flowId: string | null = null;
  let starting = false;
  let now = Date.now();
  const clock = setInterval(() => (now = Date.now()), 5000);
  onDestroy(() => clearInterval(clock));

  $: shares = parseShares(sharesText);
  $: tooMany = side === 'sell' && shares !== null && shares > heldShares;
  $: flow = flowId ? $flows.find((f) => f.id === flowId) ?? null : null;
  $: stale = quote && now - quotedAt > 120_000;
  // What the trade costs (buy) or brings (sell), all fees counted: the miner fee is paid on top of a buy's price and
  // comes out of a sell's proceeds.
  $: about = quote ? (side === 'buy' ? quote.sats + quote.minerFee : Math.max(0, quote.sats - quote.minerFee)) : 0;
  // What counts against the phone's daily limit: a buy at its cap; a sell at its number of shares, a sat each (the
  // most they can pay, so no price move lowers it).
  $: counts = quote ? (side === 'buy' ? quote.limit : shares ?? 0) : 0;
  $: overLimit = quote && $status ? counts > $status.leftSats : false;

  function pick(s: Side) {
    if (quoting) return;
    side = s;
    quote = null;
    quoteFor = null;
    error = '';
  }

  async function getPrice() {
    if (shares === null || tooMany || quoting) return;
    const asked = { shares, side };
    quoting = true;
    error = '';
    quote = null;
    quoteFor = null;
    try {
      const q = await currentApi().quote({ id: market.id, outcome, shares: asked.shares, side: asked.side });
      // An answer for a form that has changed meanwhile isn't this trade's price.
      if (shares !== asked.shares || side !== asked.side) return;
      quote = q;
      quoteFor = asked;
      quotedAt = Date.now();
      now = quotedAt;
    } catch (e) {
      error = failureText(e);
    } finally {
      quoting = false;
    }
  }

  async function trade() {
    if (!quote || !quoteFor || starting) return;
    // The trade is the one quoted: its own shares and side, never whatever the form says now.
    if (shares !== quoteFor.shares || side !== quoteFor.side) {
      quote = null;
      quoteFor = null;
      return;
    }
    const t = quoteFor;
    starting = true;
    try {
      const f = await tradeFlowsNow().start(
        { marketId: market.id, outcome, shares: t.shares, side: t.side, limit: quote.limit },
        { title: market.title, label },
      );
      flowId = f.id;
    } catch (e) {
      error = failureText(e);
    } finally {
      starting = false;
    }
  }
</script>

<section class="stack" data-testid="trade">
  <Back title={side === 'buy' ? 'Buy' : 'Sell'} on:back={() => dispatch('back', null)} />
  <div class="card stack-sm">
    <p class="small muted">{market.title}</p>
    <div class="row">
      <strong class="grow">{label}</strong>
      <span class="num">chance {fmtChanceFine(o?.price ?? 0)}</span>
    </div>
    <p class="small muted">Each share pays 1 sat if {label} happens.</p>
  </div>

  {#if flow}
    <FlowCard {flow} on:positions={() => dispatch('home', null)} />
    {#if flow.state.k !== 'sending'}
      <button class="full" on:click={() => dispatch('home', null)}>Done</button>
    {/if}
  {:else}
    <form class="card stack" on:submit|preventDefault={() => (quote && !stale ? trade() : getPrice())}>
      {#if heldShares > 0}
        <div class="seg" role="group" aria-label="Buy or sell">
          <button type="button" class:on={side === 'buy'} aria-pressed={side === 'buy'} disabled={quoting} on:click={() => pick('buy')}
            >Buy</button
          >
          <button type="button" class:on={side === 'sell'} aria-pressed={side === 'sell'} disabled={quoting} on:click={() => pick('sell')}
            >Sell</button
          >
        </div>
      {/if}
      <div>
        <label for="shares">Shares</label>
        <input
          id="shares"
          bind:value={sharesText}
          on:input={() => ((quote = null), (quoteFor = null))}
          disabled={quoting}
          inputmode="numeric"
          autocomplete="off"
          placeholder={side === 'sell' ? `up to ${fmtShares(heldShares)}` : 'for example 1,000'}
          enterkeyhint="go"
        />
        {#if side === 'sell'}
          <p class="small muted hint">You hold {fmtShares(heldShares)} shares of {label}.</p>
        {/if}
        {#if sharesText && shares === null}
          <p class="small error hint">Type a whole number of shares.</p>
        {:else if tooMany}
          <p class="small error hint">You hold only {fmtShares(heldShares)} shares.</p>
        {/if}
      </div>

      {#if error}<p class="error small">{error}</p>{/if}

      {#if quote}
        <div class="quote stack-sm" data-testid="quote">
          {#if side === 'buy'}
            <p class="lead num">About {fmtSats(about)}, at most {fmtSats(quote.limit)} <span class="muted">(fees included)</span></p>
            <p class="small num">If {label} happens, {fmtShares(shares ?? 0)} shares pay {fmtSats(shares ?? 0)}.</p>
          {:else}
            <p class="lead num">About {fmtSats(about)} to you, at least {fmtSats(quote.limit)} <span class="muted">(after fees)</span></p>
          {/if}
          <p class="small num">
            Chance of {label}: {fmtChanceFine(quote.priceNow)} now, about {fmtChanceFine(quote.priceAfter)} after this trade.
          </p>
          <p class="small muted num">Trading fee {fmtSats(quote.fee)} · miner fee {fmtSats(quote.minerFee)}</p>
          <p class="small muted">
            A price is a guide: the trade goes through at the next Truthcoin block's price, and not at all if that is
            {side === 'buy' ? 'over your most' : 'under your least'}.
          </p>
          {#if overLimit && $status}
            <p class="small warn">
              This trade counts {fmtSats(counts)} against this phone's limit, more than the {fmtSats($status.leftSats)} left
              today, so your computer will ask you to confirm it. A buy counts at its most; a sell at its number of shares (a sat each).
            </p>
          {/if}
          {#if stale}
            <p class="small warn">This price is a few minutes old.</p>
          {/if}
        </div>
      {/if}

      {#if quote && !stale}
        <button class="primary full" type="submit" disabled={starting}>
          {quoteFor?.side === 'sell' ? 'Sell' : 'Buy'} {fmtShares(quoteFor?.shares ?? 0)} {label}
        </button>
        <button class="full" type="button" on:click={getPrice} disabled={quoting}>Get a new price</button>
      {:else}
        <button class="primary full" type="submit" disabled={quoting || shares === null || tooMany}>
          {#if quoting}<span class="spinner"></span> Asking for a price…{:else}Get a price{/if}
        </button>
      {/if}
    </form>
  {/if}
</section>

<style>
  .seg {
    display: flex;
    gap: 6px;
    padding: 4px;
    border-radius: 14px;
    background: var(--inset);
    border: 1px solid var(--border);
  }
  .seg button {
    flex: 1;
    min-height: 44px;
    border: none;
    background: transparent;
    color: var(--muted);
  }
  .seg button.on {
    background: var(--surface);
    color: var(--text);
    box-shadow: 0 0 0 1px var(--border);
  }
  .hint {
    margin-top: 6px;
  }
  .quote {
    padding: 14px;
    border-radius: 12px;
    background: var(--inset);
    border: 1px solid var(--border);
  }
  .lead {
    font-size: 18px;
    font-weight: 650;
  }
</style>
