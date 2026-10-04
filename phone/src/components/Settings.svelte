<script lang="ts">
  // This phone and the computer it's paired with; the relays; forgetting the computer.
  import { createEventDispatcher, onMount } from 'svelte';
  import { fmtDate, fmtSats, keyFingerprint, limitSentence } from '../lib/format';
  import { relayHost } from '../lib/relays';
  import { pairing, refreshStatus, relays, status } from '../lib/session';

  const dispatch = createEventDispatcher<{ forget: null; repair: null }>();
  let confirming = false;

  const STATE_WORDS = { open: 'connected', connecting: 'connecting…', waiting: 'not connected' } as const;
  const version = __APP_VERSION__;
  // The limit and the relays as the computer has them now (they change there).
  onMount(() => void refreshStatus().catch(() => undefined));
</script>

<section class="stack" data-testid="settings">
  <h2>Settings</h2>
  {#if $pairing}
    <div class="card stack-sm">
      <h3>This phone</h3>
      <dl class="facts">
        <dt>Name</dt>
        <dd>{$pairing.name}</dd>
        <dt>Limit for trades</dt>
        <dd>{fmtSats($status?.limitSats ?? $pairing.limitSats)} a day</dd>
        <dt>Paired</dt>
        <dd>{fmtDate(new Date($pairing.pairedAt))}</dd>
      </dl>
      <p class="small muted">{limitSentence($status?.limitSats ?? $pairing.limitSats)}</p>
      <p class="small muted">You change the limit on your computer, in its Phone tab.</p>
    </div>

    <div class="card stack-sm">
      <h3>Your computer</h3>
      <dl class="facts">
        <dt>Key</dt>
        <dd class="mono" data-testid="fingerprint">{keyFingerprint($pairing.d)}</dd>
        {#if $status}
          <dt>App</dt>
          <dd>{$status.app}</dd>
          <dt>Network</dt>
          <dd>{$status.network}</dd>
        {/if}
      </dl>
    </div>

    <div class="card stack-sm">
      <h3>Relays</h3>
      <ul class="list">
        {#each $relays as r (r.url)}
          <li class="row small">
            <span class="grow mono">{relayHost(r.url)}</span>
            <span class="state {r.state}">{STATE_WORDS[r.state]}</span>
          </li>
        {/each}
      </ul>
      <p class="small muted">
        Public Nostr relays carry the messages, sealed so only your computer and this phone can read them. One working
        relay is enough. Your computer chooses them; this phone follows.
      </p>
    </div>

    <div class="card stack-sm">
      <h3>Pairing</h3>
      <button class="full" on:click={() => dispatch('repair', null)}>Pair with a computer again</button>
      {#if !confirming}
        <button class="danger full" on:click={() => (confirming = true)}>Forget this computer</button>
      {:else}
        <p class="warn">
          This deletes this phone's keys here and asks your computer to forget this phone too. To use it again, you pair
          it again.
        </p>
        <div class="buttons">
          <button on:click={() => (confirming = false)}>Keep</button>
          <button class="danger" on:click={() => dispatch('forget', null)} data-testid="forget-confirm">Forget</button>
        </div>
      {/if}
    </div>
  {/if}
  <p class="small muted center">Truthcoin App phone page · v{version}</p>
</section>

<style>
  .state.open {
    color: var(--accent);
  }
  .state.waiting {
    color: var(--error);
  }
  .state.connecting {
    color: var(--warn);
  }
  .center {
    text-align: center;
  }
</style>
