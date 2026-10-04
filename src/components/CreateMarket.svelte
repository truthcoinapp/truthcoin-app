<script lang="ts">
  // Create a market: one question, its period, its depth; every cost shown before it is made.
  import { createEventDispatcher, onMount } from "svelte";
  import { api, errText, type NewMarket } from "../lib/api";
  import { sats } from "../lib/format";

  const dispatch = createEventDispatcher();
  let info: Awaited<ReturnType<typeof api.createInfo>> | null = null;
  let err = "";
  let busy = false;
  let cost: { liquidity_sats: number; listing_fee_sats: number; tx_fee_sats: number; total_sats: number } | null = null;

  let title = "";
  let description = "";
  let kind: NewMarket["kind"] = "binary";
  let question = "";
  let rules = "";
  let noLabel = "No";
  let yesLabel = "Yes";
  let options = "";
  let min = "0";
  let max = "100";
  let increment = "1";
  let period = 0;
  let depth = 1_000_000;
  let feePct = "1";
  let tags = "";

  const DEPTHS = [
    { beta: 100_000, name: "Shallow" },
    { beta: 1_000_000, name: "Medium" },
    { beta: 10_000_000, name: "Deep" },
  ];

  onMount(async () => {
    try {
      info = await api.createInfo();
      const later = info.periods.filter((p) => p.period_index > info!.current_period);
      period = (later[1] ?? later[0] ?? info.periods[0])?.period_index ?? 0;
    } catch (e) {
      err = errText(e);
    }
  });

  function market(): NewMarket {
    return {
      title,
      description,
      kind,
      question: question || title,
      rules,
      period,
      no_label: noLabel,
      yes_label: yesLabel,
      options: options.split("\n").map((o) => o.trim()).filter(Boolean),
      min: Number(min),
      max: Number(max),
      increment: Number(increment),
      beta: depth,
      trading_fee: Number(feePct) / 100,
      tags: tags.split(",").map((t) => t.trim()).filter(Boolean),
    };
  }

  $: if (title || kind || period || depth || options || question) cost = null;

  async function check() {
    err = "";
    busy = true;
    try {
      cost = await api.createCost(market());
    } catch (e) {
      err = errText(e);
    }
    busy = false;
  }

  async function create() {
    if (!cost) return;
    err = "";
    busy = true;
    try {
      const r = await api.createMarket(market(), cost.listing_fee_sats);
      dispatch("created", r.market_id);
    } catch (e) {
      err = errText(e);
    }
    busy = false;
  }

  function periodName(i: number): string {
    if (!info) return `Period ${i}`;
    const ahead = i - info.current_period;
    if (info.testing && info.blocks_per_period) return `Period ${i}: in about ${ahead * info.blocks_per_period} blocks`;
    return `Period ${i}: ${ahead} quarter${ahead === 1 ? "" : "s"} from now`;
  }
  // Sats in a few characters: 69,315 · 693,148 · 6.9M.
  function short(n: number): string {
    return n >= 1e6 ? `${(n / 1e6).toFixed(1)}M` : n >= 1e3 ? `${Math.round(n / 1e3)}k` : `${Math.round(n)}`;
  }
  $: outcomeCount = kind === "category" ? Math.max(2, options.split("\n").filter((o) => o.trim()).length) : 2;
</script>

