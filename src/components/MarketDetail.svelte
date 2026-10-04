<script lang="ts">
  import { createEventDispatcher, onDestroy, onMount, tick } from "svelte";
  import { api, errText, type MarketDetail, type Side } from "../lib/api";
  import { chance, num, sats } from "../lib/format";
  import { created } from "../lib/created";
  import TradePanel from "./TradePanel.svelte";

  export let id: string;
  const dispatch = createEventDispatcher();
  let d: MarketDetail | null = null;
  let err = "";
  let waiting = false;
  let pick: number | null = null;
  let pickSide: Side = "buy";
  let timer: ReturnType<typeof setInterval> | undefined;
  let panelEl: HTMLElement | undefined;

  async function load() {
    try {
      d = await api.market(id);
      err = "";
      waiting = false;
    } catch (e) {
      const t = errText(e);
      if ($created.has(id) && t.includes("no such market")) {
        waiting = true;
      } else {
        err = t;
      }
    }
  }
  onMount(() => {
    load();
    // Prices move while the page is open; a market just made appears with its block.
    timer = setInterval(load, 20000);
  });
  onDestroy(() => timer && clearInterval(timer));

  // In a one-question market, "Q: Yes" reads "Yes".
  function label(o: any): string {
    const l: string = o?.label ?? "";
    if (d && d.market.dimensions?.length === 1) {
      const i = l.lastIndexOf(": ");
      if (i >= 0) return l.slice(i + 2);
    }
    return l;
  }
  function labelOf(i: number): string {
    return label(d?.market.outcomes.find((o: any) => o.outcome_index === i) ?? {});
  }
  async function trade(i: number, side: Side) {
    pick = i;
    pickSide = side;
    await tick();
    panelEl?.scrollIntoView({ behavior: "smooth", block: "start" });
  }
  function whenVoting(period: number): string {
    if (!d) return "";
    const ahead = period - d.current_period;
    if (ahead <= 0) return `period ${period} (now)`;
    if (d.testing && d.blocks_per_period) return `period ${period} (in about ${ahead * d.blocks_per_period} blocks)`;
    return `period ${period} (about ${ahead} quarter${ahead === 1 ? "" : "s"} from now)`;
  }
  $: m = d?.market;
  $: held = (i: number) => d?.holdings.find((h) => h.outcome === i);
  $: settled = m?.state === "settled";
  // What each outcome's share paid, once settled: the node's final price for it (0 if not among the winners).
  $: paidPer = (i: number): number => {
    const w = (m?.resolution?.winning_outcomes ?? []).find((x: any) => x.outcome_index === i);
    return w ? Number(w.price) || 0 : 0;
  };
  $: split = settled && (m?.resolution?.winning_outcomes ?? []).length > 1;
  $: stateWord = m?.state === "trading" ? "Open for trading" : settled ? "Settled" : m?.state ?? "";
</script>

<button class="link back" on:click={() => dispatch("back")}>‹ Markets</button>
{#if err}<div class="notice error">{err}</div>{/if}
{#if waiting}
  <div class="notice ok">
    Sent. The market appears with the next Truthcoin block (about 10–17 minutes); this page shows it then.
  </div>
{/if}
{#if m}
  <div class="card">
    <div class="row" style="align-items:flex-start">
      <h2 style="margin:0">{m.title}</h2>
      <span class="pill {m.state === 'trading' ? 'ok' : ''}">{stateWord}</span>
    </div>
    {#if m.description}<p class="small muted" style="white-space:pre-wrap;margin-top:8px">{m.description}</p>{/if}
    <p class="small muted">
      Traded {sats(m.total_volume_sats)} · fee {(m.trading_fee_rate * 100).toFixed(1)}% (at least 1,000 sats a trade) + 1,000 sats to the miner
    </p>
    {#each d?.decisions ?? [] as q}
      <div style="margin-top:8px">
        {#if (d?.decisions.length ?? 0) > 1}<div class="small"><strong>{q.question}</strong></div>{/if}
        {#if q.rules}<p class="small"><span class="muted">How it's decided:</span> {q.rules}</p>{/if}
        {#if settled}
          <p class="small"><span class="muted">Decided in</span> period {q.period}</p>
        {:else}
          <p class="small"><span class="muted">Voters decide in</span> {whenVoting(q.period)}</p>
        {/if}
      </div>
    {/each}
  </div>

  {#if settled}
    <div class="notice ok">
      {#if split}
        Settled without one clear answer: the shares share the payout, as below.
      {:else}
        Settled: {labelOf((m.resolution?.winning_outcomes ?? [])[0]?.outcome_index)}. Each of its shares paid 1 sat; the others paid nothing.
      {/if}
      The node paid holders automatically.
    </div>
  {/if}

  <h3>Outcomes</h3>
  <div class="card flush list">
    {#each m.outcomes as o}
      <div class="item" style:background={pick === o.outcome_index ? "var(--accent-tint)" : ""}>
        <div style="flex:1">
          <div class="row">
            <div class="title">{label(o)}</div>
            {#if settled}
              <div class="nowrap">{paidPer(o.outcome_index) >= 0.999 ? "paid 1 sat a share" : paidPer(o.outcome_index) > 0 ? `paid ${paidPer(o.outcome_index).toFixed(2)} sat a share` : "paid nothing"}</div>
            {:else}
              <div>{chance(o.price)}</div>
            {/if}
          </div>
          {#if !settled}<div class="bar"><div style:width="{Math.max(0, Math.min(100, o.price * 100))}%"></div></div>{/if}
          {#if held(o.outcome_index)}
            <div class="small" style="margin-top:4px">You hold {num(held(o.outcome_index)?.shares)} shares, worth about {sats(held(o.outcome_index)?.value_sats)}</div>
          {/if}
          {#if m.state === "trading"}
            <div class="actions" style="margin-top:8px">
              <button class="primary small" on:click={() => trade(o.outcome_index, "buy")}>Buy</button>
              {#if held(o.outcome_index)}<button class="small" on:click={() => trade(o.outcome_index, "sell")}>Sell</button>{/if}
            </div>
          {/if}
        </div>
      </div>
    {/each}
  </div>
  {#if !settled}<p class="small muted">A share pays 1 sat if its outcome happens. Its price is the market's chance.</p>{/if}

  {#if pick !== null && m.state === "trading"}
    <div bind:this={panelEl}>
      <TradePanel market={m} outcome={pick} outcomeLabel={labelOf(pick)} side={pickSide}
        held={held(pick)?.shares ?? 0} on:done={load} on:close={() => (pick = null)} />
    </div>
  {/if}
{:else if !err && !waiting}
  <p class="muted"><span class="spin"></span></p>
{/if}
