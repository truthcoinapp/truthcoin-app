<script lang="ts">
  export let text: string;
  export let label = 'Copy';
  let done = false;
  let timer: ReturnType<typeof setTimeout>;

  async function copy() {
    let ok = false;
    try {
      await navigator.clipboard.writeText(text);
      ok = true;
    } catch {
      // Older WebViews: copy through a temporary selection.
      const ta = document.createElement('textarea');
      ta.value = text;
      ta.setAttribute('readonly', '');
      ta.className = 'copy-offscreen';
      document.body.appendChild(ta);
      ta.select();
      try {
        ok = document.execCommand('copy');
      } catch {
        ok = false;
      }
      ta.remove();
    }
    if (ok) {
      done = true;
      clearTimeout(timer);
      timer = setTimeout(() => (done = false), 1600);
    }
  }
</script>

<button class="copy" class:done on:click|stopPropagation={copy} aria-live="polite">{done ? 'Copied' : label}</button>

<style>
  .copy {
    min-height: 44px;
    min-width: 88px;
    font-size: 15px;
  }
  .done {
    color: var(--accent);
    border-color: var(--accent);
  }
  :global(.copy-offscreen) {
    position: fixed;
    top: -1000px;
    opacity: 0;
  }
</style>
