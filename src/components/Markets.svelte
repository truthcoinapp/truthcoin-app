<script lang="ts">
  import { onMount } from "svelte";
  import { api, errText, type MarketSummary } from "../lib/api";
  import { sats } from "../lib/format";
  import MarketDetail from "./MarketDetail.svelte";
  import CreateMarket from "./CreateMarket.svelte";

  export let running: boolean;
  /** A market to open (from Home's positions). */
  export let open: string | null = null;

  let list: MarketSummary[] = [];
  let err = "";
  let filter: "trading" | "all" = "trading";
  let q = "";
  let creating = false;
  let fresh = false;

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
</script>

{#if creating}
  <CreateMarket on:close={() => { creating = false; load(); }} on:created={(e) => { creating = false; fresh = true; open = e.detail; }} />
{:else if open}
  <MarketDetail id={open} {fresh} on:back={() => { open = null; fresh = false; load(); }} />
{:else}
  {#if err}<div class="notice error">{err}</div>{/if}
  <div class="row" style="margin-bottom:10px">
    <input placeholder="Search markets" bind:value={q} />
    <div class="seg">
      <button class:on={filter === "trading"} on:click={() => (filter = "trading")}>Open</button>
      <button class:on={filter === "all"} on:click={() => (filter = "all")}>All</button>
    </div>
  </div>
  {#if !shown.length}
    <p class="muted small">{list.length ? "No markets match." : running ? "No markets yet." : "The node isn't running."}</p>
  {:else}
    <div class="card flush list">
      {#each shown as m}
        <button class="item" on:click={() => (open = m.market_id)}>
          <div>
            <div class="title">{m.title}</div>
            <div class="small muted">
              {m.outcome_count} outcomes · traded {sats(m.volume_sats)}{m.state !== "trading" ? ` · ${m.state}` : ""}
            </div>
          </div>
          <div class="muted">›</div>
        </button>
      {/each}
    </div>
  {/if}
  <details>
    <summary>Create a market</summary>
    <p class="small muted">
      Ask a question that voters will answer later. You put in the liquidity that lets people trade, and you earn the
      trading fees.
    </p>
    <button on:click={() => (creating = true)} disabled={!running}>Create a market…</button>
  </details>
{/if}
