<script lang="ts">
  // Not paired, and pairing: scan (or paste) the computer's QR code, name this phone, compare the codes both screens
  // show while the computer asks "Allow this phone?", then confirm here that it did. Nothing is kept as a pairing
  // until then. The attempt's keys are kept only while pairing is under way, and dropped on every way out but
  // unloading the page; after a reload the page asks before carrying on.
  import { createEventDispatcher, onDestroy, onMount } from 'svelte';
  import { defaultDeviceName, keyFingerprint } from '../lib/format';
  import { pairValueFrom } from '../lib/fragment';
  import { ios, standalone } from '../lib/homescreen';
  import {
    attemptFor,
    pairPhone,
    parsePairValue,
    PairExpiredError,
    PairNoAnswerError,
    PairRefusedError,
    type PairAttempt,
    type Pairing,
    type PairLink,
  } from '../lib/pairing';
  import { storage } from '../lib/session';
  import { dropAttempt, loadAttempt, saveAttempt } from '../lib/store';
  import { cleanName } from '../lib/text';
  import HomeScreenHint from './HomeScreenHint.svelte';
  import Mark from './Mark.svelte';
  import Scanner from './Scanner.svelte';

  export let pairValue: string | null;
  export let alreadyPaired = false;
  /** The computer this phone is paired with now (its D), to tell a different one apart. */
  export let currentD: string | null = null;
  /** A pairing that was under way before the page reloaded: offered, never carried on without asking. */
  export let resume: PairAttempt | null = null;

  const dispatch = createEventDispatcher<{ paired: Pairing; cancel: null }>();

  type State =
    | 'start'
    | 'scanning'
    | 'reading'
    | 'resume'
    | 'form'
    | 'waiting'
    | 'confirm'
    | 'declined'
    | 'refused'
    | 'expired'
    | 'noanswer'
    | 'error';
  let state: State = resume ? 'resume' : pairValue ? 'reading' : 'start';
  let link: PairLink | null = resume?.link ?? null;
  let kept: PairAttempt | null = resume;
  const inApp = standalone();
  const iphone = ios();
  // On an iPhone, the Home Screen app is where pairing belongs (its own storage); the browser can still pair if asked.
  let pairHere = !iphone || inApp;
  let name = resume?.name ?? defaultDeviceName();
  let message = '';
  let code = resume?.code ?? '';
  let slow = false;
  let refusal = '';
  let pasted = '';
  let pasteError = '';
  let pasteOpen = false;
  let scanError = '';
  let scanReload = false;
  let left = 0;
  let gone = false;
  let clock: ReturnType<typeof setInterval> | undefined;
  /** The desktop said yes: kept only once the person confirms. */
  let yes: Pairing | null = null;

  const nowS = () => Math.floor(Date.now() / 1000);
  $: fingerprint = link ? keyFingerprint(link.d) : '';
  $: otherComputer = !!(alreadyPaired && currentD && link && currentD !== link.d);

  async function readLink(value: string) {
    state = 'reading';
    try {
      const l = await parsePairValue(value);
      link = l;
      // This code's attempt from before a reload: ask before carrying on.
      const k = await keptFor(l);
      if (k) {
        kept = k;
        name = k.name;
        code = k.code ?? '';
        state = 'resume';
        return;
      }
      if (nowS() >= l.x) {
        state = 'expired';
        return;
      }
      tickClock();
      state = 'form';
    } catch (e) {
      message = (e as Error).message;
      state = 'error';
    }
  }

  async function keptFor(l: PairLink): Promise<PairAttempt | null> {
    try {
      return attemptFor(await loadAttempt(storage()), l, nowS());
    } catch {
      return null;
    }
  }

  function drop() {
    kept = null;
    void dropAttempt(storage()).catch(() => undefined);
  }

  function tickClock() {
    clearInterval(clock);
    const upd = () => {
      left = link ? Math.max(0, link.x - nowS()) : 0;
      if (link && left === 0 && state === 'form') state = 'expired';
    };
    upd();
    clock = setInterval(upd, 1000);
  }

  onMount(() => {
    if (!resume && pairValue) void readLink(pairValue);
  });
  // Leaving the pairing screen in any way ends the attempt (unloading the page doesn't run this).
  onDestroy(() => {
    gone = true;
    clearInterval(clock);
    drop();
  });

  function scanned(e: CustomEvent<string>) {
    void readLink(e.detail);
  }

  function scanFailed(e: CustomEvent<{ text: string; reload: boolean }>) {
    scanError = e.detail.text;
    scanReload = e.detail.reload;
    pasteOpen = !scanReload;
    state = 'start';
  }

  /** Back to the start: whatever was under way is over. */
  function again() {
    drop();
    link = null;
    message = '';
    code = '';
    slow = false;
    yes = null;
    state = 'start';
  }

  function stopResume() {
    again();
    if (alreadyPaired) dispatch('cancel', null);
  }

  function pastePair() {
    pasteError = '';
    const v = pairValueFrom(pasted);
    if (!v) {
      pasteError = "That isn't a pairing link. Copy it again from the Truthcoin App on your computer.";
      return;
    }
    pasted = '';
    void readLink(v);
  }

  async function pasteFromClipboard() {
    pasteError = '';
    try {
      pasted = await navigator.clipboard.readText();
    } catch {
      pasteError = "This browser didn't hand over the clipboard: press and hold the box, then Paste.";
      return;
    }
    pastePair();
  }

  async function pair() {
    if (!link) return;
    const l = link;
    state = 'waiting'; // the request goes out at once; the code shows once the computer's nonce comes
    slow = false;
    gone = false;
    try {
      const p = await pairPhone(l, cleanName(name), {
        attempt: kept,
        keep: (a) => {
          kept = a;
          return saveAttempt(storage(), a);
        },
        onCode: (c) => (code = c),
        onSlow: () => (slow = true),
        cancelled: () => gone || state !== 'waiting',
      });
      drop();
      yes = p;
      state = 'confirm';
    } catch (e) {
      if (gone || (e as Error).message === 'cancelled') return;
      drop();
      if (e instanceof PairRefusedError) {
        refusal = e.message;
        state = 'refused';
      } else if (e instanceof PairExpiredError) state = 'expired';
      else if (e instanceof PairNoAnswerError) state = 'noanswer';
      else {
        message = (e as Error).message || 'Pairing failed.';
        state = 'error';
      }
    }
  }

  function confirmYes() {
    if (yes) dispatch('paired', yes);
  }

  function confirmNo() {
    yes = null; // not kept; the pairing's keys go with it, and nothing more is sent
    state = 'declined';
  }
