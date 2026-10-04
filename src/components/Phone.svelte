<script lang="ts">
  // Settings › Phone: pair a phone (QR code, then the same code on both screens), phones and their daily limits, trades
  // held for you, and the relays the link uses.
  import { onDestroy, onMount } from "svelte";
  import { api, errText, type PhoneInfo } from "../lib/api";
  import { num, parseWhole, sats, when } from "../lib/format";
  import QrCode from "./QrCode.svelte";

  let info: PhoneInfo | null = null;
  let err = "";
  let pairUrl = "";
  let pair: { state: string; name?: string; code?: string; expires?: number } = { state: "none" };
  let timer: ReturnType<typeof setInterval>;
  let editLimit: Record<string, string> = {};
  let relaysText = "";
  let page = "";
  let editingRelays = false;

  async function load() {
    try {
      info = await api.phoneInfo();
      if (!editingRelays) {
        relaysText = info.relays.join("\n");
        page = info.page;
      }
      if (pairUrl) pair = await api.pairState();
    } catch (e) {
      err = errText(e);
    }
  }
  onMount(() => {
    load();
    timer = setInterval(load, 1500);
  });
  onDestroy(() => {
    clearInterval(timer);
    if (pairUrl && pair.state !== "allowed") api.pairCancel();
  });

  async function startPair() {
    err = "";
    pairUrl = await api.pairStart();
    pair = await api.pairState();
  }
  async function answer(allow: boolean) {
    try {
      await api.pairAnswer(allow);
      pair = await api.pairState();
      load();
    } catch (e) {
      err = errText(e);
    }
  }
  function endPair() {
    api.pairCancel();
    pairUrl = "";
    pair = { state: "none" };
  }
  async function held(id: string, approve: boolean) {
    try {
      await api.heldAnswer(id, approve);
      load();
    } catch (e) {
      err = errText(e);
    }
  }
  async function setLimit(np: string) {
    const n = parseWhole(editLimit[np] ?? "");
    if (!(n >= 0)) return (err = "The limit is a number of sats");
    try {
      await api.phoneSetLimit(np, n);
      delete editLimit[np];
      editLimit = editLimit;
      load();
    } catch (e) {
      err = errText(e);
    }
  }
  async function saveRelays() {
    try {
      await api.setRelays(relaysText.split("\n"), page);
      editingRelays = false;
      load();
    } catch (e) {
      err = errText(e);
    }
  }
</script>

