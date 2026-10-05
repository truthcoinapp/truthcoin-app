<script lang="ts">
  // First run: BitWindow's eCash stack is found, the Truthcoin node installed and started, and the wallet made from new
  // words (shown once, three asked back) or brought back from old ones.
  import { createEventDispatcher, onDestroy } from "svelte";
  import { api, errText, type InstallProgress, type NodeStatus, type WalletStatus } from "../lib/api";
  import { num } from "../lib/format";
  import EnforcerAddress from "./EnforcerAddress.svelte";

  export let node: NodeStatus;
  export let wallet: WalletStatus | null;
  const dispatch = createEventDispatcher();

  let prog: InstallProgress | null = null;
  let err = "";
  let busy = false;
  let timer: ReturnType<typeof setInterval> | undefined;
  let mode: "choose" | "new" | "confirm" | "restore" = "choose";
  let words: string[] = [];
  let ask: number[] = [];
  let answers: string[] = ["", "", ""];
  let restoreText = "";
  let saw = false;
  // The enforcer box takes the saved address once: the status is read again every few seconds, and that mustn't
  // overwrite what is being typed, or close the box.
  let enforcerTyped = node.enforcer.address;
  let anotherOpen = node.enforcer.remote;

  onDestroy(() => timer && clearInterval(timer));

  $: step = !node.enforcer.reachable
    ? "stack"
    : !node.installed
      ? "install"
      : node.run.state !== "running"
        ? "start"
        : "wallet";
  $: if (step === "wallet" && wallet?.has_seed) dispatch("done");

  async function install() {
    err = "";
    try {
      await api.nodeInstall();
      timer = setInterval(async () => {
        prog = await api.installProgress();
        if (prog.error) {
          err = prog.error;
          clearInterval(timer);
        } else if (prog.finished) {
          clearInterval(timer);
          await start();
        }
      }, 400);
    } catch (e) {
      err = errText(e);
    }
  }

  async function start() {
    err = "";
    busy = true;
    try {
      await api.nodeStart();
    } catch (e) {
      err = errText(e);
    }
    busy = false;
    dispatch("changed");
  }

  async function newWords() {
    err = "";
    try {
      const w = await api.newWords();
      words = w.words;
      ask = w.ask;
      answers = ask.map(() => "");
      saw = false;
      mode = "new";
    } catch (e) {
      err = errText(e);
    }
  }

  async function confirm() {
    err = "";
    busy = true;
    try {
      await api.confirmWords(answers);
      words = [];
      dispatch("changed");
      dispatch("done");
    } catch (e) {
      err = errText(e);
    }
    busy = false;
  }

  async function restore() {
    err = "";
    busy = true;
    try {
      await api.restore(restoreText);
      restoreText = "";
      dispatch("changed");
      dispatch("done");
    } catch (e) {
      err = errText(e);
    }
    busy = false;
  }
</script>

<div class="card">
  <h2>Set up</h2>
  <p class="muted small">
    This app runs a Truthcoin node on this computer. It follows eCash through the enforcer that BitWindow runs, on this
    computer or another of yours.
  </p>
  <ol class="small muted" style="margin-left: 18px">
    <li style:color={step === "stack" ? "var(--text)" : ""}>BitWindow's eCash stack {step !== "stack" ? "✓" : ""}</li>
    <li style:color={step === "install" ? "var(--text)" : ""}>The Truthcoin node {step === "start" || step === "wallet" ? "✓" : ""}</li>
    <li style:color={step === "start" ? "var(--text)" : ""}>Start it {step === "wallet" ? "✓" : ""}</li>
    <li style:color={step === "wallet" ? "var(--text)" : ""}>Your wallet</li>
  </ol>
</div>