</script>

<section class="stack" data-testid="pair">
  {#if state === 'start' || state === 'reading'}
    <div class="card stack intro">
      <div class="hero"><Mark size={56} /></div>
      <h2>{inApp && iphone ? 'Pair this Home Screen app' : 'Pair with your computer'}</h2>
      <p>
        A remote for the Truthcoin App on your computer. Open the app there, go to <strong>Settings › Phone</strong>,
        and scan its code.
      </p>
      <p class="small warn">Only scan the code your own computer shows. Never use a pairing link someone sent you.</p>
      {#if alreadyPaired}
        <p class="small warn">This phone is paired with a computer already. Pairing again replaces it.</p>
      {/if}
      {#if scanError}<p class="error small" data-testid="scan-error">{scanError}</p>{/if}
      {#if state === 'reading'}
        <p class="muted"><span class="spinner"></span> Reading the code…</p>
      {:else if scanReload}
        <button class="primary full" on:click={() => location.reload()}>Reload</button>
      {:else}
        <button class="primary full" on:click={() => ((scanError = ''), (state = 'scanning'))}>Scan the code</button>
      {/if}
      <details bind:open={pasteOpen}>
        <summary>Can't scan? Paste the pairing link</summary>
        <form class="stack paste" on:submit|preventDefault={pastePair}>
          <p class="muted small">
            On your own computer, click <strong>Copy link</strong> next to the QR code, and paste it here. Never paste a
            link someone sent you.
          </p>
          <label for="pairlink">Pairing link</label>
          <input
            id="pairlink"
            bind:value={pasted}
            placeholder="https://…/#pair=…"
            autocomplete="off"
            autocapitalize="off"
            spellcheck="false"
            enterkeyhint="go"
          />
          {#if pasteError}<p class="error small">{pasteError}</p>{/if}
          <div class="buttons">
            <button type="button" on:click={pasteFromClipboard}>Paste</button>
            <button class="primary" type="submit" disabled={!pasted.trim()}>Use this link</button>
          </div>
        </form>
      </details>
      <p class="muted small">
        Your phone gets its own key, which you can remove on your computer at any time. Messages go through public Nostr
        relays, sealed so only your computer and this phone can read them.
      </p>
    </div>
    <HomeScreenHint />
    {#if alreadyPaired}
      <button class="full" on:click={() => dispatch('cancel', null)}>Keep the current pairing</button>
    {/if}
  {:else if state === 'scanning'}
    <div class="card stack">
      <h2>Scan the code</h2>
      <Scanner on:value={scanned} on:cancel={again} on:failed={scanFailed} />
    </div>
  {:else if state === 'resume'}
    <div class="card stack" data-testid="pair-resume">
      <h2>Carry on pairing?</h2>
      <p>Carry on pairing with the computer whose key is <strong class="mono" data-testid="pair-fingerprint">{fingerprint}</strong>?</p>
      <p class="small muted">This page was closed or reloaded while it was pairing as <strong>{name}</strong>.</p>
      {#if otherComputer && currentD}
        <p class="small warn">
          This is a different computer from the one this phone is paired with (key {keyFingerprint(currentD)}, now
          {fingerprint}). Pairing replaces it.
        </p>
      {:else if alreadyPaired}
        <p class="small warn">Pairing again replaces the computer this phone is paired with now.</p>
      {/if}
      <div class="buttons">
        <button on:click={stopResume}>Stop</button>
        <button class="primary" on:click={pair}>Carry on</button>
      </div>
    </div>
  {:else if state === 'form' && !pairHere}
    <div class="card stack" data-testid="homescreen-first">
      <h2>Add Truthcoin to your Home Screen first</h2>
      <p>
        On an iPhone, pair from the Home Screen app: it keeps its own storage apart from Safari, and Safari may delete
        this page's key after a week without a visit.
      </p>
      <ol class="steps">
        <li>Tap <strong>Share</strong> (under <strong>•••</strong> if you don't see it), then <strong>Add to Home Screen</strong>.</li>
        <li>Open <strong>Truthcoin</strong> from your Home Screen.</li>
        <li>Tap <strong>Scan the code</strong> and scan the same QR code again (it works for {Math.ceil(left / 60)} more min).</li>
      </ol>
    </div>
    <button class="full" on:click={() => (pairHere = true)}>Pair in this browser instead</button>
    <button class="full" on:click={again}>Back</button>
  {:else if state === 'form'}
    <form class="card stack" on:submit|preventDefault={pair}>
      <h2>Name this phone</h2>
      <p class="small">
        The computer's key: <strong class="mono" data-testid="pair-fingerprint">{fingerprint}</strong>. Your computer shows
        the same under Settings › Phone.
      </p>
      <p class="small warn">Only scan the code your own computer shows. Never use a pairing link someone sent you.</p>
      {#if otherComputer && currentD}
        <p class="small warn" data-testid="pair-other-computer">
          This is a different computer from the one this phone is paired with (key {keyFingerprint(currentD)}, now
          {fingerprint}). Pairing replaces it.
        </p>
      {:else if alreadyPaired}
        <p class="small warn">Pairing again replaces the computer this phone is paired with now.</p>
      {/if}
      <div>
        <label for="devname">Name</label>
        <input id="devname" bind:value={name} maxlength="40" autocomplete="off" autocapitalize="words" enterkeyhint="go" />
        <p class="muted small hint">Your computer shows this name when it asks you to allow the phone.</p>
      </div>
      <button class="primary full" type="submit">Pair</button>
      <button class="full" type="button" on:click={again}>Back</button>
      <p class="muted small">The code works for {Math.floor(left / 60)}:{String(left % 60).padStart(2, '0')} more.</p>
    </form>
  {:else if state === 'waiting'}
    <div class="card stack center" data-testid="pair-waiting">
      {#if code}
        <p>Check that your computer shows the same code:</p>
        <p class="code num" data-testid="pair-code">{code}</p>
        <p>
          The Truthcoin App on your computer asks <strong>“Allow this phone?”</strong> for <strong>{cleanName(name)}</strong>.
          Allow it only if the codes match.
        </p>
        <p class="muted small">A different code means someone else is pairing with your QR code: refuse that one.</p>
      {:else}
        <p data-testid="pair-no-code-yet"><span class="spinner"></span> Waiting for your computer…</p>
        <p class="muted small">The code to compare appears here once your computer has the request.</p>
      {/if}
      <p class="small">The computer's key: <strong class="mono">{fingerprint}</strong></p>
      {#if slow}
        <p class="warn" data-testid="pair-slow">No answer yet. Is the app open on your computer?</p>
      {:else if code}
        <p class="muted small"><span class="spinner"></span> Waiting for your computer. Keep this page open.</p>
      {/if}
      <button class="full" on:click={again}>Cancel</button>
    </div>
  {:else if state === 'confirm'}
    <div class="card stack" data-testid="pair-confirm">
      <h2>Did you allow it?</h2>
      {#if code}
        <p>Did your computer show <strong class="num">{code}</strong>, and did you allow it there?</p>
      {:else}
        <p>Did your computer ask you to allow <strong>{cleanName(name)}</strong>, and did you allow it there?</p>
      {/if}
      <p class="small muted">
        Only your own computer can ask. If it didn't, the code came from someone else: answer No, and nothing is kept.
      </p>
      <div class="buttons">
        <button on:click={confirmNo}>No</button>
        <button class="primary" on:click={confirmYes}>Yes, I allowed it</button>
      </div>
    </div>
  {:else if state === 'declined'}
    <div class="card stack bad" data-testid="pair-declined">
      <h2>Not paired</h2>
      <p>Nothing was kept on this phone. If your own computer lists it under Settings › Phone, remove it there.</p>
      <button class="full" on:click={again}>Back</button>
    </div>
  {:else if state === 'refused'}
    <div class="card stack bad">
      <h2>Not paired</h2>
      <p>Your computer said no.</p>
      {#if refusal && refusal !== 'not allowed'}<p class="small" data-testid="pair-refusal">{refusal}</p>{/if}
      <p class="muted small">To try again, show a new code on your computer (Settings › Phone) and scan it.</p>
      <button class="full" on:click={again}>Back</button>
    </div>
  {:else if state === 'expired'}
    <div class="card stack bad">
      <h2>This code has expired</h2>
      <p>Pairing codes last 5 minutes. Show a new one on your computer (Settings › Phone) and scan it.</p>
      <button class="full" on:click={again}>Back</button>
    </div>
  {:else if state === 'noanswer'}
    <div class="card stack bad">
      <h2>No answer from your computer</h2>
      <p>Is the Truthcoin App open there? Show a new code (Settings › Phone) and scan it again.</p>
      <button class="full" on:click={again}>Back</button>
    </div>
  {:else}
    <div class="card stack bad">
      <h2>Could not pair</h2>
      <p class="error">{message}</p>
      <button class="full" on:click={again}>Back</button>
    </div>
  {/if}
</section>

<style>
  .intro .hero {
    display: flex;
    justify-content: center;
    padding-top: 6px;
  }
  .center {
    text-align: center;
  }
  .hint {
    margin-top: 6px;
  }
  .code {
    font-size: 44px;
    font-weight: 700;
    letter-spacing: 0.06em;
    color: var(--accent);
    white-space: nowrap;
    line-height: 1.1;
  }
  details summary {
    cursor: pointer;
    color: var(--accent);
    font-size: 15px;
    min-height: 32px;
  }
  details[open] summary {
    margin-bottom: 10px;
  }
  .steps {
    padding-left: 20px;
    display: grid;
    gap: 8px;
  }
</style>
