<script lang="ts">
  import { onMount } from "svelte";
  import { api, errText, type MarketRow } from "../lib/api";
  import { chance, sats } from "../lib/format";
  import { created, noteCreated } from "../lib/created";
  import MarketDetail from "./MarketDetail.svelte";
  import CreateMarket from "./CreateMarket.svelte";

  export let running: boolean;
  /** A market to open (from Home's positions). */
  export let open: string | null = null;

  let list: MarketRow[] = [];
  let err = "";
  let filter: "trading" | "all" = "trading";
  let q = "";
  let creating = false;

  async function load() {
    if (!running) return;
    try {
      list = await api.markets();
      err = "";
    } catch (e) {
      err = errText(e);
    }
  }
  onMount(load);
  $: if (running && !list.length) load();

  $: shown = list
    .filter((m) => filter === "all" || m.state === "trading")
    .filter((m) => !q.trim() || (m.title + " " + m.description).toLowerCase().includes(q.trim().toLowerCase()))
    .sort((a, b) => b.created_at_height - a.created_at_height);
  $: waitingNew = [...$created].filter((id) => !list.some((m) => m.market_id === id));
</script>

{#if creating}
  <CreateMarket on:close={() => { creating = false; load(); }} on:created={(e) => { creating = false; noteCreated(e.detail); open = e.detail; }} />
{:else if open}
  <MarketDetail id={open} on:back={() => { open = null; load(); }} />
{:else}
  {#if err}<div class="notice error">{err}</div>{/if}
  <div class="row" style="margin-bottom:10px">
    <input placeholder="Search markets" bind:value={q} />
    <div class="seg">
      <button class:on={filter === "trading"} on:click={() => (filter = "trading")}>Open</button>
      <button class:on={filter === "all"} on:click={() => (filter = "all")}>All</button>
    </div>
    <button disabled={!running} on:click={() => (creating = true)}>New market</button>
  </div>
  {#each waitingNew as id}
    <p class="small muted">A market you made appears with the next Truthcoin block. <button class="link" on:click={() => (open = id)}>Open it</button></p>
  {/each}
  {#if !shown.length}
    <p class="muted small">
      {#if !running}The node isn't running.
      {:else if q.trim()}No markets match.
      {:else if list.length && filter === "trading"}No open markets. <button class="link" on:click={() => (filter = "all")}>All</button> shows settled ones.
      {:else}No markets yet. Make the first one with New market.{/if}
    </p>
  {:else}
    <div class="card flush list">
      {#each shown as m}
        <button class="item" on:click={() => (open = m.market_id)}>
          <div style="flex:1">
            <div class="title">{m.title}</div>
            <div class="small muted">
              {#if m.leading && m.state === "trading"}<span style="color:var(--text)">{m.leading[0]} {chance(m.leading[1])}</span> · {/if}traded {sats(m.volume_sats)}{m.state !== "trading" ? ` · ${m.state === "settled" ? "Settled" : m.state}` : ""}
            </div>
          </div>
          <div class="muted">›</div>
        </button>
      {/each}
    </div>
  {/if}
  <p class="small muted" style="margin-top:12px">
    New market: ask a question that voters will answer later. You put in the liquidity that lets people trade, and you
    earn the trading fees.
  </p>
{/if}
