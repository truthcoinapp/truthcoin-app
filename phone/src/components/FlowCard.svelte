<script lang="ts">
  // One trade sent from this phone, and where it stands. Never "Not sent" when it may have gone.
  import { createEventDispatcher } from 'svelte';
  import type { TradeFlow } from '../lib/flows';
  import { fmtSats, fmtShares, shortId } from '../lib/format';
  import { tradeFlowsNow } from '../lib/session';

  export let flow: TradeFlow;
  const dispatch = createEventDispatcher<{ positions: null }>();

  $: s = flow.state;
  $: what = `${flow.side === 'buy' ? 'Buy' : 'Sell'} ${fmtShares(flow.shares)} ${flow.label}`;
  $: cap = flow.side === 'buy' ? `at most ${fmtSats(flow.limit)}` : `at least ${fmtSats(flow.limit)}`;
  $: final = s.k === 'pending' || s.k === 'refused';
  const HEADLINE = 'Not confirmed: check Positions before trying again.';
  // The computer's own words when they add something to the headline.
  $: why = s.k === 'unconfirmed' && s.why.replace(/\.?$/, '.') !== HEADLINE ? s.why : '';
</script>

<div
  class="card stack-sm"
  class:held={s.k === 'held'}
  class:bad={s.k === 'unconfirmed' || s.k === 'refused'}
  class:good={s.k === 'pending'}
  data-testid="flow"
  data-state={s.k}
>
  <div class="row">
    <strong class="grow">{what}</strong>
    <span class="small muted num">{cap}</span>
  </div>
  <p class="small muted">{flow.title}</p>
  {#if s.k === 'sending'}
    <p><span class="spinner"></span> Sending to your computer…</p>
  {:else if s.k === 'held'}
    <p class="warn"><strong>Waiting for you to confirm on your computer.</strong></p>
    {#if s.text}<p class="small">{s.text}</p>{/if}
    <p class="small muted">It's over this phone's limit for today, so the Truthcoin App on your computer asks first.</p>
  {:else if s.k === 'pending'}
    <p><strong>Sent.</strong> It goes through with the next Truthcoin block (about 10–17 minutes).</p>
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
