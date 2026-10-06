<script lang="ts">
  // Settings › Phone: pair a phone (QR code, then the same code on both screens), phones and their daily limits, trades
  // held for you, and the relays the link uses.
  import { onDestroy, onMount } from "svelte";
  import { api, errText, type PhoneInfo } from "../lib/api";
  import { num, parseWhole, sats, when } from "../lib/format";
  import QrCode from "./QrCode.svelte";
  import { copy, COPY_FAILED } from "../lib/copy";

  let info: PhoneInfo | null = null;
  let err = "";
  let pairUrl = "";
  let pair: { state: string; name?: string; code?: string; expires?: number } = { state: "none" };
  let timer: ReturnType<typeof setInterval>;
  let editLimit: Record<string, string> = {};
  let relaysText = "";
  let page = "";
  let editingRelays = false;
  let linkCopied = false;
  let heldNote = "";
  let removeAsk: string | null = null;
  // Pairing the same phone again (a new Home Screen icon, a cleared browser) leaves its old entry: the newest seen first,
  // names told apart by when they were paired, old entries of the same name replaced as it pairs, and phones not seen
  // for a week removed in one go.
  const WEEK = 7 * 24 * 3600;
  let replaceSame = true;
  let tidying = false;
  $: devices = info?.devices ?? [];
  $: sorted = [...devices].sort((a, b) => b.last_seen - a.last_seen);
  $: stale = devices.filter((d) => d.last_seen < Date.now() / 1000 - WEEK);
  $: sameNamed = pair.name ? devices.filter((d) => d.name === pair.name) : [];
  $: sameName = sameNamed.length;
  function label(d: { name: string; paired_at: number }): string {
    const same = devices.filter((x) => x.name === d.name).length > 1;
    return same ? `${d.name} (paired ${new Date(d.paired_at * 1000).toLocaleDateString(undefined, { dateStyle: "medium" })})` : d.name;
  }
  async function removeStale() {
    err = "";
    for (const d of stale) {
      await api.phoneRevoke(d.np).catch((e) => (err = errText(e)));
    }
    tidying = false;
    load();
  }

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
      await api.pairAnswer(allow, allow && sameName > 0 && replaceSame);
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
      const r = await api.heldAnswer(id, approve);
      heldNote = !approve ? "Refused: the phone is told." : r?.ok ? "Allowed: sent to the node." : r?.err ? `Not done: ${r.err}` : "Answered.";
      setTimeout(() => (heldNote = ""), 6000);
      load();
    } catch (e) {
      err = errText(e);
    }
  }
  async function remove(np: string) {
    if (removeAsk !== np) return void (removeAsk = np);
    removeAsk = null;
    await api.phoneRevoke(np).catch((e) => (err = errText(e)));
    load();
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

{#if heldNote}<div class="notice ok">{heldNote}</div>{/if}
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
    <button class:primary={!info?.devices.length} on:click={startPair}>{info?.devices.length ? "Pair another phone" : "Pair a phone"}</button>
  {:else if pair.state === "waiting"}
    <p>Scan this with the phone's camera. It opens the phone page, which pairs with this computer.</p>
    <div class="qr-wrap"><QrCode text={pairUrl} size={300} /></div>
    <p class="small muted"><span class="spin"></span> Waiting for the phone… (the code works for 5 minutes, once)</p>
    <div class="actions">
      <button on:click={async () => { linkCopied = await copy(pairUrl); if (!linkCopied) err = COPY_FAILED; setTimeout(() => (linkCopied = false), 2500); }}>{linkCopied ? "Copied" : "Copy link"}</button>
      <button on:click={endPair}>Cancel</button>
    </div>
    <p class="small muted">If the camera can't read the code, copy the link to your phone some private way and open it there.</p>
  {:else if pair.state === "claimed"}
    <p><strong>{pair.name}</strong> asks to pair. Allow it only if the phone shows this same code. If the phone shows no
      code, or a different one, refuse.</p>
    <div class="code">{pair.code}</div>
    {#if sameName}
      <label class="check small">
        <input type="checkbox" bind:checked={replaceSame} />
        <span>Remove the {sameName === 1 ? "other phone" : `${sameName} other phones`} called “{pair.name}”
          ({sameNamed.map((d) => `paired ${when(d.paired_at)}, last seen ${when(d.last_seen)}`).join("; ")}). Pairing the
          same phone again leaves its old entry behind; untick this if that's a different phone.</span>
      </label>
    {/if}
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
  {#if stale.length}
    {#if tidying}
      <div class="notice warn">
        Remove {stale.length} phone{stale.length === 1 ? "" : "s"} not seen for a week? Each is cut off at once; a phone you
        still use can pair again.
        <div class="actions">
          <button class="danger" on:click={removeStale}>Remove {stale.length}</button>
          <button on:click={() => (tidying = false)}>Cancel</button>
        </div>
      </div>
    {:else}
      <p class="small"><button class="link" on:click={() => (tidying = true)}>Remove {stale.length} phone{stale.length === 1 ? "" : "s"} not seen for a week…</button></p>
    {/if}
  {/if}
  <div class="card flush list">
    {#each sorted as d (d.np)}
      <div class="item">
        <div style="flex:1">
          <div class="title">{label(d)}</div>
          <div class="small muted">Paired {when(d.paired_at)} · last seen {when(d.last_seen)}</div>
          {#if editLimit[d.np] !== undefined}
            <label for="lim-{d.np}" style="margin-top:6px">Daily limit (sats)</label>
            <div class="row">
              <input id="lim-{d.np}" bind:value={editLimit[d.np]} inputmode="numeric" />
              <button on:click={() => setLimit(d.np)}>Save</button>
              <button on:click={() => { delete editLimit[d.np]; editLimit = editLimit; }}>Cancel</button>
            </div>
          {:else}
            <div class="small">Daily limit {sats(d.limit_sats)} · {sats(d.left_sats)} left today
              <button class="link small" on:click={() => (editLimit[d.np] = String(d.limit_sats))}>Change</button></div>
          {/if}
        </div>
        <button class="danger" on:click={() => remove(d.np)}>{removeAsk === d.np ? "Yes, remove it" : "Remove"}</button>
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

<style>
  .check {
    display: flex;
    gap: 8px;
    align-items: flex-start;
    margin: 10px 0;
  }
  .check input {
    margin-top: 3px;
  }
</style>
