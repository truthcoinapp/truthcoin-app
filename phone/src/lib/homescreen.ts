import { writable } from 'svelte/store';

// The page on a Home Screen. iOS gives a Home Screen icon its own storage, apart from Safari's, so an iPhone installs
// first and then pairs from the icon (it scans the computer's QR code there).

/** Opened from a Home Screen icon (iOS says so in navigator.standalone; others in the display mode). */
export function standalone(): boolean {
  if (typeof navigator === 'undefined') return false;
  return (navigator as { standalone?: boolean }).standalone === true || matchMedia('(display-mode: standalone)').matches;
}

/** An iPhone or iPad (iPadOS calls itself a Mac, but has touch). */
export function ios(): boolean {
  if (typeof navigator === 'undefined') return false;
  return /iPhone|iPad|iPod/.test(navigator.userAgent) || (/Macintosh/.test(navigator.userAgent) && navigator.maxTouchPoints > 1);
}

/** Chrome's install prompt (Android; it needs the page's manifest), kept for the Install button. */
export interface InstallPrompt {
  prompt: () => Promise<void>;
}
export const installPrompt = writable<InstallPrompt | null>(null);
if (typeof window !== 'undefined') {
  window.addEventListener('beforeinstallprompt', (e) => {
    e.preventDefault();
    installPrompt.set(e as unknown as InstallPrompt);
  });
  window.addEventListener('appinstalled', () => installPrompt.set(null));
}
