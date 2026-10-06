<script lang="ts">
  import { createEventDispatcher } from "svelte";
  import { api, errText, type AppInfo, type NodeStatus } from "../lib/api";
  import { num } from "../lib/format";
  import EnforcerAddress from "./EnforcerAddress.svelte";
  import Obliterate from "./Obliterate.svelte";
  import AppUpdate from "./AppUpdate.svelte";

  export let node: NodeStatus;
  export let info: AppInfo | null;
  const dispatch = createEventDispatcher();
  let log = "";
  let err = "";
  let busy = false;
  let adv: Awaited<ReturnType<typeof api.advanced>> | null = null;
  let advSaved = false;

  async function start() {
    busy = true;
    err = "";
    try {
      await api.nodeStart();
    } catch (e) {
      err = errText(e);
    }
    busy = false;
    dispatch("changed");
  }
  async function stop() {
    busy = true;
    await api.nodeStop();
    busy = false;
    dispatch("changed");
  }
  async function showLog() {
    log = await api.nodeLog();
  }
  async function loadAdv() {
    adv = await api.advanced();
  }
  async function saveAdv() {
    if (!adv) return;
    err = "";
    try {
      await api.advancedSet({ ...adv, rpc_port: Number(adv.rpc_port), zmq_port: Number(adv.zmq_port) });
      advSaved = true;
    } catch (e) {
      err = errText(e);
    }
  }
  $: running = node.run.state === "running";
</script>

{#if err}<div class="notice error">{err}</div>{/if}
{#if node.enforcer.remote}
  <div class="notice warn">The enforcer is on another computer ({node.enforcer.address}). Anyone who can reach it can
    spend its eCash wallet, and the connection isn't encrypted. An SSH tunnel fixes both (README, "eCash on another
    computer").</div>
{/if}

<div class="card">
  <h2>Truthcoin node</h2>
  <dl class="kv">
    <dt>State</dt><dd>{node.run.state}</dd>
    <dt>Network</dt><dd>{node.network}</dd>
    <dt>Truthcoin height</dt><dd>{node.height ?? "—"}</dd>
    <dt>Following eCash</dt><dd>{!node.sync ? "—" : node.sync.phase === "idle" ? "up to date" : `catching up (${num(node.sync.done)} of ${num(node.sync.total)})`}</dd>
    <dt>Peers</dt><dd>{node.peers ?? "—"}</dd>
    <dt>Enforcer</dt><dd>{node.enforcer.reachable ? `${node.enforcer.address}, eCash block ${num(node.enforcer.height)}` : `not answering at ${node.enforcer.address}`}</dd>
    <dt>Node version</dt><dd>{info?.node_version}{node.own_program ? " (your own program)" : ""}</dd>

  </dl>
  {#if node.run.state === "failed"}<pre class="log" style="margin-top:10px">{node.run.message}</pre>{/if}
  <div class="actions">
    {#if running}
      <button disabled={busy} on:click={stop}>Stop the node</button>
    {:else}
      <button class="primary" disabled={busy || node.run.state === "starting"} on:click={start}>Start the node</button>
    {/if}
    <button on:click={showLog}>Show its log</button>
  </div>
  {#if log}<pre class="log" style="margin-top:10px">{log}</pre>{/if}
</div>

<div class="card">
  <h2>Security</h2>
  <p class="small">
    The Truthcoin node has no login, and lets any web page call it: a program on this computer that finds its wallet
    port could spend from this wallet, and its seed is stored unencrypted in its data folder. This app puts the wallet
    on a port it picks at random each start (on Linux, at a random local address too, so a web page can't find it by
    scanning), listening only on this computer, but it can't make the node safe. Run it on a computer you trust, close
    it when you aren't using it, and keep only what you're trading in it.
  </p>
  <p class="small muted">
    A paired phone reaches only what this app allows (markets, positions, quotes, trades within its limit, receiving
    addresses); bigger trades wait for you here.
  </p>
</div>

<details on:toggle={(e) => e.currentTarget.open && loadAdv()}>
  <summary>Advanced</summary>
  {#if adv}
    <div class="card">
      {#if info?.beta_only}
        <p class="small muted">Network: eCash beta. This release runs only there: it checks that the enforcer follows
          eCash beta (its fork block, 967,680) before starting the node.</p>
      {:else}
        <div class="field"><label for="n">Network (a development build)</label>
          <select id="n" bind:value={adv.network}>
            <option value="betanet">betanet (eCash beta)</option><option value="forknet">forknet</option>
            <option value="signet">signet</option><option value="regtest">regtest</option>
          </select>
        </div>
      {/if}
      <EnforcerAddress bind:address={adv.enforcer} />
      <div class="row">
        <div class="field" style="flex:1"><label for="rp">RPC port</label><input id="rp" bind:value={adv.rpc_port} inputmode="numeric" /></div>
        <div class="field" style="flex:1"><label for="zp">ZMQ port</label><input id="zp" bind:value={adv.zmq_port} inputmode="numeric" /></div>
      </div>
      <div class="field"><label for="pa">Peer-to-peer address</label><input id="pa" bind:value={adv.p2p_addr} /></div>
      {#if node.wallet_host}<p class="small muted">The node's wallet calls are on <code>{node.wallet_host}</code>{node.wallet_host === "127.0.0.1" ? " (a web page could find it by scanning)" : ", a random local address"}.</p>{/if}
      <button on:click={saveAdv}>Save</button>
      {#if advSaved}
        <p class="small muted" style="margin-top:6px">Saved: they apply when the node next starts.
          <button class="link" disabled={busy} on:click={async () => { await stop(); await start(); advSaved = false; }}>Restart the node now</button></p>
      {/if}
    </div>
  {/if}
</details>

<div class="card">
  <h2>About</h2>
  <dl class="kv">
    <dt>App</dt><dd>Truthcoin App {info?.version}</dd>
    <dt>Data folder</dt><dd><code>{info?.dir}</code></dd>
  </dl>
  <AppUpdate />
  <p class="small muted" style="margin-top:8px">
    Open source (MIT). It runs L2L's Truthcoin node, which it downloads from L2L's GitHub releases and checks against
    the hash pinned in this app. Not made by L2L.
  </p>
</div>

<Obliterate {running} on:obliterated />
