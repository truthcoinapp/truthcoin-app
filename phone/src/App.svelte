<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { takePairFragment } from './lib/fragment';
  import type { Pairing } from './lib/pairing';
  import {
    announce,
    forget,
    kick,
    onOtherTab,
    pairing,
    refreshHome,
    relays,
    startSession,
    stopSession,
    storage,
  } from './lib/session';
  import { attemptFor, type PairAttempt } from './lib/pairing';
  import { dropAttempt, dropOtherPending, loadAttempt, loadPairing, savePairing } from './lib/store';
  import type { Market as MarketData, MarketSummary, Side } from './lib/validate';
  import Home from './components/Home.svelte';
  import LastLine from './components/LastLine.svelte';
  import Mark from './components/Mark.svelte';
  import Market from './components/Market.svelte';
  import Markets from './components/Markets.svelte';
  import Pair from './components/Pair.svelte';
  import Receive from './components/Receive.svelte';
  import Settings from './components/Settings.svelte';
  import Trade from './components/Trade.svelte';

  export let pairValue: string | null = null;

  type Tab = 'home' | 'markets' | 'receive' | 'settings';
  type Screen = 'loading' | 'pair' | Tab | 'market' | 'trade';
  let screen: Screen = 'loading';
  let tab: Tab = 'home';
  let loadError = '';
  let marketId = '';
  let marketHint: { title: string; state?: string } | null = null;
  let tradeOf: { market: MarketData; outcome: number; side: Side } | null = null;
  let resume: PairAttempt | null = null;
  let notice = '';
  let forgetting = false;
  let unsubTabs: (() => void) | null = null;

  onMount(async () => {
    // Another tab or the installed app paired or forgot: this one's state is stale, so start over from storage.
    unsubTabs = onOtherTab(() => location.reload());
    let p: Pairing | null = null;
    let a: PairAttempt | null = null;
    try {
      p = await loadPairing(storage());
      a = await loadAttempt(storage());
    } catch (e) {
      loadError = "This browser won't let the page keep its key (private browsing?), so it can't stay paired.";
    }
    if (p) startSession(p);
    // A pairing that was under way when the page reloaded (the #pair= link is gone by then): offered, never carried on
    // without asking.
    resume = a && !pairValue ? attemptFor(a, a.link, Math.floor(Date.now() / 1000)) : null;
    if (a && !resume && !pairValue) void dropAttempt(storage()).catch(() => undefined);
    screen = pairValue || !p || resume ? 'pair' : 'home';
    history.replaceState({ screen: screen === 'pair' ? 'pair' : 'home' }, '');
  });
  onDestroy(() => {
    unsubTabs?.();
    stopSession();
  });

  // Tabs replace each other; a market and a trade are pages on top (the phone's back gesture goes back).
  function go(s: Screen, push = false) {
    if (s === 'home' || s === 'markets' || s === 'receive' || s === 'settings') tab = s;
    screen = s;
    if (push) history.pushState({ screen: s, marketId, marketHint }, '');
    else history.replaceState({ screen: s, marketId, marketHint }, '');
    window.scrollTo(0, 0);
  }

  function onPop(e: PopStateEvent) {
    if (!$pairing) return;
    const st = e.state as { screen?: Screen; marketId?: string; marketHint?: typeof marketHint } | null;
    let s = st?.screen ?? 'home';
    if (s === 'trade' && !tradeOf) s = st?.marketId ? 'market' : 'markets'; // a trade's page needs its market
    if (s === 'market') {
      if (!st?.marketId) s = 'markets';
      else {
        marketId = st.marketId;
        marketHint = st.marketHint ?? null;
      }
    }
    if (s === 'pair' || s === 'loading') s = 'home';
    if (s === 'home' || s === 'markets' || s === 'receive' || s === 'settings') tab = s;
    screen = s;
  }

  function openMarket(id: string, hint: { title: string; state?: string } | null) {
    marketId = id;
    marketHint = hint;
    go('market', true);
  }

  function openTrade(e: CustomEvent<{ market: MarketData; outcome: number; side: Side }>) {
    tradeOf = e.detail;
    go('trade', true);
  }

  // A pairing link opened while the page is open (only the hash changes).
  function onHash() {
    const v = takePairFragment();
    if (!v) return;
    resume = null;
    pairValue = v;
    screen = 'pair';
  }

  function onVisible() {
    if (document.visibilityState === 'visible') kick();
  }

  async function onPaired(e: CustomEvent<Pairing>) {
    const p = e.detail;
    stopSession(); // first: the old session writes nothing from here on
    try {
      await dropOtherPending(storage(), p.npub); // earlier pairings' waiting trades go with them
      await savePairing(storage(), p);
      await dropAttempt(storage());
      loadError = '';
    } catch {
      loadError = "This browser won't let the page keep its key (private browsing?): this pairing lasts only until the page closes.";
    }
    announce('paired');
    notice = '';
    pairValue = null;
    resume = null;
    startSession(p);
    go('home');
    void refreshHome();
  }

  function cancelPair() {
    pairValue = null;
    resume = null;
    go($pairing ? 'home' : 'pair');
  }

  async function doForget() {
    forgetting = true;
    try {
      const r = await forget();
      loadError = '';
      notice = r.told
        ? ''
        : "Probably done: your computer usually forgets this phone at once, but its answer didn't come back. Check the Phone tab on your computer to be sure.";
    } catch (e) {
      loadError = `This phone's keys may still be here: ${(e as Error).message}`;
    }
    forgetting = false;
    tradeOf = null;
    go('pair');
  }

  const TABS: { id: Tab; label: string; icon: string }[] = [
    { id: 'home', label: 'Home', icon: 'M4 11l8-7 8 7v9h-5v-6H9v6H4z' },
    { id: 'markets', label: 'Markets', icon: 'M5 19V11M12 19V5M19 19v-9' },
    { id: 'receive', label: 'Receive', icon: 'M12 4v12M7 11l5 5 5-5M5 20h14' },
    { id: 'settings', label: 'Settings', icon: 'M12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6zM4 12h2M18 12h2M12 4v2M12 18v2' },
  ];

  $: online = $relays.some((r) => r.state === 'open');
