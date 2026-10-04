<script lang="ts">
  // The camera view for "Scan the code" (lib/scan.ts). The camera turns off as soon as a pairing code is read, on
  // Cancel, when this view closes, and when the page goes to the background.
  import { createEventDispatcher, onDestroy, onMount } from 'svelte';
  import { ios } from '../lib/homescreen';
  import { cameraError, scannedPair, ScanCancelled, startScan } from '../lib/scan';

  const dispatch = createEventDispatcher<{ value: string; cancel: null; failed: { text: string; reload: boolean } }>();
  let video: HTMLVideoElement;
  let stop: (() => void) | null = null;
  let gone = false;
  let starting = true;
  let note = '';

  onMount(async () => {
    document.addEventListener('visibilitychange', away);
    try {
      stop = await startScan(
        video,
        (text) => {
          if (gone) return;
          const r = scannedPair(text);
          if ('wrong' in r) {
            note = r.wrong;
            return;
          }
          end();
          dispatch('value', r.value);
        },
        () => gone,
      );
    } catch (e) {
      if (!gone && !(e instanceof ScanCancelled)) dispatch('failed', cameraError(e, ios()));
    }
    starting = false;
  });

  function end() {
    gone = true;
    stop?.();
    stop = null;
  }

  // Leaving the page (another app, the lock screen) ends the scan; Scan starts it again.
  function away() {
    if (!document.hidden || gone) return;
    end();
    dispatch('cancel', null);
  }

  onDestroy(() => {
    document.removeEventListener('visibilitychange', away);
    end();
  });
</script>

<div class="stack" data-testid="scanner">
  <div class="view">
    <!-- svelte-ignore a11y-media-has-caption -->
    <video bind:this={video} playsinline muted></video>
    <div class="frame" aria-hidden="true"></div>
  </div>
  <p class="small center">{starting ? 'Opening the camera…' : 'Point the camera at the QR code on your computer.'}</p>
  {#if note}<p class="error small">{note}</p>{/if}
  <button
    class="full"
    on:click={() => {
      end();
      dispatch('cancel', null);
    }}>Cancel</button
  >
</div>

<style>
  .view {
    position: relative;
    aspect-ratio: 1;
    border-radius: 14px;
    overflow: hidden;
    background: #000;
  }
  video {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .frame {
    position: absolute;
    inset: 14%;
    border: 3px solid rgba(255, 255, 255, 0.85);
    border-radius: 12px;
  }
  .center {
    text-align: center;
  }
</style>
