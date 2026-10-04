<script lang="ts">
  // One market: its outcomes with their chances, what this wallet holds, and the result once decided.
  import { createEventDispatcher, onMount } from 'svelte';
  import { failureText } from '../lib/api';
  import { fmtChance, fmtPaid, fmtSats, fmtShares, stateWord } from '../lib/format';
  import { currentApi } from '../lib/session';
  import type { Market, Side } from '../lib/validate';
  import Back from './Back.svelte';

  export let id: string;
  /** What the list said, shown while the market loads (and its state, if the market's own record lacks one). */
  export let hint: { title: string; state?: string } | null = null;

  const dispatch = createEventDispatcher<{ back: null; trade: { market: Market; outcome: number; side: Side } }>();
  let market: Market | null = null;
  let loading = false;
  let error = '';
  let more = false;

  async function load() {
    loading = true;
    error = '';
    try {
      market = await currentApi().market(id);
    } catch (e) {
      error = failureText(e);
    } finally {
      loading = false;
    }
  }
  onMount(load);

  $: state = market?.state || hint?.state || '';
  $: trading = state === 'trading';
  $: held = (i: number) => market?.holdings.find((h) => h.outcome === i) ?? null;
  $: winners = new Set(market?.resolution?.winners ?? []);
  $: longText = (market?.description.length ?? 0) > 220;
  $: settled = !!market?.resolution;
  const labelOf = (o: { i: number; label: string }) => o.label || `Outcome ${o.i + 1}`;
  /** What each share of outcome `o` paid at settlement: 1 sat for a sole winner; with several, each winner's final
   * price; nothing for the rest. */
  const paidOf = (m: Market, o: { i: number; price: number }) => {
    const w = m.resolution?.winners ?? [];
    if (!w.includes(o.i)) return 0;
    return w.length === 1 ? 1 : o.price;
  };
  /** "Settled: Yes. Each Yes share paid 1 sat; No paid nothing." (or, with several winners, what each paid). */
  function settledText(m: Market): string {
    const won = m.outcomes.filter((o) => winners.has(o.i));
    const lost = m.outcomes.filter((o) => !winners.has(o.i));
    const lostText = lost.length === 1 ? `${labelOf(lost[0])} paid nothing` : lost.length ? 'the others paid nothing' : '';
    if (won.length === 1) {
      const w = labelOf(won[0]);
      return `Settled: ${w}. Each ${w} share paid 1 sat${lostText ? `; ${lostText}` : ''}.`;
    }
    if (won.length > 1) {
      const each = won.map((o) => `each ${labelOf(o)} share paid ${fmtPaid(paidOf(m, o))}`).join(', ');
      return `Voters didn't settle on one answer, so ${each}${lostText ? `; ${lostText}` : ''}.`;
    }
    return 'Settled: no outcome paid.';
  }
</script>

<section class="stack" data-testid="market">
  <Back title="Market" on:back={() => dispatch('back', null)} />
  <div class="card stack-sm">
    <div class="row">
      <h2 class="grow title">{market?.title || hint?.title || 'Market'}</h2>
      {#if state}<span class="badge {state}">{stateWord(state)}</span>{/if}
    </div>
    {#if market}
      {#if market.description}
        <p class="small desc" class:clamp={longText && !more}>{market.description}</p>
        {#if longText}
          <button class="link small" on:click={() => (more = !more)}>{more ? 'Less' : 'More'}</button>
        {/if}
      {/if}
      <p class="small muted num" data-testid="market-fees">
        Volume {fmtSats(market.volume)} · fee {Math.round(market.feeRate * 1000) / 10}% (at least 1,000 sats a trade) + 1,000
        sats to the miner
      </p>
    {/if}
  </div>

  {#if error}
    <div class="card stack-sm bad">
      <p>{error}</p>
      <button class="full" on:click={load} disabled={loading}>Ask again</button>
    </div>
  {/if}

  {#if market && settled}
    <div class="card stack-sm good" data-testid="resolution">
      <h3>Settled</h3>
      <p>{settledText(market)}</p>
      <p class="small muted">Shares were paid out automatically; there's nothing to claim.</p>
    </div>
  {/if}

  {#if market}
    <div class="card stack" data-testid="outcomes">
      {#if !settled}<p class="small muted">Each share pays 1 sat if its outcome happens.</p>{/if}
      {#each market.outcomes as o (o.i)}
        {@const h = held(o.i)}
        <div class="stack-sm outcome" class:won={winners.has(o.i)}>
          <div class="row">
            <strong class="grow">{labelOf(o)}</strong>
            {#if settled}
              <span class="paid num">{paidOf(market, o) > 0 ? `paid ${fmtPaid(paidOf(market, o))} a share` : 'paid nothing'}</span>
            {:else}
              <span class="chance num">{fmtChance(o.price)}</span>
            {/if}
          </div>
          {#if !settled}
            <div class="bar" role="img" aria-label="Chance {fmtChance(o.price)}">
              <span style:width="{Math.round(o.price * 1000) / 10}%"></span>
            </div>
          {/if}
          {#if h && h.shares > 0}
            <p class="small num">You hold {fmtShares(h.shares)} shares, worth about {fmtSats(h.value)}.</p>
          {/if}
          {#if trading}
            <div class="buttons">
              <button class="primary" on:click={() => market && dispatch('trade', { market, outcome: o.i, side: 'buy' })}
                >Buy</button
              >
              {#if h && h.shares > 0}
                <button on:click={() => market && dispatch('trade', { market, outcome: o.i, side: 'sell' })}>Sell</button>
              {/if}
            </div>
          {/if}
        </div>
      {/each}
      {#if !trading && !market.resolution}
        <p class="small muted">This market isn't trading{state ? ` (${stateWord(state).toLowerCase()})` : ''}.</p>
      {/if}
    </div>
  {:else if loading}
    <p class="muted"><span class="spinner"></span> Asking your computer…</p>
  {/if}
</section>

<style>
  .title {
    font-size: 20px;
  }
  .desc {
    white-space: pre-line;
  }
  .clamp {
    display: -webkit-box;
    -webkit-line-clamp: 4;
    line-clamp: 4;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .chance {
    font-size: 20px;
    font-weight: 700;
  }
  .outcome + .outcome {
    border-top: 1px solid var(--border);
    padding-top: 14px;
  }
  .won .paid,
  .won strong {
    color: var(--accent);
  }
  .paid {
    font-weight: 650;
  }
</style>
