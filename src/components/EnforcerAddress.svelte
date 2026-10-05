<script lang="ts">
  // The enforcer's address: type it, test it (it answers and, in a release, follows eCash beta), and on the Setup
  // screen save it. Used in Setup, when nothing answers on this computer, and in Settings › Advanced (which saves it
  // with the rest).
  import { createEventDispatcher } from "svelte";
  import { api, errText, type EnforcerCheck } from "../lib/api";

  export let address: string;
  /** Setup: a "Save and check again" button that saves this address alone. */
  export let saveHere = false;
  const dispatch = createEventDispatcher<{ saved: string }>();

  let testing = false;
  let saving = false;
  let check: EnforcerCheck | null = null;
  let checkedFor = "";
  let err = "";
  $: if (check && address !== checkedFor) check = null;
  // On this computer: localhost, 127.x.y.z or ::1. Anything else goes over a network (the app checks again).
  $: host = address.trim().replace(/^http:\/\//i, "").replace(/:\d*\/?$/, "").replace(/^\[|\]$/g, "");
  $: remote = !!host && !(host === "localhost" || /^127\.\d+\.\d+\.\d+$/.test(host) || host === "::1");

  async function test() {
    testing = true;
    err = "";
    checkedFor = address;
    try {
      check = await api.enforcerTest(address);
    } catch (e) {
      err = errText(e);
    }
    testing = false;
  }

  async function save() {
    saving = true;
    err = "";
    try {
      address = await api.enforcerSet(address);
      dispatch("saved", address);
    } catch (e) {
      err = errText(e);
    }
    saving = false;
  }
</script>

<div class="field">
  <label for="enforcer">Enforcer (gRPC)</label>
  <input id="enforcer" bind:value={address} placeholder="127.0.0.1:50051" spellcheck="false" autocomplete="off" />
</div>
<div class="actions">
  <button disabled={testing || !address.trim()} on:click={test}>{testing ? "Testing…" : "Test"}</button>
  {#if saveHere}
    <button class="primary" disabled={saving || !address.trim()} on:click={save}>Save and check again</button>
  {/if}
</div>
{#if testing}
  <p class="small muted"><span class="spin"></span> Asking {address.trim()}…</p>
{:else if check}
  <p class="small" style:color={check.ok ? "var(--accent)" : "var(--error)"}>{check.ok ? "✓ " : ""}{check.detail}</p>
{/if}
{#if err}<p class="small" style:color="var(--error)">{err}</p>{/if}
{#if remote}
  <div class="notice warn small">
    That's another computer. Anyone who can reach that enforcer can spend its eCash wallet (it has no login), and the
    connection isn't encrypted, so someone on the network between them could show the Truthcoin node a false eCash.
    Safer: an SSH tunnel to it, then <code>127.0.0.1:50051</code> here (README, "eCash on another computer").
  </div>
{/if}
