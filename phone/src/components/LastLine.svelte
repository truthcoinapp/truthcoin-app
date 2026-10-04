<script lang="ts">
  // The last request and how it went, always on screen: asking, answered, held, or no answer (with "Ask again").
  import { onDestroy } from 'svelte';
  import { last } from '../lib/session';

  let now = Date.now();
  const t = setInterval(() => (now = Date.now()), 15_000);
  onDestroy(() => clearInterval(t));

  function ago(at: number, n: number): string {
    const s = Math.max(0, Math.round((n - at) / 1000));
    if (s < 45) return 'just now';
    const m = Math.round(s / 60);
    return m < 60 ? `${m} min ago` : new Date(at).toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' });
  }
</script>

{#if $last}
  <div class="line {$last.state}" role="status" aria-live="polite" data-testid="last-line">
    {#if $last.state === 'asking'}
      <span class="spinner small-spin" aria-hidden="true"></span>
      <span class="grow">Asking your computer{$last.what ? ` for ${$last.what}` : ''}…</span>
    {:else if $last.state === 'answered'}
      <span class="dot" aria-hidden="true"></span>
      <span class="grow">Your computer answered {ago($last.at, now)}.</span>
    {:else if $last.state === 'held'}
      <span class="dot" aria-hidden="true"></span>
      <span class="grow">Waiting for you to confirm on your computer.</span>
    {:else}
      <span class="dot" aria-hidden="true"></span>
      <span class="grow">{$last.text}</span>
      {#if $last.retry}
        <button class="link" on:click={() => $last?.retry?.()}>Ask again</button>
      {/if}
    {/if}
  </div>
{/if}

<style>
  .line {
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 14px;
    color: var(--muted);
    min-height: 44px;
    padding: 4px 12px;
    margin-bottom: 12px;
    border-radius: 10px;
    background: var(--inset);
    border: 1px solid var(--border);
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .dot {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    background: var(--muted);
    flex: none;
  }
  .answered .dot {
    background: var(--accent);
  }
  .held {
    color: var(--warn);
    border-color: var(--warn);
  }
  .held .dot {
    background: var(--warn);
  }
  .no-answer,
  .failed,
  .refused {
    color: var(--error);
    border-color: var(--error);
  }
  .no-answer .dot,
  .failed .dot,
  .refused .dot {
    background: var(--error);
  }
  .small-spin {
    width: 16px;
    height: 16px;
    border-width: 2px;
  }
  .link {
    font-size: 14px;
    min-height: 36px;
  }
</style>