{#if err}<div class="notice error">{err}</div>{/if}
{#if info?.blocked}<div class="notice warn">{info.blocked}. Settings › About shows the data folder; the unreadable files
  are there, renamed to end in <code>.bad-…</code>. Phone limits may have restarted from zero.
  <button class="link" on:click={() => api.recordsSeen().then(load)}>I've looked: let phones trade</button></div>{/if}

{#if info?.held.length}
  <h3>Waiting for you</h3>
  {#each info.held as h}
    <div class="card">
      <p><strong>{h.name}</strong> wants to {h.side} {num(h.shares)} <strong>{h.label}</strong> in “{h.title}” for {h.side === "buy" ? "at most" : "at least"} {sats(h.limit_sats)}.</p>
      <p class="small muted">Over this phone's daily limit · {when(h.at)}</p>
      <div class="actions">
        <button class="primary" on:click={() => held(h.id, true)}>Allow</button>
        <button class="danger" on:click={() => held(h.id, false)}>Refuse</button>
      </div>
    </div>
  {/each}
{/if}

<div class="card">
  <h2>Your phone</h2>
  {#if !pairUrl}
    <p>
      Pair a phone to see markets and your positions on it, and trade within a daily limit. It talks to this computer
      through public Nostr relays, sealed end to end; this app must be open for the phone to reach it.
    </p>
    <button class="primary" on:click={startPair}>Pair a phone</button>
  {:else if pair.state === "waiting"}
    <p>Scan this with the phone's camera. It opens the phone page, which pairs with this computer.</p>
    <div class="qr-wrap"><QrCode text={pairUrl} size={300} /></div>
    <p class="small muted"><span class="spin"></span> Waiting for the phone… (the code works for 5 minutes, once)</p>
    <button on:click={endPair}>Cancel</button>
  {:else if pair.state === "claimed"}
    <p><strong>{pair.name}</strong> asks to pair. Allow it only if the phone shows this same code. If the phone shows no
      code, or a different one, refuse.</p>
    <div class="code">{pair.code}</div>
    <div class="actions">
      <button class="primary" on:click={() => answer(true)}>Same code: allow</button>
      <button class="danger" on:click={() => answer(false)}>Different: refuse</button>
    </div>
  {:else if pair.state === "allowed"}
    <div class="notice ok">Paired. Your phone should now say so; if it doesn't, remove it below and pair again.</div>
    <button on:click={endPair}>Done</button>
  {:else if pair.state === "contested"}
    <div class="notice error">Two phones tried to pair with this code: someone else has seen it, or your phone cancelled
      and scanned it again. Refuse, and start again (where nobody can see your screen).</div>
    <button class="danger" on:click={() => answer(false)}>Refuse</button>
  {:else}
    <div class="notice warn">{pair.state === "refused" ? "Refused." : "The code expired."}</div>
    <button on:click={startPair}>Start again</button>
  {/if}
</div>

{#if info?.devices.length}
  <h3>Paired phones</h3>
  <div class="card flush list">
    {#each info.devices as d}
      <div class="item">
        <div style="flex:1">
          <div class="title">{d.name}</div>
          <div class="small muted">Paired {when(d.paired_at)} · last seen {when(d.last_seen)}</div>
          {#if editLimit[d.np] !== undefined}
            <div class="row" style="margin-top:6px">
              <input bind:value={editLimit[d.np]} inputmode="numeric" />
              <button on:click={() => setLimit(d.np)}>Save</button>
            </div>
          {:else}
            <div class="small">Daily limit {sats(d.limit_sats)} · {sats(d.left_sats)} left today
              <button class="link small" on:click={() => (editLimit[d.np] = String(d.limit_sats))}>Change</button></div>
          {/if}
        </div>
        <button class="danger" on:click={() => api.phoneRevoke(d.np).then(load)}>Remove</button>
      </div>
    {/each}
  </div>
  <p class="small muted">
    This computer's key: <code>{info.fingerprint}</code> (a paired phone shows the same in its Settings).
    A phone's trades count against its limit (a buy at its cap, a sell at its number of shares, a sat each); over it,
    they wait here for you for up to an hour. A phone can never withdraw, send coins, create markets or see your recovery words.
  </p>
{/if}

<details>
  <summary>Relays</summary>
  {#if info}
    {#each info.relay_status as r}
      <div class="row small"><code>{r.url}</code><span class="pill {r.connected ? 'ok' : 'bad'}">{r.connected ? "connected" : r.error ?? "not connected"}</span></div>
    {/each}
    {#if !info.relay_status.length}<p class="small muted">Not connected (nothing paired).</p>{/if}
  {/if}
  <div class="field" style="margin-top:8px">
    <label for="rl">Relays, one per line (wss://)</label>
    <textarea id="rl" rows="3" bind:value={relaysText} on:focus={() => (editingRelays = true)}></textarea>
  </div>
  <div class="field">
    <label for="pg">Phone page</label>
    <input id="pg" bind:value={page} on:focus={() => (editingRelays = true)} />
  </div>
  {#if editingRelays}
    <div class="actions"><button on:click={saveRelays}>Save</button><button on:click={() => { editingRelays = false; load(); }}>Cancel</button></div>
  {/if}
  <p class="small muted">A phone paired earlier learns new relays the next time it connects through an old one.</p>
</details>
