<script lang="ts">
  // Settings › Obliterate: remove Truthcoin (the node program, its data with the wallet, the wallet's records), the app
  // (settings, phone link, what the window stored, and where it can, the program itself as it closes), or both. The
  // Rust side works out the list; the screen sends back exactly what it showed, and the Rust side acts only if a fresh
  // list still matches. The wallet's holdings are shown, as far as the node can tell, and its recovery words are asked
  // for whenever the wallet goes (the Rust side checks the tick too: what a wallet holds can't be read reliably enough
  // to excuse it).
  import { createEventDispatcher } from "svelte";
  import { api, errText, type ObliteratePlan, type Obliterated } from "../lib/api";
  import { bytes, num, sats } from "../lib/format";

  export let running: boolean;
  const dispatch = createEventDispatcher<{ obliterated: Obliterated }>();

  let open = false;
  let plan: ObliteratePlan | null = null;
  let err = "";
  let busy = false;
  let typed = "";
  let truthcoin = true;
  let theApp = true;
  let haveWords = false;
  let held: { sats: number; out: number; shares: number; markets: number } | null = null;
  let heldNote = "";

  async function load() {
    err = "";
    plan = null;
    held = null;
    heldNote = "";
    try {
      plan = await api.obliteratePlan();
    } catch (e) {
      err = errText(e);
      return;
    }
    if (!running) {
      heldNote = "The node isn't running, so the app can't say what your wallet holds.";
      return;
    }
    try {
      const [w, pos, st] = await Promise.all([api.wallet(), api.positions(), api.nodeStatus()]);
      if (!w.has_seed) return;
      held = {
        sats: w.total_sats,
        out: w.withdrawing_sats,
        shares: pos.reduce((a, h) => a + h.shares, 0),
        markets: new Set(pos.filter((h) => h.shares > 0).map((h) => h.market_id)).size,
      };
      if (st.sync?.phase !== "idle") heldNote = "The node is still catching up, so this may not be everything.";
    } catch (e) {
      heldNote = `The app couldn't read what your wallet holds (${errText(e)}).`;
    }
  }

  function toggle() {
    open = !open;
    typed = "";
    haveWords = false;
    if (open) load();
  }

  const sum = (l: { size: number }[]) => l.reduce((a, i) => a + i.size, 0);
  $: tcItems = plan?.items.filter((i) => i.part === "truthcoin") ?? [];
  $: appItems = plan?.items.filter((i) => i.part === "app") ?? [];

  // Nothing of Truthcoin's left (removed before, or never installed): its box is off.
  $: if (plan && tcItems.length === 0) truthcoin = false;
  $: walletGoes = truthcoin && tcItems.some((i) => i.id === "node-data");
  $: ready = !!plan && !plan.blocked && (truthcoin || theApp) && typed === "OBLITERATE" && (!walletGoes || haveWords);

  async function go() {
    if (!plan || !ready) return;
    busy = true;
    err = "";
    try {
      const r = await api.obliterate(truthcoin, theApp, walletGoes && haveWords, plan.items.map((i) => ({ id: i.id, path: i.path })));
      dispatch("obliterated", r);
    } catch (e) {
      err = errText(e);
      busy = false;
      load();
    }
  }
</script>

