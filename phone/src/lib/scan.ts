// "Scan the code": the page reads the computer's pairing QR code with the camera. A Home Screen icon has no other
// easy way in, since iOS keeps its storage apart from Safari's. The pictures stay on the phone: each frame is drawn
// to a canvas here and decoded by jsQR, which loads (as its own file, from this page's server) only when Scan is
// tapped. Safari has no BarcodeDetector.
import { pairValueFrom } from './fragment';

/** What the scanned text means: a pairing value to read, or why it isn't one. */
export type Scanned = { value: string } | { wrong: string };

const NOT_OURS = "That QR code isn't a pairing code. Scan the one the Truthcoin App shows in its Phone tab.";

export function scannedPair(text: string): Scanned {
  const value = pairValueFrom(text);
  return value ? { value } : { wrong: NOT_OURS };
}

/** Why the camera didn't open, and whether reloading the page may help (the reader's file didn't load). */
export function cameraError(e: unknown, iphone: boolean): { text: string; reload: boolean } {
  const name = (e as { name?: string } | null)?.name ?? '';
  if (loadFailed(e)) return { text: 'The scanner did not load. Check your connection, then tap Reload.', reload: true };
  if (name === 'NotAllowedError' || name === 'SecurityError')
    return {
      text: iphone
        ? 'This page may not use the camera. Tap Scan again and allow the camera when your iPhone asks. If it doesn’t ask, allow it in Settings › Safari › Camera. Or paste the pairing link instead.'
        : 'This page may not use the camera. Allow it in your browser’s settings for this site, then tap Scan again. Or paste the pairing link instead.',
      reload: false,
    };
  if (name === 'NotFoundError' || name === 'OverconstrainedError')
    return { text: 'This phone has no camera the page can use. Paste the pairing link instead.', reload: false };
  if (name === 'NotReadableError') return { text: 'The camera is busy. Close any app using it, then tap Scan again.', reload: false };
  return { text: 'The camera did not open. Paste the pairing link instead.', reload: false };
}

function loadFailed(e: unknown): boolean {
  return /dynamically imported module|Importing a module script failed|error loading dynamically imported module/i.test(
    String((e as Error | null)?.message ?? ''),
  );
}

/** The size to decode a `w`×`h` frame at: no side over `max`, aspect kept. */
export function decodeSize(w: number, h: number, max = 720): { w: number; h: number } {
  const k = Math.min(1, max / Math.max(w, h, 1));
  return { w: Math.max(1, Math.round(w * k)), h: Math.max(1, Math.round(h * k)) };
}

type JsQR = (data: Uint8ClampedArray, w: number, h: number, opts?: { inversionAttempts?: string }) => { data: string } | null;

/** Thrown when `cancelled()` turned true while the camera was starting; the camera is off again. */
export class ScanCancelled extends Error {}

/**
 * Open the back camera into `video` and call `onText` with each QR code's text until stopped. Returns the stop
 * function, which turns the camera off. The reader loads first, so the phone asks for the camera only once it can
 * be used; `cancelled()` is checked after each wait, so a scan left while the camera was starting turns it off.
 * Rejects when the camera or the reader can't start (see `cameraError`).
 */
export async function startScan(video: HTMLVideoElement, onText: (text: string) => void, cancelled: () => boolean): Promise<() => void> {
  if (!navigator.mediaDevices?.getUserMedia) throw Object.assign(new Error('no camera API'), { name: 'NotFoundError' });
  const decoder = (await import('jsqr')).default as unknown as JsQR;
  if (cancelled()) throw new ScanCancelled();
  const stream = await navigator.mediaDevices.getUserMedia({
    audio: false,
    video: { facingMode: { ideal: 'environment' }, width: { ideal: 1280 }, height: { ideal: 720 } },
  });
  let stopped = false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const stop = () => {
    stopped = true;
    clearTimeout(timer);
    for (const t of stream.getTracks()) t.stop();
    video.srcObject = null;
  };
  try {
    if (cancelled()) throw new ScanCancelled();
    video.srcObject = stream;
    video.setAttribute('playsinline', ''); // iOS: play in the page, not full screen
    video.muted = true;
    await video.play();
    if (cancelled()) throw new ScanCancelled();
  } catch (e) {
    stop();
    throw e;
  }
  const canvas = document.createElement('canvas');
  const ctx = canvas.getContext('2d', { willReadFrequently: true });
  const tick = () => {
    if (stopped) return;
    try {
      if (ctx && video.readyState >= 2 && video.videoWidth) {
        const { w, h } = decodeSize(video.videoWidth, video.videoHeight);
        canvas.width = w;
        canvas.height = h;
        ctx.drawImage(video, 0, 0, w, h);
        const found = decoder(ctx.getImageData(0, 0, w, h).data, w, h, { inversionAttempts: 'attemptBoth' });
        if (found?.data) onText(found.data);
      }
    } catch {
      // A frame that can't be read: try the next one.
    }
    if (!stopped) timer = setTimeout(tick, 150);
  };
  timer = setTimeout(tick, 0); // after the caller holds `stop`
  return stop;
}