<button class="link back" on:click={() => dispatch("close")}>‹ Markets</button>
<div class="card">
  <h2>Create a market</h2>
  <div class="field"><label for="t">Title</label><input id="t" bind:value={title} maxlength="200" placeholder="Will it rain in Melbourne on 1 January 2027?" /></div>
  <div class="field"><label for="d">What it's about</label><textarea id="d" rows="3" bind:value={description} maxlength="4000"></textarea></div>
  <div class="field">
    <div class="small muted" style="margin-bottom:4px">Kind of answer</div>
    <div class="seg">
      <button class:on={kind === "binary"} on:click={() => (kind = "binary")}>Yes or no</button>
      <button class:on={kind === "category"} on:click={() => (kind = "category")}>One of several</button>
      <button class:on={kind === "scaled"} on:click={() => (kind = "scaled")}>A number</button>
    </div>
  </div>
  <div class="field"><label for="q">The question voters answer (short)</label><input id="q" bind:value={question} maxlength="200" placeholder={title} /></div>
  <div class="field"><label for="r">How it will be decided</label><textarea id="r" rows="3" bind:value={rules} maxlength="2000" placeholder="The source voters should check, and what counts."></textarea></div>
  {#if kind === "binary"}
    <div class="row">
      <div class="field" style="flex:1"><label for="yl">"Yes" answer</label><input id="yl" bind:value={yesLabel} maxlength="60" /></div>
      <div class="field" style="flex:1"><label for="nl">"No" answer</label><input id="nl" bind:value={noLabel} maxlength="60" /></div>
    </div>
  {:else if kind === "category"}
    <div class="field"><label for="op">Options, one per line (2 to 16)</label><textarea id="op" rows="4" bind:value={options}></textarea></div>
  {:else}
    <div class="row">
      <div class="field" style="flex:1"><label for="mn">From</label><input id="mn" bind:value={min} inputmode="numeric" /></div>
      <div class="field" style="flex:1"><label for="mx">To</label><input id="mx" bind:value={max} inputmode="numeric" /></div>
      <div class="field" style="flex:1"><label for="inc">Step</label><input id="inc" bind:value={increment} inputmode="numeric" /></div>
    </div>
  {/if}
  <div class="field">
    <label for="p">When voters decide</label>
    <select id="p" bind:value={period}>
      {#each info?.periods ?? [] as p}
        <option value={p.period_index}>{periodName(p.period_index)}</option>
      {/each}
    </select>
  </div>
  <div class="field">
    <div class="small muted" style="margin-bottom:4px">Depth: in a deeper market each trade moves the price less, and you put in more</div>
    <div class="seg">
      {#each DEPTHS as d}<button class:on={depth === d.beta} on:click={() => (depth = d.beta)}>{d.name} · {short(d.beta * Math.log(outcomeCount))}</button>{/each}
    </div>
  </div>
  <div class="row">
    <div class="field" style="flex:1"><label for="f">Trading fee (%)</label><input id="f" bind:value={feePct} inputmode="decimal" /></div>
    <div class="field" style="flex:2"><label for="tg">Tags (comma-separated)</label><input id="tg" bind:value={tags} /></div>
  </div>
  {#if err}<div class="notice error">{err}</div>{/if}
  {#if !cost}
    <button class="primary" disabled={busy || !title.trim()} on:click={check}>{#if busy}<span class="spin"></span>{/if} Show the costs</button>
  {:else}
    <dl class="kv">
      <dt>Liquidity you put in</dt><dd>{sats(cost.liquidity_sats)}</dd>
      <dt>Listing fee</dt><dd>{sats(cost.listing_fee_sats)}</dd>
      <dt>Transaction fee</dt><dd>{sats(cost.tx_fee_sats)}</dd>
      <dt><strong>Total</strong></dt><dd><strong>{sats(cost.total_sats)}</strong></dd>
    </dl>
    <p class="small muted" style="margin-top:8px">
      The liquidity is what lets people trade at smoothly moving prices. When the market settles, what's left of it
      after paying the winners comes back to you, and you earn the trading fees. It can all go to the winners if traders
      call it right. The listing and transaction fees don't come back. The market opens for trading with the next
      Truthcoin block.
    </p>
    <div class="actions">
      <button class="primary" disabled={busy} on:click={create}>{#if busy}<span class="spin"></span>{/if} Create it</button>
      <button on:click={() => (cost = null)}>Change something</button>
    </div>
  {/if}
</div>
