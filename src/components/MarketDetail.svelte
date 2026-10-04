<script lang="ts">
  import { createEventDispatcher, onDestroy, onMount } from "svelte";
  import { api, errText, type MarketDetail } from "../lib/api";
  import { chance, num, sats } from "../lib/format";
  import TradePanel from "./TradePanel.svelte";

  export let id: string;
  /** Made a moment ago: it appears with the next Truthcoin block. */
  export let fresh = false;
  const dispatch = createEventDispatcher();
  let d: MarketDetail | null = null;
  let err = "";
  let waiting = false;
  let pick: number | null = null;
  let timer: ReturnType<typeof setTimeout> | undefined;

  async function load() {
    try {
      d = await api.market(id);
      err = "";
      waiting = false;
    } catch (e) {
      const t = errText(e);
      if (fresh && t.includes("no such market")) {
        waiting = true;
        timer = setTimeout(load, 10000);
      } else {
        err = t;
      }
    }
  }
  onMount(load);
  onDestroy(() => timer && clearTimeout(timer));

  // In a one-question market, "Q: Yes" reads "Yes".
  function label(o: any): string {
    const l: string = o.label ?? "";
    if (d && d.market.dimensions?.length === 1) {
      const i = l.lastIndexOf(": ");
      if (i >= 0) return l.slice(i + 2);
    }
    return l;
  }
  function labelOf(i: number): string {
    return label(d?.market.outcomes.find((o: any) => o.outcome_index === i) ?? {});
  }
  $: m = d?.market;
  $: held = (i: number) => d?.holdings.find((h) => h.outcome === i);
  $: winners = new Set<number>((m?.resolution?.winning_outcomes ?? []).map((w: any) => w.outcome_index));
</script>

<button class="link back" on:click={() => dispatch("back")}>‹ Markets</button>
{#if err}<div class="notice error">{err}</div>{/if}
{#if waiting}
  <div class="notice ok">
    Created. The market appears with the next Truthcoin block (about 10–17 minutes); this page shows it then.
  </div>
{/if}
{#if m}
  <div class="card">
    <h2>{m.title}</h2>
    {#if m.description}<p class="small muted" style="white-space:pre-wrap">{m.description}</p>{/if}
    <p class="small muted">
      {m.state === "trading" ? "Open for trading" : m.state} · traded {sats(m.total_volume_sats)} · fee {(m.trading_fee_rate * 100).toFixed(1)}%
    </p>
    {#if m.resolution}
      <div class="notice ok">{m.resolution.summary}</div>
    {/if}
  </div>

  <h3>Outcomes</h3>
  <div class="card flush list">
    {#each m.outcomes as o}
      <button class="item" disabled={m.state !== "trading" && !held(o.outcome_index)} on:click={() => (pick = o.outcome_index)}
        style:background={pick === o.outcome_index ? "var(--accent-tint)" : ""}>
        <div style="flex:1">
          <div class="row">
            <div class="title">{label(o)}{winners.has(o.outcome_index) ? " ✓" : ""}</div>
            <div>{chance(o.price)}</div>
          </div>
          <div class="bar"><div style:width="{Math.max(0, Math.min(100, o.price * 100))}%"></div></div>
          {#if held(o.outcome_index)}
            <div class="small" style="margin-top:4px">You hold {num(held(o.outcome_index)?.shares)} shares, worth about {sats(held(o.outcome_index)?.value_sats)}</div>
          {/if}
        </div>
      </button>
    {/each}
  </div>
  <p class="small muted">A share pays 1 sat if its outcome happens. Its price is the market's chance.</p>

  {#if pick !== null && m.state === "trading"}
    <TradePanel market={m} outcome={pick} outcomeLabel={labelOf(pick)}
      held={held(pick)?.shares ?? 0} on:done={load} on:close={() => (pick = null)} />
  {/if}
{:else if !err && !waiting}
  <p class="muted"><span class="spin"></span></p>
{/if}