</script>

<svelte:window on:popstate={onPop} on:hashchange={onHash} on:online={kick} />
<svelte:document on:visibilitychange={onVisible} />

<!-- The header and the last request's line stay in view while the page scrolls. -->
<div class="top">
<header>
  <div class="brand"><Mark size={28} /><span>Truthcoin</span></div>
  {#if $pairing && screen !== 'pair'}
    <span class="conn" class:on={online} data-testid="conn">
      <span class="dot" aria-hidden="true"></span>{online ? 'Connected' : 'Connecting…'}
    </span>
  {/if}
</header>
{#if $pairing && !forgetting && screen !== 'pair' && screen !== 'loading'}
  <LastLine />
{/if}
</div>

{#if loadError}<p class="card bad small notice" data-testid="load-error">{loadError}</p>{/if}
{#if notice}<p class="card held small notice" data-testid="notice">{notice}</p>{/if}

{#if forgetting}
  <p class="card center-card"><span class="spinner"></span> Forgetting this computer…</p>
{:else if screen === 'loading'}
  <p class="muted center"><span class="spinner"></span></p>
{:else if screen === 'pair'}
  {#key pairValue}
    <Pair
      {pairValue}
      {resume}
      alreadyPaired={!!$pairing}
      currentD={$pairing?.d ?? null}
      on:paired={onPaired}
      on:cancel={cancelPair}
    />
  {/key}
{:else if $pairing}
  {#if screen === 'home'}
    <Home on:market={(e) => openMarket(e.detail.id, { title: e.detail.title })} />
  {:else if screen === 'markets'}
    <Markets on:open={(e) => openMarket(e.detail.id, { title: e.detail.title, state: e.detail.state })} />
  {:else if screen === 'market'}
    {#key marketId}
      <Market id={marketId} hint={marketHint} on:back={() => history.back()} on:trade={openTrade} />
    {/key}
  {:else if screen === 'trade' && tradeOf}
    <Trade
      market={tradeOf.market}
      outcome={tradeOf.outcome}
      side={tradeOf.side}
      on:back={() => history.back()}
      on:home={() => go('home')}
    />
  {:else if screen === 'receive'}
    <Receive />
  {:else if screen === 'settings'}
    <Settings on:forget={doForget} on:repair={() => go('pair')} />
  {/if}

  <nav aria-label="Main">
    {#each TABS as t (t.id)}
      <button class="tab" class:on={tab === t.id} aria-current={tab === t.id ? 'page' : undefined} on:click={() => go(t.id)}>
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d={t.icon} /></svg>
        <span>{t.label}</span>
      </button>
    {/each}
  </nav>
{/if}

<style>
  .top {
    position: sticky;
    top: 0;
    z-index: 9;
    background: var(--bg);
    padding-top: max(10px, env(safe-area-inset-top));
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    min-height: 48px;
    margin-bottom: 8px;
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 9px;
    font-size: 20px;
    font-weight: 700;
    letter-spacing: -0.01em;
  }
  .conn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 13px;
    color: var(--muted);
  }
  .conn .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--warn);
  }
  .conn.on .dot {
    background: var(--accent);
  }
  .notice {
    margin-bottom: 12px;
  }
  .center-card {
    text-align: center;
  }
  .center {
    text-align: center;
    padding: 40px 0;
  }
  nav {
    position: fixed;
    left: 0;
    right: 0;
    bottom: 0;
    display: flex;
    justify-content: center;
    background: var(--surface);
    border-top: 1px solid var(--border);
    padding: 4px max(8px, env(safe-area-inset-right)) max(6px, env(safe-area-inset-bottom)) max(8px, env(safe-area-inset-left));
    z-index: 10;
  }
  .tab {
    flex: 1;
    max-width: 140px;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 2px;
    min-height: 56px;
    border: none;
    background: transparent;
    color: var(--muted);
    font-size: 12px;
    font-weight: 600;
    padding: 0 4px;
  }
  .tab svg {
    width: 24px;
    height: 24px;
    fill: none;
    stroke: currentColor;
    stroke-width: 2;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .tab.on {
    color: var(--accent);
  }
</style>
