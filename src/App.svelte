<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { api, type AppInfo, type NodeStatus, type WalletStatus } from "./lib/api";
  import Setup from "./components/Setup.svelte";
  import Home from "./components/Home.svelte";
  import Markets from "./components/Markets.svelte";
  import Phone from "./components/Phone.svelte";
  import Settings from "./components/Settings.svelte";

  let info: AppInfo | null = null;
  let node: NodeStatus | null = null;
  let wallet: WalletStatus | null = null;
  let tab: "home" | "markets" | "phone" | "settings" = "home";
  let walletReady = false;
  let timer: ReturnType<typeof setInterval>;
  let heldCount = 0;
  let marketToOpen: string | null = null;

  async function poll() {
    try {
      node = await api.nodeStatus();
      if (node.run.state === "running") {
        wallet = await api.wallet().catch(() => wallet);
        if (wallet?.has_seed) walletReady = true;
      }
      const p = await api.phoneInfo().catch(() => null);
      heldCount = p?.held.length ?? 0;
    } catch {}
  }

  onMount(async () => {
    info = await api.appInfo();
    walletReady = info.wallet_ready;
    await poll();
    timer = setInterval(poll, 4000);
  });
  onDestroy(() => clearInterval(timer));

  $: running = node?.run.state === "running";
  $: setup = !node?.installed || !walletReady || (running && wallet !== null && !wallet.has_seed);
  $: synced = node?.sync?.phase === "idle";
</script>

<main>
  <header class="top">
    <div class="brand">
      <img src="/icon.png" alt="" />
      <h1>Truthcoin App</h1>
    </div>
    {#if node}
      {#if running}
        <span class="pill {synced ? 'ok' : 'warn'}" title="Truthcoin block height">
          {synced ? "" : "syncing · "}block {node.height ?? "…"}
        </span>
      {:else if node.run.state === "starting"}
        <span class="pill warn">starting</span>
      {:else if node.installed}
        <span class="pill bad">node stopped</span>
      {/if}
    {/if}
  </header>

  {#if info && !info.supported}
    <div class="notice error">
      L2L doesn't publish a Truthcoin node for this kind of computer (Linux x86-64 and macOS only), so this app can't
      run it here.
    </div>
  {:else if !node}
    <p class="muted"><span class="spin"></span> Looking around…</p>
  {:else if setup}
    <Setup {node} {wallet} on:changed={poll} on:done={() => { walletReady = true; tab = "home"; poll(); }} />
  {:else}
    {#if !running}
      <div class="notice warn">
        {#if node.run.state === "failed"}
          The Truthcoin node isn't running: {node.run.message.split("\n")[0]}
          <button class="link" on:click={() => (tab = "settings")}>More</button>
        {:else if node.run.state === "starting"}
          The Truthcoin node is starting…
        {:else}
          The Truthcoin node isn't running.
          <button class="link" on:click={async () => { await api.nodeStart().catch(() => {}); poll(); }}>Start it</button>
        {/if}
      </div>
    {/if}
    {#if heldCount > 0 && tab !== "phone"}
      <div class="notice warn">
        A phone is waiting for you to confirm a trade.
        <button class="link" on:click={() => (tab = "phone")}>Look</button>
      </div>
    {/if}
    {#if tab === "home"}
      <Home {wallet} {running} on:market={(e) => { tab = "markets"; marketToOpen = e.detail; }} on:changed={poll} />
    {:else if tab === "markets"}
      <Markets {running} bind:open={marketToOpen} />
    {:else if tab === "phone"}
      <Phone />
    {:else}
      <Settings {node} {info} on:changed={poll} />
    {/if}
    <nav class="tabs">
      <div class="inner">
        <button class:on={tab === "home"} on:click={() => (tab = "home")}>Home</button>
        <button class:on={tab === "markets"} on:click={() => (tab = "markets")}>Markets</button>
        <button class:on={tab === "phone"} on:click={() => (tab = "phone")}>Phone{heldCount ? ` (${heldCount})` : ""}</button>
        <button class:on={tab === "settings"} on:click={() => (tab = "settings")}>Settings</button>
      </div>
    </nav>
  {/if}
</main>
