<script lang="ts">
  // One trade sent from this phone, and where it stands. Never "Not sent" when it may have gone.
  import { createEventDispatcher } from 'svelte';
  import type { TradeFlow } from '../lib/flows';
  import { fmtSats, fmtShares, MINER_FEE_SATS, shortId } from '../lib/format';
  import { tradeFlowsNow, trades } from '../lib/session';

  export let flow: TradeFlow;
  const dispatch = createEventDispatcher<{ positions: null }>();

  $: s = flow.state;
  $: what = `${flow.side === 'buy' ? 'Buy' : 'Sell'} ${fmtShares(flow.shares)} ${flow.label}`;
  $: cap = flow.side === 'buy' ? `at most ${fmtSats(flow.limit)}` : `at least ${fmtSats(flow.limit)}`;
  $: final = s.k === 'pending' || s.k === 'refused';
  // Once sent, the desktop's trade list says how it went (the same id): the card follows it, in place.
  $: record = s.k === 'pending' ? ($trades ?? []).find((t) => t.id === flow.id) ?? null : null;
  $: done = record?.status === 'done';
  $: failed = !!record && ['failed', 'cancelled', 'dropped'].includes(record.status);
  // What it cost or brought, the miner fee counted as the quote did.
  $: amount =
    record && record.sats !== null
      ? flow.side === 'buy'
        ? record.sats + MINER_FEE_SATS
        : Math.max(0, record.sats - MINER_FEE_SATS)
      : null;
  $: doneText = `Done: ${flow.side === 'buy' ? 'bought' : 'sold'} ${fmtShares(flow.shares)} ${flow.label}${
    amount !== null ? ` for ${fmtSats(amount)}` : ''
  }.`;
  const HEADLINE = 'Not confirmed: check Positions before trying again.';
  // The computer's own words when they add something to the headline.
  $: why = s.k === 'unconfirmed' && s.why.replace(/\.?$/, '.') !== HEADLINE ? s.why : '';
</script>

<div
  class="card stack-sm"
  class:held={s.k === 'held'}
  class:bad={s.k === 'unconfirmed' || s.k === 'refused' || failed}
  class:good={s.k === 'pending' && !failed}
  data-testid="flow"
  data-state={done ? 'done' : s.k}
>
  <div class="row">
    <strong class="grow">{what}</strong>
    <span class="small muted num">{cap}</span>
  </div>
  <p class="small muted">{flow.title}</p>
  {#if s.k === 'sending'}
    <p><span class="spinner"></span> Sending to your computer…</p>
  {:else if s.k === 'held'}
    <p class="warn"><strong>Waiting for your OK on the computer (over today's limit).</strong></p>
  {:else if s.k === 'pending' && done}
    <p data-testid="flow-done"><strong>{doneText}</strong></p>
    {#if s.txid}<p class="small muted mono">Transaction {shortId(s.txid)}</p>{/if}
  {:else if s.k === 'pending' && failed && record}
    <p><strong>Didn't go through</strong> (the trade was {record.status}). Nothing was {flow.side === 'buy' ? 'bought' : 'sold'}.</p>
  {:else if s.k === 'pending'}
    <p>
      <strong>Sent to your computer's node.</strong> It trades with the next Truthcoin block (about 10–17 minutes) if
      the price is still within your limit. Recent trades on Home shows how it went.
    </p>
    {#if s.txid}<p class="small muted mono">Transaction {shortId(s.txid)}</p>{/if}
  {:else if s.k === 'refused'}
    <p><strong>Not done.</strong> {s.msg}</p>
  {:else if s.k === 'unconfirmed'}
    <p class="error"><strong>{HEADLINE}</strong></p>
    <p class="small muted">{why ? `${why} ` : ''}Asking again is safe: it can't trade twice.</p>
    <div class="buttons">
      <button on:click={() => tradeFlowsNow().askAgain(flow.id)}>Ask again</button>
      <button on:click={() => dispatch('positions', null)}>Positions</button>
    </div>
  {/if}
  {#if final}
    <div class="right"><button class="link" on:click={() => tradeFlowsNow().dismiss(flow.id)}>Dismiss</button></div>
  {/if}
</div>

<style>
  .right {
    text-align: right;
  }
</style>
