<script lang="ts">
  // Home: trades in flight, the balance, the computer's state and this phone's limit, positions, recent trades.
  import { createEventDispatcher, onMount } from 'svelte';
  import { fmtChance, fmtHeight, fmtNum, fmtSats, fmtShares, fmtTime, stateWord } from '../lib/format';
  import { balance, flows, homeOnScreen, positions, refreshHome, status, trades } from '../lib/session';
  import type { Status } from '../lib/validate';
  import FlowCard from './FlowCard.svelte';
  import HomeScreenHint from './HomeScreenHint.svelte';

  const dispatch = createEventDispatcher<{ market: { id: string; title: string } }>();
  let refreshing = false;
  let positionsEl: HTMLElement;

  async function refresh() {
    if (refreshing) return;
    refreshing = true;
    try {
      await refreshHome();
    } finally {
      refreshing = false;
    }
  }

  // Fresh data whenever Home is shown, and every 30 s while it stays on screen (the page in front).
  onMount(() => {
    homeOnScreen(true);
    void refresh();
    const poll = setInterval(() => {
      if (!document.hidden) void refreshHome({ trades: false });
    }, 30_000);
    return () => {
      clearInterval(poll);
      homeOnScreen(false);
    };
  });

  function nodeLine(s: Status): string {
    const h = s.height === null ? '' : ` · block ${fmtHeight(s.height)}`;
    switch (s.node) {
      case 'running':
        return s.synced ? `Truthcoin is running${h}` : `Truthcoin is catching up${h}`;
      case 'starting':
        return 'Truthcoin is starting on your computer';
      case 'stopped':
        return "Truthcoin isn't running on your computer";
      default:
        return 'Truthcoin failed to start on your computer';
    }
  }

  const TRADE_WORDS: Record<string, string> = {
    sending: 'Sending',
    pending: 'Pending',
    done: 'Done',
    failed: 'Failed',
    held: 'Waiting for you',
    refused: 'Refused',
  };
</script>

<section class="stack" data-testid="home">
  {#each $flows as f (f.id)}
    <FlowCard flow={f} on:positions={() => positionsEl?.scrollIntoView({ behavior: 'smooth' })} />
  {/each}

  <div class="card stack-sm" data-testid="balance">
    <h3>Balance</h3>
    {#if $balance}
      <p class="big num">{fmtSats($balance.total)}</p>
      <dl class="facts">
        <dt>Available</dt>
        <dd>{fmtSats($balance.available)}</dd>
        {#if $balance.pendingTrades > 0 || $balance.inPendingTrades > 0}
          <dt>In pending trades</dt>
          <dd>{fmtSats($balance.inPendingTrades)} ({$balance.pendingTrades})</dd>
        {/if}
      </dl>
      {#if $balance.pendingTrades > 0}
        <p class="small muted">A pending trade ties up a whole coin until its block; it comes back then.</p>
      {/if}
    {:else}
      <p class="muted">—</p>
    {/if}
  </div>

  <div class="card stack-sm" data-testid="computer">
    <h3>Your computer</h3>
    {#if $status}
      <p class:warn={$status.node !== 'running' || !$status.synced}>{nodeLine($status)}</p>
      <dl class="facts">
        <dt>Left today for trades</dt>
        <dd>{fmtNum($status.leftSats)} of {fmtSats($status.limitSats)}</dd>
      </dl>
      <p class="small muted">
        Trades over what's left wait for you to confirm them on your computer. A buy counts at its most; a sell at its number of shares (a sat each).
      </p>
    {:else}
      <p class="muted">—</p>
    {/if}
  </div>

  <div class="card stack-sm" bind:this={positionsEl} data-testid="positions">
    <div class="row">
      <h3 class="grow">Positions</h3>
      {#if $positions && $positions.positions.length}
        <span class="small muted num">worth about {fmtSats($positions.totalValue)}</span>
      {/if}
    </div>
    {#if !$positions}
      <p class="muted">—</p>
    {:else if !$positions.positions.length}
      <p class="muted">No shares yet. Find a market under Markets.</p>
    {:else}
      <ul class="list">
        {#each $positions.positions as p (p.marketId + ':' + p.outcome)}
          <li>
            <button class="tap" on:click={() => dispatch('market', { id: p.marketId, title: p.title })}>
              <div class="row">
                <span class="grow"><strong>{p.label}</strong> · {fmtShares(p.shares)} shares</span>
                <span class="num">{fmtSats(p.value)}</span>
              </div>
              <div class="small muted">{p.title}</div>
              <div class="small muted num">
                Chance {fmtChance(p.price)}{p.paid !== null ? ` · paid ${fmtSats(p.paid)}` : ''}{p.state !== 'trading'
                  ? ` · ${stateWord(p.state)}`
                  : ''}
              </div>
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </div>

  {#if $trades && $trades.length}
    <div class="card stack-sm" data-testid="recent">
      <h3>Recent trades from this phone</h3>
      <ul class="list">
        {#each $trades as t (t.id)}
          <li class="small">
            <div class="row">
              <span class="grow">{t.side === 'buy' ? 'Buy' : 'Sell'} {fmtShares(t.shares)} {t.label}</span>
              <span class="muted">{TRADE_WORDS[t.status] ?? stateWord(t.status)}</span>
            </div>
            <div class="muted">{t.title}</div>
            <div class="muted num">
              {fmtTime(t.time)}{t.sats !== null ? ` · about ${fmtSats(t.sats)}` : ''} · {t.side === 'buy' ? 'at most' : 'at least'}
              {fmtSats(t.limit)}
            </div>
          </li>
        {/each}
      </ul>
    </div>
  {/if}

  <button class="full" on:click={refresh} disabled={refreshing}>
    {#if refreshing}<span class="spinner"></span>{:else}Refresh{/if}
  </button>
  <HomeScreenHint paired />
</section>

<style>
  .big {
    font-size: 32px;
    font-weight: 700;
    letter-spacing: -0.01em;
    line-height: 1.15;
  }
</style>