<div class="card">
  <h2>Obliterate</h2>
  <p class="small">
    Remove Truthcoin from this computer (the node program this app downloaded, its data with your wallet), this app, or
    both. It can't be undone. BitWindow, eCash and the enforcer are never touched.
  </p>
  <div class="actions">
    <button class="danger" disabled={busy} on:click={toggle}>{open ? "Cancel" : "Obliterate…"}</button>
  </div>

  {#if open}
    {#if err}<div class="notice error" style="margin-top:10px">{err}</div>{/if}
    {#if !plan}
      {#if !err}<p class="small muted" style="margin-top:10px"><span class="spin"></span> Looking at what this app put here…</p>{/if}
    {:else}
      <label class="part" data-testid="part-truthcoin">
        <input type="checkbox" bind:checked={truthcoin} disabled={busy || tcItems.length === 0} />
        <span>
          <strong>Truthcoin</strong> <span class="muted small">{bytes(sum(tcItems))}</span>
          <span class="small block">The Truthcoin node program, its data (the chain, and your wallet with its seed), and the
            wallet's records.</span>
        </span>
      </label>
      <ul class="items">
        {#each tcItems as i (i.id)}
          <li class:off={!truthcoin}><strong>{i.label}</strong> <span class="muted">{bytes(i.size)}</span><br /><code>{i.path}</code><br /><span class="muted">{i.note}</span></li>
        {:else}
          <li class="muted">Nothing of Truthcoin's is left here.</li>
        {/each}
      </ul>
      {#if plan.own_node_program}
        <p class="small muted">Your own node program (<code>{plan.own_node_program}</code>, chosen under Advanced) stays.</p>
      {/if}

      {#if walletGoes}
        <div class="notice warn" data-testid="wallet-warning">
          {#if held}
            Your wallet holds <strong>{sats(held.sats)}</strong>{held.shares
              ? `, and ${num(held.shares)} shares in ${held.markets} market${held.markets === 1 ? "" : "s"}`
              : ""}{held.out ? `; ${sats(held.out)} are on their way to eCash, and come back here if their bundle fails` : ""}.
          {/if}
          {#if heldNote}{heldNote}{/if}
          Without your recovery words, whatever it holds is lost: this app keeps no copy of them. Withdraw first, or make
          sure you have the words.
          <label class="check">
            <input type="checkbox" bind:checked={haveWords} disabled={busy} />
            <span>I have this wallet's recovery words written down, or it holds nothing I need.</span>
          </label>
        </div>
      {/if}

      <label class="part" data-testid="part-app">
        <input type="checkbox" bind:checked={theApp} disabled={busy} />
        <span>
          <strong>The app</strong> <span class="muted small">{bytes(sum(appItems))}</span>
          <span class="small block">Its settings, activity log and phone link (your paired phones stop working), and what its
            window stored.</span>
        </span>
      </label>
      <ul class="items">
        {#each appItems as i (i.id)}
          <li class:off={!theApp}><strong>{i.label}</strong> <span class="muted">{bytes(i.size)}</span><br /><code>{i.path}</code><br /><span class="muted">{i.note}</span></li>
        {/each}
      </ul>
      {#if theApp && !plan.remove_app.at_exit}
        <p class="small">
          {#if plan.remove_app.kind === "deb"}
            The program itself stays installed: remove it afterwards with <code>sudo apt remove truthcoin-app</code>.
          {:else if plan.remove_app.kind === "mac"}
            The app can't remove its own program from where it runs: afterwards, drag Truthcoin App to the Trash.
          {:else if plan.remove_app.path}
            The program itself stays: delete <code>{plan.remove_app.path}</code> afterwards.
          {/if}
        </p>
      {/if}

      {#if truthcoin && !theApp}
        <p class="small muted">The app stays, and goes back to setting up: it downloads the node again, then makes a new wallet
          or restores one from its recovery words.</p>
      {:else if theApp && !truthcoin}
        <p class="small muted">Truthcoin stays in the app's folder: a later install of this app finds the node and your wallet
          there.</p>
      {/if}

      {#if plan.blocked}<div class="notice warn">{plan.blocked} <button class="link" on:click={load}>Check again</button></div>{/if}

      <div class="field" style="margin-top:10px">
        <label for="obl">Type <code>OBLITERATE</code> to confirm</label>
        <input id="obl" bind:value={typed} autocomplete="off" autocapitalize="off" spellcheck="false" disabled={busy} />
      </div>
      <div class="actions">
        <button class="danger" disabled={!ready || busy} on:click={go} data-testid="obliterate-go">
          {#if busy}<span class="spin"></span> Removing…{:else}Obliterate{truthcoin && theApp ? " both" : truthcoin ? " Truthcoin" : theApp ? " the app" : ""}{/if}
        </button>
        <button disabled={busy} on:click={toggle}>Cancel</button>
      </div>
    {/if}
  {/if}
</div>

<style>
  .part {
    display: flex;
    gap: 10px;
    align-items: flex-start;
    margin-top: 14px;
    cursor: pointer;
  }
  .part input {
    margin-top: 4px;
  }
  .block {
    display: block;
  }
  .items {
    list-style: none;
    margin: 6px 0 4px 28px;
    padding: 0;
    font-size: 13px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .items li {
    overflow-wrap: anywhere;
  }
  .items li.off {
    opacity: 0.45;
  }
  .check {
    display: flex;
    gap: 8px;
    align-items: flex-start;
    margin-top: 8px;
  }
  .check input {
    margin-top: 3px;
  }
</style>
