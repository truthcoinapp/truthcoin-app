<script lang="ts">
  // Home: trades in flight, the balance, the computer's state and this phone's limit, positions, recent trades.
  import { createEventDispatcher, onMount } from 'svelte';
  import {
    fmtChance,
    fmtHeight,
    fmtNum,
    fmtSats,
    fmtShares,
    fmtTime,
    limitSentence,
    MINER_FEE_SATS,
    settledLine,
    stateWord,
  } from '../lib/format';
  import { balance, flows, HOME_FRESH_MS, homeOnScreen, positions, reach, refreshHome, relays, status, trades } from '../lib/session';
  import { clock, reachState } from '../lib/reach';
  import type { Status } from '../lib/validate';
  import FlowCard from './FlowCard.svelte';
  import HomeScreenHint from './HomeScreenHint.svelte';

  const dispatch = createEventDispatcher<{ market: { id: string; title: string } }>();
  let refreshing = false;
  let positionsEl: HTMLElement;

  /** The Refresh button always asks; showing Home reuses data from the last few seconds. */
  async function refresh(maxAgeMs?: number) {
    if (refreshing) return;
    refreshing = true;
    try {
      await refreshHome({ maxAgeMs });
    } finally {
      refreshing = false;
    }
  }

  // Fresh data whenever Home is shown, and every 30 s while it stays on screen (the page in front).
  onMount(() => {
    homeOnScreen(true);
    void refresh(HOME_FRESH_MS);
    const poll = setInterval(() => {
      if (!document.hidden) void refreshHome({ trades: false });
    }, 30_000);
    return () => {
      clearInterval(poll);
      homeOnScreen(false);
    };
  });

  // While the computer isn't answering, what Home shows is what it last said: each card says when that was.
  let now = Date.now();
  onMount(() => {
    const t = setInterval(() => (now = Date.now()), 30_000);
    return () => clearInterval(t);
  });
  $: rs = reachState($reach, $relays.some((r) => r.state === 'open'), now);
  $: asOf = rs !== 'answering' && $reach.lastAnswer !== null ? `as of ${clock($reach.lastAnswer, now)}` : '';

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

  // Trade cards: every one still on its way, and of the finished ones only the newest, for an hour.
  const HOUR_MS = 3_600_000;
  const FINISHED = new Set(['done', 'failed', 'cancelled', 'dropped']);
  $: shownFlows = (() => {
    let keptOne = false;
    return $flows.filter((f) => {
      const record = ($trades ?? []).find((t) => t.id === f.id);
      const finished = f.state.k === 'refused' || (f.state.k === 'pending' && !!record && FINISHED.has(record.status));
      if (!finished) return true;
      if (keptOne || Date.now() - f.at > HOUR_MS) return false;
      keptOne = true; // flows are newest first
      return true;
    });
  })();

  /** Trades that didn't go through: no amount was paid or got. */
  const GONE = new Set(['failed', 'cancelled', 'dropped']);
  /** What it cost (buy) or brought (sell), the miner fee counted as the quote does. */
  const withMinerFee = (t: { side: string; sats: number | null }) =>
    t.side === 'buy' ? (t.sats ?? 0) + MINER_FEE_SATS : Math.max(0, (t.sats ?? 0) - MINER_FEE_SATS);

  const TRADE_WORDS: Record<string, string> = {
    sending: 'Sending',
    pending: 'Pending',
    done: 'Done',
    failed: 'Failed',
    held: 'Waiting for your OK',
    refused: 'Refused',
    cancelled: 'Cancelled',
    dropped: "Didn't go through",
  };
</script>

<section class="stack" data-testid="home">
  {#each shownFlows as f (f.id)}
    <FlowCard flow={f} on:positions={() => positionsEl?.scrollIntoView({ behavior: 'smooth' })} />
  {/each}

  <div class="card stack-sm" data-testid="balance">
    <div class="row"><h3 class="grow">Balance</h3>{#if asOf && $balance}<span class="small muted" data-testid="as-of">{asOf}</span>{/if}</div>
    {#if $balance}
      <!-- What the wallet holds once what's moving settles; under it, only what isn't zero. -->
      <p class="big num">{fmtSats($balance.total)}</p>
      {#if $balance.inPendingTrades > 0 || $balance.withdrawing > 0}
        <dl class="facts">
          {#if $balance.inPendingTrades > 0}
            <dt>Ready to use now</dt>
            <dd>{fmtSats($balance.available)}</dd>
            <dt>Held by {$balance.pendingTrades} waiting trade{$balance.pendingTrades === 1 ? '' : 's'}</dt>
            <dd>{fmtSats($balance.inPendingTrades)}</dd>
          {/if}
          {#if $balance.withdrawing > 0}
            <dt>On its way to eCash</dt>
            <dd>{fmtSats($balance.withdrawing)} <span class="muted">(pays out in days)</span></dd>
          {/if}
        </dl>
      {/if}
    {:else}
      <p class="muted">—</p>
    {/if}
  </div>

  <div class="card stack-sm" data-testid="computer">
    <h3>Your computer</h3>
    {#if rs === 'silent'}
      <p class="warn" data-testid="computer-silent">
        Not answering since {clock($reach.silentSince ?? now, now)}{$reach.lastAnswer !== null &&
        clock($reach.lastAnswer, now) !== clock($reach.silentSince ?? now, now)
          ? `; last heard from at ${clock($reach.lastAnswer, now)}`
          : ''}. Is the Truthcoin App open on it?
      </p>
    {:else if rs === 'offline'}
      <p class="warn">This phone can't reach the relays, so it can't ask.</p>
    {/if}
    {#if $status}
      {#if rs !== 'silent' && rs !== 'offline'}
        <p class:warn={$status.node !== 'running' || !$status.synced}>{nodeLine($status)}</p>
      {/if}
      <dl class="facts">
        <dt>Left today for trades</dt>
        <dd>{fmtNum($status.leftSats)} of {fmtSats($status.limitSats)}</dd>
      </dl>
      <p class="small muted">{limitSentence($status.limitSats)}</p>
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
    {#if asOf && $positions}<p class="small muted">{asOf}</p>{/if}
    {#if !$positions}
      <p class="muted">—</p>
    {:else if !$positions.positions.length}
      <p class="muted">{$positions.settled.length ? 'No open positions.' : 'No shares yet. Find a market under Markets.'}</p>
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

  {#if $positions && $positions.settled.length}
    <div class="card stack-sm" data-testid="settled">
      <h3>Settled</h3>
      <ul class="list">
        {#each $positions.settled as s (s.marketId)}
          <li>
            <button class="tap" on:click={() => dispatch('market', { id: s.marketId, title: s.title })}>
              <div class="small muted">{s.title}</div>
              <div class="num">{settledLine(s)}</div>
            </button>
          </li>
        {/each}
      </ul>
    </div>
  {/if}

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
              {fmtTime(t.time)}{t.sats !== null && !GONE.has(t.status) ? ` · about ${fmtSats(withMinerFee(t))}` : ''} ·
              {t.side === 'buy' ? 'at most' : 'at least'}
              {fmtSats(t.limit)}
            </div>
          </li>
        {/each}
      </ul>
    </div>
  {/if}

  <button class="full" on:click={() => refresh()} disabled={refreshing}>
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
