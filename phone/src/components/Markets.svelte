<script context="module" lang="ts">
  /** When the list last came in (ms): showing Markets again within 10 s reuses it (the Refresh button always asks). */
  let loadedAt = 0;
</script>

<script lang="ts">
  // The markets, trading first, newest first, a page at a time.
  import { createEventDispatcher, onMount } from 'svelte';
  import { failureText } from '../lib/api';
  import { fmtChance, fmtSats, stateWord } from '../lib/format';
  import { currentApi, markets } from '../lib/session';
  import type { MarketSummary } from '../lib/validate';

  const dispatch = createEventDispatcher<{ open: MarketSummary }>();
  let loading = false;
  let error = '';

  async function load(page: number) {
    loading = true;
    error = '';
    try {
      markets.set(await currentApi().markets(page));
      loadedAt = Date.now();
    } catch (e) {
      error = failureText(e);
    } finally {
      loading = false;
    }
  }

  // The list as it is now, whenever Markets is shown (the last one shows meanwhile).
  onMount(() => {
    if (!$markets || Date.now() - loadedAt > 10_000) void load($markets?.page ?? 0);
  });
</script>

<section class="stack" data-testid="markets">
  <div class="row">
    <h2 class="grow">Markets</h2>
    <button class="plain refresh" on:click={() => load($markets?.page ?? 0)} disabled={loading} aria-label="Refresh">
      {#if loading}<span class="spinner"></span>{:else}↻{/if}
    </button>
  </div>
  {#if error}<p class="error small">{error}</p>{/if}
  {#if $markets}
    {#if !$markets.markets.length}
      <p class="muted">No markets yet.</p>
    {:else}
      <ul class="list card">
        {#each $markets.markets as m (m.id)}
          <li>
            <button class="tap" on:click={() => dispatch('open', m)}>
              <div class="row">
                <strong class="grow">{m.title || 'Untitled market'}</strong>
                <span class="badge {m.state}">{stateWord(m.state)}</span>
              </div>
              <div class="small muted num">
                {#if m.leading}<strong class="lead">{m.leading.label || 'Leading'} {fmtChance(m.leading.price)}</strong> ·
                {/if}{m.outcomes} outcomes · volume {fmtSats(m.volume)}
              </div>
            </button>
          </li>
        {/each}
      </ul>
    {/if}
    {#if $markets.pages > 1}
      <div class="buttons">
        <button disabled={loading || $markets.page <= 0} on:click={() => $markets && load($markets.page - 1)}>Previous</button>
        <button disabled={loading || $markets.page >= $markets.pages - 1} on:click={() => $markets && load($markets.page + 1)}
          >Next</button
        >
      </div>
      <p class="small muted center">Page {$markets.page + 1} of {$markets.pages}</p>
    {/if}
  {:else if loading}
    <p class="muted"><span class="spinner"></span> Asking your computer…</p>
  {/if}
</section>

<style>
  .lead {
    color: var(--text);
    font-weight: 650;
  }
  .refresh {
    font-size: 22px;
    min-width: 48px;
    color: var(--accent);
  }
  .center {
    text-align: center;
  }
  ul.card {
    padding: 14px 16px;
  }
</style>
