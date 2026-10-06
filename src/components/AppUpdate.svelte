<script lang="ts">
  // The app's own updates (v0.1.3, lib/appUpdate.ts). As a notice at the top of the app when a signed release is out
  // (`notice`), and in Settings › About.
  import { invoke } from "@tauri-apps/api/core";
  import { openUrl } from "../lib/api";
  import {
    appUpdate,
    appUpdateLater,
    appUpdateProgress,
    checkAppUpdate,
    megabytes,
    stageText,
    startAppUpdate,
  } from "../lib/appUpdate";

  export let notice = false;
  /** Settings: "Check for updates by itself", as saved (null: not shown). */
  export let auto: boolean | null = null;
  let autoErr = "";
  async function setAuto(on: boolean) {
    autoErr = "";
    try {
      await invoke("app_update_auto", { on });
      auto = on;
    } catch (e) {
      autoErr = String(e);
    }
  }

  let checking = false;
  let checked = false;
  let startError = "";

  $: c = $appUpdate;
  $: p = $appUpdateProgress;
  $: updating = !!p?.running;
  $: failed = p && !p.running ? p.error : null;
  $: show = notice ? !!c?.available && (!$appUpdateLater || updating) : true;

  async function check() {
    checking = true;
    await checkAppUpdate(true);
    checking = false;
    checked = true;
  }

  async function update() {
    startError = "";
    try {
      await startAppUpdate();
    } catch (e) {
      startError = String(e);
    }
  }
</script>

{#if show}
  <div class={notice ? "notice ok" : ""} data-testid={notice ? "update-notice" : "update-settings"}>
    {#if c?.available}
      <p><strong>Truthcoin App {c.latest} is out.</strong>{notice ? ` You have ${c.current}.` : ""}</p>
      {#if updating && p}
        <p class="small">
          {stageText(p)}{p.stage === "download" ? ` ${megabytes(p.bytes, p.total)}` : ""}
          {#if p.note}<br />Waiting: {p.note}{/if}
        </p>
        {#if p.stage === "download" && p.total}
          <div class="bar"><div class="fill" style="width:{(p.bytes / p.total) * 100}%"></div></div>
        {/if}
      {:else if c.how === "self"}
        <p class="small muted">The app closes and opens again on the new version. The Truthcoin node stops and starts with it.</p>
        <div class="actions">
          <button class="primary" on:click={update}>Update and restart</button>
          {#if c.page}<button on:click={() => openUrl(c?.page ?? "")}>What's new</button>{/if}
          {#if notice}<button on:click={() => appUpdateLater.set(true)}>Later</button>{/if}
        </div>
      {:else if c.how === "deb"}
        <p class="small muted">Download the new .deb from the release page and install it (<code>sudo apt install ./truthcoin-app_{c.latest}_amd64.deb</code>).</p>
        <div class="actions">
          {#if c.page}<button class="primary" on:click={() => openUrl(c?.page ?? "")}>Release page</button>{/if}
          {#if notice}<button on:click={() => appUpdateLater.set(true)}>Later</button>{/if}
        </div>
      {:else}
        {#if c.why}<p class="small muted">{c.why}</p>{/if}
        <div class="actions">
          {#if c.page}<button class="primary" on:click={() => openUrl(c?.page ?? "")}>Release page</button>{/if}
          {#if notice}<button on:click={() => appUpdateLater.set(true)}>Later</button>{/if}
        </div>
      {/if}
      {#if startError}<p class="small error">{startError}</p>{/if}
      {#if failed}<p class="small error">The update didn't finish: {failed}</p>{/if}
    {:else if !notice}
      {#if c?.error && checked}
        <p class="small error">{c.error}</p>
      {:else if checked && c}
        <p class="small muted">This is the newest version.</p>
      {/if}
      <div class="actions">
        <button disabled={checking} on:click={check}>{#if checking}<span class="spin"></span>{/if} Check for a newer version</button>
      </div>
    {/if}
    {#if !notice}
      {#if auto !== null}
        <label class="check small">
          <input type="checkbox" checked={auto} on:change={(e) => setAuto(e.currentTarget.checked)} data-testid="update-auto" />
          <span>Check for a newer version by itself: shortly after the app starts, and twice a day. Each check asks GitHub
            for the latest release's checksums. Off, it checks only when you press the button.</span>
        </label>
        {#if autoErr}<p class="small error">{autoErr}</p>{/if}
      {/if}
      <p class="small muted" style="margin-top:8px">
        The app installs a new version only when the release carries the signature of this app's release key.
      </p>
    {/if}
  </div>
{/if}

<style>
  .bar {
    height: 6px;
    border-radius: 3px;
    background: var(--inset);
    overflow: hidden;
    margin: 6px 0 10px;
  }
  .fill {
    height: 100%;
    background: var(--accent);
  }
  p {
    margin: 0 0 8px;
  }
  .check {
    display: flex;
    gap: 8px;
    align-items: flex-start;
    margin-top: 10px;
  }
  .check input {
    margin-top: 3px;
  }
</style>
