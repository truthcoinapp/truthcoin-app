<script lang="ts">
  // One market: its outcomes with their chances, what this wallet holds, and the result once decided.
  import { createEventDispatcher, onMount } from 'svelte';
  import { failureText } from '../lib/api';
  import { fmtChance, fmtSats, fmtShares, stateWord } from '../lib/format';
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
      <p class="small muted num">
        Volume {fmtSats(market.volume)} · trading fee {Math.round(market.feeRate * 1000) / 10}%
      </p>
    {/if}
  </div>

  {#if error}
    <div class="card stack-sm bad">
      <p>{error}</p>
      <button class="full" on:click={load} disabled={loading}>Ask again</button>
    </div>
  {/if}

  {#if market?.resolution}
    <div class="card stack-sm good" data-testid="resolution">
      <h3>Result</h3>
      <p>{market.resolution.summary || 'Decided.'}</p>
      <p class="small muted">Winning shares were paid out automatically; there's nothing to claim.</p>
    </div>
  {/if}

  {#if market}
    <div class="card stack" data-testid="outcomes">
      <p class="small muted">Each share pays 1 sat if its outcome happens.</p>
      {#each market.outcomes as o (o.i)}
        {@const h = held(o.i)}
        <div class="stack-sm outcome" class:won={winners.has(o.i)}>
          <div class="row">
            <strong class="grow">{o.label || `Outcome ${o.i + 1}`}{winners.has(o.i) ? ' · happened' : ''}</strong>
            <span class="chance num">{fmtChance(o.price)}</span>
          </div>
          <div class="bar" role="img" aria-label="Chance {fmtChance(o.price)}">
            <span style:width="{Math.round(o.price * 1000) / 10}%"></span>
          </div>
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
  .won .chance,
  .won strong {
    color: var(--accent);
  }
</style>