{#if err}
  <div class="notice error">{err}</div>
{/if}

{#if step === "stack"}
  <div class="card">
    <h2>Start eCash in BitWindow</h2>
    <p>
      The Truthcoin node follows eCash through the enforcer, which BitWindow starts with eCash. Nothing answers at
      <code>{node.enforcer.address}</code> yet.
    </p>
    <p class="muted small">Open BitWindow and wait until eCash is running, then check again.</p>
    <div class="actions">
      <button class="primary" on:click={() => dispatch("changed")}>Check again</button>
    </div>
  </div>
  <details class="card" bind:open={anotherOpen}>
    <summary>eCash on another computer?</summary>
    <p class="small muted" style="margin-top:8px">
      If BitWindow runs eCash on another computer of yours, type its enforcer's address. On that computer, BitWindow
      starts the enforcer on 127.0.0.1 only: it must listen on an address this computer can reach (the enforcer's
      <code>--serve-grpc-addr</code>), behind a firewall that lets in only this computer.
    </p>
    <EnforcerAddress bind:address={enforcerTyped} saveHere on:saved={() => dispatch("changed")} />
  </details>
{:else if step === "install"}
  <div class="card">
    <h2>Install the Truthcoin node</h2>
    <p>
      The app downloads L2L's Truthcoin node from GitHub (about 56 MB) and keeps it only if it is exactly the release
      this app was checked with.
    </p>
    {#if prog?.running}
      <p class="small muted">
        <span class="spin"></span> {num(prog.done_bytes / 1e6)} of {num(prog.total_bytes / 1e6)} MB
      </p>
      <div class="bar"><div style:width="{prog.total_bytes ? (100 * prog.done_bytes) / prog.total_bytes : 0}%"></div></div>
    {:else}
      <div class="actions"><button class="primary" on:click={install}>Install</button></div>
    {/if}
  </div>
{:else if step === "start"}
  <div class="card">
    <h2>Start the node</h2>
    {#if node.run.state === "starting" || busy}
      <p><span class="spin"></span> Starting the Truthcoin node…</p>
    {:else}
      {#if node.run.state === "failed"}
        <div class="notice error">{node.run.message.split("\n")[0]}</div>
        {#if node.run.message.includes("\n")}<details><summary>The node's log</summary><pre class="log">{node.run.message}</pre></details>{/if}
      {/if}
      <div class="actions"><button class="primary" on:click={start}>Start</button></div>
    {/if}
  </div>
{:else if mode === "choose"}
  <div class="card">
    <h2>Your wallet</h2>
    <p>Make a new wallet, or bring one back from its recovery words.</p>
    <p class="small muted">
      Anything running on this computer could spend from this wallet: keep only what you're trading in it, and close the
      app when you're done.
    </p>
    <div class="actions">
      <button class="primary" on:click={newWords}>New wallet</button>
      <button on:click={() => (mode = "restore")}>I have recovery words</button>
    </div>
  </div>
{:else if mode === "new"}
  <div class="card">
    <h2>Write these words down</h2>
    <p>
      They are the only way to get this wallet back. <strong>The app shows them once and keeps no copy.</strong> Write
      them on paper, in order, and keep it safe. Anyone with them can spend.
    </p>
    <div class="words">
      {#each words as w, i}<div><span>{i + 1}</span>{w}</div>{/each}
    </div>
    <label style="display:flex;gap:8px;align-items:center;margin-top:10px">
      <input type="checkbox" bind:checked={saw} style="width:auto" /> I wrote them down
    </label>
    <div class="actions">
      <button class="primary" disabled={!saw} on:click={() => (mode = "confirm")}>Next</button>
      <button on:click={() => (mode = "choose")}>Back</button>
    </div>
  </div>
{:else if mode === "confirm"}
  <div class="card">
    <h2>Check your words</h2>
    <p class="muted">From what you wrote down:</p>
    {#each ask as n, i}
      <div class="field">
        <label for="w{i}">Word {n}</label>
        <input id="w{i}" bind:value={answers[i]} autocomplete="off" autocapitalize="off" spellcheck="false" />
      </div>
    {/each}
    <div class="actions">
      <button class="primary" disabled={busy || answers.some((a) => !a.trim())} on:click={confirm}>Make the wallet</button>
      <button on:click={() => { err = ""; mode = "new"; }}>Show the words again</button>
    </div>
  </div>
{:else}
  <div class="card">
    <h2>Bring a wallet back</h2>
    <p class="muted small">Type or paste its 12 (or 24) recovery words, in order, separated by spaces.</p>
    <textarea rows="4" bind:value={restoreText} autocomplete="off" autocapitalize="off" spellcheck="false"></textarea>
    <p class="muted small">The node then looks through the chain for the wallet's coins; that can take a while.</p>
    <div class="actions">
      <button class="primary" disabled={busy || !restoreText.trim()} on:click={restore}>
        {#if busy}<span class="spin"></span>{/if} Restore
      </button>
      <button on:click={() => (mode = "choose")}>Back</button>
    </div>
  </div>
{/if}
