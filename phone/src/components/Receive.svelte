<script lang="ts">
  // Where coins come in: the wallet's address, and the form an eCash deposit uses.
  import { onMount } from 'svelte';
  import { failureText } from '../lib/api';
  import { address, currentApi } from '../lib/session';
  import Copy from './Copy.svelte';

  let loading = false;
  let error = '';

  async function load() {
    loading = true;
    error = '';
    try {
      address.set(await currentApi().receive());
    } catch (e) {
      error = failureText(e);
    } finally {
      loading = false;
    }
  }
  onMount(() => {
    if (!$address) void load();
  });
</script>

<section class="stack" data-testid="receive">
  <h2>Receive</h2>
  <div class="card stack-sm">
    <p>
      Deposits from eCash are made in BitWindow, to the deposit address below (the Truthcoin App's Deposit shows it
      too): they move coins from eCash into this wallet.
    </p>
    <p class="small muted">The addresses below are for when you need them written out, to give to someone or to check.</p>
  </div>
  {#if error}
    <div class="card stack-sm bad">
      <p>{error}</p>
      <button class="full" on:click={load} disabled={loading}>Ask again</button>
    </div>
  {/if}
  {#if $address}
    <div class="card stack-sm">
      <h3>Deposit address (from eCash)</h3>
      <p class="mono addr" data-testid="deposit-address">{$address.depositAddress}</p>
      <Copy text={$address.depositAddress} />
    </div>
    <div class="card stack-sm">
      <h3>Truthcoin address</h3>
      <p class="mono addr">{$address.address}</p>
      <p class="small muted">For coins sent from another Truthcoin wallet.</p>
      <Copy text={$address.address} />
    </div>
    <button class="full" on:click={load} disabled={loading}>New address</button>
  {:else if loading}
    <p class="muted"><span class="spinner"></span> Asking your computer…</p>
  {/if}
</section>

<style>
  .addr {
    user-select: all;
    -webkit-user-select: all;
    word-break: break-all;
    padding: 10px 12px;
    border-radius: 10px;
    background: var(--inset);
    border: 1px solid var(--border);
  }
</style>
